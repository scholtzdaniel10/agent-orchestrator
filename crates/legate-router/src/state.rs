//! In-memory plan usage. A pick slides a running weight sum; it does not scan run history
//! and it does not allocate.
//!
//! Hydrate once from [`Store::runs_since_all`]. After that, [`RouterState::record_run`],
//! [`RouterState::set_windows`], and [`RouterState::set_resting`] keep the cache current.
//! `now` may move forward or backward inside the runs still held in the deque.

use std::collections::VecDeque;

use legate_core::{JobType, PROVIDERS, ProviderId, RouterRules};
use legate_store::{RunRow, Store, StoredWindow};

use crate::ids::{PROVIDER_COUNT, ProviderSet, RunningCounts, plan_index, provider_index};
use crate::usage::{
    CompiledRules, DASHBOARD_WINDOW, MiniWindow, PaceSnap, Pick, PlanUsage, RouterError, Score,
    Summary, SummaryAcc, WindowState, dashboard_cycle, dashboard_mini, decide, run_weight,
    scores_from, usage_from_summary, weight_used,
};

#[derive(Clone, Copy)]
struct RunPoint {
    started_at: i64,
    id: i64,
    weight: f64,
}

struct PlanMemory {
    resting_until: Option<i64>,
    stored: Vec<StoredWindow>,
    runs: VecDeque<RunPoint>,
    /// How many runs at the front are older than `allowance_since`.
    below_allowance: usize,
    allowance_sum: f64,
    allowance_since: i64,
    dashboard_mark: Option<i64>,
    dashboard_sum: f64,
}

impl PlanMemory {
    fn new() -> Self {
        Self {
            resting_until: None,
            stored: Vec::new(),
            runs: VecDeque::new(),
            below_allowance: 0,
            allowance_sum: 0.0,
            allowance_since: i64::MIN,
            dashboard_mark: None,
            dashboard_sum: 0.0,
        }
    }
}

/// Per-plan usage kept in memory.
///
/// A provider with no allowance row, including `local` in the bundled rules, is unlimited:
/// with no reported windows its used fraction is 0 and its headroom is 1 unless it is resting.
/// A reported window still counts. A dashboard window whose allowance units are missing adds
/// no extra weight.
pub struct RouterState {
    rules: CompiledRules,
    plans: [PlanMemory; PROVIDER_COUNT],
    cached_now: Option<i64>,
    snaps_valid: bool,
    snaps: [PaceSnap; PROVIDER_COUNT],
}

impl RouterState {
    /// Load rest, windows, and every run inside the lookback. One `runs_since_all` query.
    pub fn hydrate(store: &Store, rules: &RouterRules, now: i64) -> Result<Self, RouterError> {
        let mut state = Self {
            rules: CompiledRules::compile(rules),
            plans: std::array::from_fn(|_| PlanMemory::new()),
            cached_now: None,
            snaps_valid: false,
            snaps: [PaceSnap::fresh(); PROVIDER_COUNT],
        };
        state.pull_control(store)?;
        let mut since = now.saturating_sub(state.rules.max_lookback_ms());
        for plan in [ProviderId::Claude, ProviderId::Cursor, ProviderId::Local] {
            for window in &state.plans[plan_index(plan)].stored {
                if window.name == DASHBOARD_WINDOW {
                    since = since.min(window.observed_at);
                }
            }
        }
        for run in store.runs_since_all(since)? {
            state.absorb_loaded(&run);
        }
        state.rebuild_allowance(now);
        Ok(state)
    }

    /// Rest and reported windows only. Run history stays in the deque.
    pub fn pull_control(&mut self, store: &Store) -> Result<(), RouterError> {
        for plan in [ProviderId::Claude, ProviderId::Cursor, ProviderId::Local] {
            let until = store.resting_until(plan.as_str())?;
            let windows = store.windows(plan.as_str())?;
            let slot = &mut self.plans[plan_index(plan)];
            if slot.resting_until != until {
                slot.resting_until = until;
                self.snaps_valid = false;
            }
            if slot.stored != windows {
                slot.stored = windows;
                slot.dashboard_mark = None;
                self.snaps_valid = false;
            }
        }
        Ok(())
    }

    pub fn resting_deadline(&self, plan: ProviderId) -> Option<i64> {
        self.plans[plan_index(plan)].resting_until
    }

    pub fn set_resting(&mut self, plan: ProviderId, until: Option<i64>) {
        let slot = &mut self.plans[plan_index(plan)];
        if slot.resting_until != until {
            slot.resting_until = until;
            self.snaps_valid = false;
        }
    }

    pub fn set_windows(&mut self, plan: ProviderId, windows: Vec<StoredWindow>) {
        let slot = &mut self.plans[plan_index(plan)];
        if slot.stored != windows {
            slot.stored = windows;
            slot.dashboard_mark = None;
            self.snaps_valid = false;
        }
    }

    pub fn record_run(&mut self, row: &RunRow) {
        self.absorb_loaded(row);
        self.snaps_valid = false;
    }

    pub fn headroom(&mut self, provider: ProviderId, now: i64) -> f64 {
        self.prepare(now);
        self.snaps[provider_index(provider)].headroom
    }

    pub fn plan_usage(&mut self, provider: ProviderId, now: i64) -> PlanUsage {
        self.prepare(now);
        let slot = plan_index(provider);
        let (summary, windows) = self.describe(slot, now);
        usage_from_summary(summary, windows)
    }

    pub fn score_providers(
        &mut self,
        job: JobType,
        candidates: &[ProviderId],
        now: i64,
        exclude: &[ProviderId],
    ) -> Vec<Score> {
        self.prepare(now);
        scores_from(
            &self.rules,
            job,
            &ProviderSet::from_slice(candidates),
            &ProviderSet::from_slice(exclude),
            &self.snaps,
        )
    }

    pub fn pick_provider(
        &mut self,
        job: JobType,
        candidates: &[ProviderId],
        now: i64,
        exclude: &[ProviderId],
        running: &RunningCounts,
    ) -> Option<ProviderId> {
        self.pick_with_reason(job, candidates, now, exclude, running)
            .map(|pick| pick.provider)
    }

    /// Choose a provider. Reasons are `'static`. No heap allocation.
    pub fn pick_with_reason(
        &mut self,
        job: JobType,
        candidates: &[ProviderId],
        now: i64,
        exclude: &[ProviderId],
        running: &RunningCounts,
    ) -> Option<Pick> {
        self.prepare(now);
        decide(
            &self.rules,
            job,
            &ProviderSet::from_slice(candidates),
            &ProviderSet::from_slice(exclude),
            running,
            &self.snaps,
        )
    }

    pub fn compiled(&self) -> &CompiledRules {
        &self.rules
    }

    fn absorb_loaded(&mut self, row: &RunRow) {
        let Some(provider) = ProviderId::parse(&row.provider) else {
            return;
        };
        let point = RunPoint {
            started_at: row.started_at,
            id: row.id.unwrap_or(0),
            weight: run_weight(row.cost_usd, row.tokens),
        };
        self.insert_sorted(plan_index(provider), point);
    }

    fn insert_sorted(&mut self, slot: usize, point: RunPoint) {
        let plan = &mut self.plans[slot];
        let idx = lower_bound(&plan.runs, point.started_at, point.id);
        let in_allowance = point.started_at >= plan.allowance_since;
        let in_dashboard = plan
            .dashboard_mark
            .is_some_and(|mark| point.started_at >= mark);
        let weight = point.weight;
        plan.runs.insert(idx, point);
        if in_allowance {
            plan.allowance_sum += weight;
        } else {
            plan.below_allowance += 1;
        }
        if in_dashboard {
            plan.dashboard_sum += weight;
        }
    }

    fn rebuild_allowance(&mut self, now: i64) {
        for slot in 0..PROVIDER_COUNT {
            let since = self.allowance_since(slot, now);
            let plan = &mut self.plans[slot];
            plan.allowance_since = since;
            plan.allowance_sum = 0.0;
            plan.below_allowance = 0;
            plan.dashboard_mark = None;
            plan.dashboard_sum = 0.0;
            for (index, run) in plan.runs.iter().enumerate() {
                if run.started_at >= since {
                    plan.allowance_sum += run.weight;
                } else {
                    plan.below_allowance = index + 1;
                }
            }
        }
        self.snaps_valid = false;
        self.cached_now = None;
    }

    fn allowance_since(&self, slot: usize, now: i64) -> i64 {
        let window_ms = self
            .rules
            .allowance_at(slot)
            .map(|allowance| allowance.window_ms)
            .unwrap_or_else(|| self.rules.max_lookback_ms());
        now.saturating_sub(window_ms)
    }

    fn prepare(&mut self, now: i64) {
        if self.snaps_valid && self.cached_now == Some(now) {
            return;
        }
        for slot in 0..PROVIDER_COUNT {
            self.slide(slot, now);
        }
        let mut by_slot = [Summary::flat(0.0, None); PROVIDER_COUNT];
        for (slot, summary) in by_slot.iter_mut().enumerate() {
            *summary = self.summary_of(slot, now);
        }
        for provider in PROVIDERS {
            let slot = plan_index(provider);
            let resting = self.plans[slot]
                .resting_until
                .is_some_and(|until| until > now);
            self.snaps[provider_index(provider)] = PaceSnap::from_summary(by_slot[slot], resting);
        }
        self.cached_now = Some(now);
        self.snaps_valid = true;
    }

    fn slide(&mut self, slot: usize, now: i64) {
        let new_since = self.allowance_since(slot, now);
        let horizon = self.horizon(slot, now, new_since);
        let plan = &mut self.plans[slot];
        if new_since > plan.allowance_since {
            while plan.below_allowance < plan.runs.len() {
                let run = plan.runs[plan.below_allowance];
                if run.started_at >= new_since {
                    break;
                }
                plan.allowance_sum -= run.weight;
                plan.below_allowance += 1;
            }
            plan.allowance_since = new_since;
        } else if new_since < plan.allowance_since {
            while plan.below_allowance > 0 {
                let previous = plan.runs[plan.below_allowance - 1];
                if previous.started_at < new_since {
                    break;
                }
                plan.below_allowance -= 1;
                plan.allowance_sum += previous.weight;
            }
            plan.allowance_since = new_since;
        }
        while plan.below_allowance > 0 {
            let Some(front) = plan.runs.front().copied() else {
                break;
            };
            if front.started_at >= horizon {
                break;
            }
            plan.runs.pop_front();
            plan.below_allowance -= 1;
            if plan
                .dashboard_mark
                .is_some_and(|mark| front.started_at >= mark)
            {
                plan.dashboard_sum -= front.weight;
            }
        }
    }

    fn horizon(&self, slot: usize, now: i64, allowance_since: i64) -> i64 {
        let mut horizon = allowance_since;
        for window in &self.plans[slot].stored {
            if window.name == DASHBOARD_WINDOW {
                let (mark, _) = dashboard_cycle(window.resets_at, window.observed_at, now);
                horizon = horizon.min(mark);
            }
        }
        horizon
    }

    fn summary_of(&mut self, slot: usize, now: i64) -> Summary {
        if self.plans[slot].stored.is_empty() {
            return match self.rules.allowance_at(slot) {
                Some(allowance) => {
                    let used = weight_used(self.plans[slot].allowance_sum, allowance.units);
                    Summary::flat(crate::usage::clamp01(used), None)
                }
                None => Summary::flat(0.0, None),
            };
        }
        let len = self.plans[slot].stored.len();
        let mut acc = SummaryAcc::new();
        for index in 0..len {
            acc.push(self.mini_at(slot, index, now));
        }
        acc.finish(self.rules.pace, now)
    }

    fn mini_at(&mut self, slot: usize, index: usize, now: i64) -> MiniWindow {
        let (is_dashboard, utilization, resets_at, observed_at, length) = {
            let row = &self.plans[slot].stored[index];
            (
                row.name == DASHBOARD_WINDOW,
                row.utilization,
                row.resets_at,
                row.observed_at,
                crate::usage::length_ms_for(&row.name),
            )
        };
        if is_dashboard {
            let units = self.rules.allowance_at(slot).map(|row| row.units);
            let (mark, next_reset) = dashboard_cycle(resets_at, observed_at, now);
            let weight = self.weight_since(slot, mark);
            let include_reading = resets_at.is_none_or(|resets| resets > now);
            dashboard_mini(utilization, weight, units, next_reset, include_reading)
        } else if resets_at.is_some_and(|resets| resets <= now) {
            MiniWindow {
                used: 0.0,
                resets_at: None,
                length_ms: length,
            }
        } else {
            MiniWindow {
                used: crate::usage::clamp01(utilization),
                resets_at,
                length_ms: length,
            }
        }
    }

    fn weight_since(&mut self, slot: usize, mark: i64) -> f64 {
        let plan = &mut self.plans[slot];
        if plan.dashboard_mark == Some(mark) {
            return plan.dashboard_sum;
        }
        let sum = plan
            .runs
            .iter()
            .filter(|run| run.started_at >= mark)
            .map(|run| run.weight)
            .sum();
        plan.dashboard_mark = Some(mark);
        plan.dashboard_sum = sum;
        sum
    }

    fn describe(&mut self, slot: usize, now: i64) -> (Summary, Vec<WindowState>) {
        if self.plans[slot].stored.is_empty() {
            return (self.summary_of(slot, now), Vec::new());
        }
        let len = self.plans[slot].stored.len();
        let mut windows = Vec::with_capacity(len);
        let mut acc = SummaryAcc::new();
        for index in 0..len {
            let name = self.plans[slot].stored[index].name.clone();
            let mini = self.mini_at(slot, index, now);
            acc.push(mini);
            windows.push(WindowState {
                name,
                used: mini.used,
                resets_at: mini.resets_at,
                length_ms: mini.length_ms,
            });
        }
        (acc.finish(self.rules.pace, now), windows)
    }
}

/// Referenced by nothing; dashboard readings live in `usage`.
fn lower_bound(runs: &VecDeque<RunPoint>, started_at: i64, id: i64) -> usize {
    let mut lo = 0;
    let mut hi = runs.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let run = runs[mid];
        if run.started_at < started_at || (run.started_at == started_at && run.id < id) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}
