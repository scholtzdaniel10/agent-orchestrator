//! Scoring from `src/core/router/router.ts`.
//!
//! Pure functions read a [`UsageSource`] on every call. [`crate::RouterState`] answers the same
//! questions from memory and does not allocate while picking.
//!
//! A plan with no allowance row — `local` has none in the bundled rules — does not panic.
//! With no reported windows its headroom is 1 (unlimited) unless it is resting. Reported
//! windows still apply. A dashboard reading with no units does not add run weight: there is
//! no finite quota for those runs to spend down.

use legate_core::{
    DEFAULT_PACE, JobType, PROVIDERS, PaceRules, PlanAllowance, ProviderId, RouterRules,
};
use legate_store::{RunRow, Store, StoredWindow};
use serde_json::Value;

use crate::ids::{
    JOB_COUNT, MAX_ORDER, PROVIDER_COUNT, ProviderSet, RunningCounts, job_index, jobs_on_plan,
    plan_index, provider_index,
};

/// Name of a dashboard reading copied from a plan's usage page.
pub const DASHBOARD_WINDOW: &str = "dashboard";

const FIVE_HOUR_MS: i64 = 5 * 3_600_000;
const SEVEN_DAY_MS: i64 = 168 * 3_600_000;
const DASHBOARD_MS: i64 = 30 * 24 * 3_600_000;

pub const REASON_ONLY: &str = "only plan available";
pub const REASON_EXPIRING: &str = "allowance expiring";
pub const REASON_FIRST: &str = "first choice";
pub const REASON_BEHIND: &str = "behind pace";
pub const REASON_HEADROOM: &str = "more headroom";

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error(transparent)]
    Store(#[from] legate_store::StoreError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowState {
    pub name: String,
    pub used: f64,
    pub resets_at: Option<i64>,
    pub length_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanUsage {
    pub windows: Vec<WindowState>,
    pub used: f64,
    pub resets_at: Option<i64>,
    pub slack: Option<f64>,
    pub pace_factor: f64,
    pub at_risk: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Score {
    pub provider: ProviderId,
    pub fit: f64,
    pub headroom: f64,
    pub pace_factor: f64,
    pub at_risk: bool,
    pub score: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub provider: ProviderId,
    pub reason: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DashboardReading {
    pub name: &'static str,
    pub utilization: f64,
    pub resets_at: i64,
}

/// What the pure path needs from a ledger. The store implements this.
pub trait UsageSource {
    fn resting_until(&self, plan: ProviderId) -> Result<Option<i64>, RouterError>;
    fn windows(&self, plan: ProviderId) -> Result<Vec<StoredWindow>, RouterError>;
    fn runs_since(&self, provider: ProviderId, since: i64) -> Result<Vec<RunRow>, RouterError>;
}

impl UsageSource for Store {
    fn resting_until(&self, plan: ProviderId) -> Result<Option<i64>, RouterError> {
        Ok(Store::resting_until(self, plan.as_str())?)
    }

    fn windows(&self, plan: ProviderId) -> Result<Vec<StoredWindow>, RouterError> {
        Ok(Store::windows(self, plan.as_str())?)
    }

    fn runs_since(&self, provider: ProviderId, since: i64) -> Result<Vec<RunRow>, RouterError> {
        Ok(Store::runs_since(self, provider.as_str(), since)?)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Allowance {
    pub window_ms: i64,
    pub units: f64,
}

/// Rules copied into fixed tables. Built once; a pick only indexes them.
#[derive(Clone, Copy)]
pub struct CompiledRules {
    order: [[ProviderId; MAX_ORDER]; JOB_COUNT],
    order_len: [usize; JOB_COUNT],
    pub fallback_fit: f64,
    pub pace: PaceRules,
    allowance: [Option<Allowance>; PROVIDER_COUNT],
    pub max_parallel: i64,
    pub default_rest_hours: f64,
}

impl CompiledRules {
    pub fn compile(rules: &RouterRules) -> Self {
        let mut order = [[ProviderId::Local; MAX_ORDER]; JOB_COUNT];
        let mut order_len = [0usize; JOB_COUNT];
        for job in JobType::ALL {
            let list = rules.rules.get(job);
            let n = list.len().min(MAX_ORDER);
            let slot = job_index(job);
            order_len[slot] = n;
            for (i, provider) in list.iter().take(n).enumerate() {
                order[slot][i] = *provider;
            }
        }
        let mut allowance = [None; PROVIDER_COUNT];
        for (id, row) in &rules.allowance {
            allowance[provider_index(*id)] = Some(Allowance::from_plan(row));
        }
        Self {
            order,
            order_len,
            fallback_fit: rules.fallback_fit,
            pace: rules.pace.unwrap_or(DEFAULT_PACE),
            allowance,
            max_parallel: rules.max_parallel,
            default_rest_hours: rules.default_rest_hours,
        }
    }

    pub(crate) fn order(&self, job: JobType) -> &[ProviderId] {
        let slot = job_index(job);
        &self.order[slot][..self.order_len[slot]]
    }

    pub(crate) fn first(&self, job: JobType) -> Option<ProviderId> {
        self.order(job).first().copied()
    }

    /// Allowance for a plan. `None` means unlimited.
    pub(crate) fn allowance(&self, plan: ProviderId) -> Option<Allowance> {
        self.allowance_at(plan_index(plan))
    }

    pub(crate) fn allowance_at(&self, slot: usize) -> Option<Allowance> {
        self.allowance.get(slot).copied().flatten()
    }

    pub(crate) fn max_lookback_ms(&self) -> i64 {
        let mut lookback = DASHBOARD_MS;
        for row in self.allowance.into_iter().flatten() {
            lookback = lookback.max(row.window_ms);
        }
        // A calendar cycle can run slightly past the 30-day dashboard length.
        lookback.max(32 * 24 * 3_600_000)
    }
}

impl Allowance {
    fn from_plan(row: &PlanAllowance) -> Self {
        Self {
            window_ms: hours_to_ms(row.window_hours),
            units: row.units,
        }
    }
}

fn hours_to_ms(hours: f64) -> i64 {
    let ms = hours * 3_600_000.0;
    if ms.is_finite() { ms as i64 } else { 0 }
}

/// Weight of one run. A present cost wins, including 0. Otherwise tokens, including 0.
/// Missing both counts as 1.
pub fn run_weight(cost_usd: Option<f64>, tokens: Option<i64>) -> f64 {
    if let Some(cost) = cost_usd {
        return cost;
    }
    if let Some(tokens) = tokens {
        return tokens as f64;
    }
    1.0
}

pub(crate) fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

pub(crate) fn clamp01(value: f64) -> f64 {
    clamp(value, 0.0, 1.0)
}

fn later_reset(a: Option<i64>, b: Option<i64>) -> bool {
    match (a, b) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(left), Some(right)) => left > right,
    }
}

pub(crate) fn length_ms_for(name: &str) -> Option<i64> {
    match name {
        "five_hour" => Some(FIVE_HOUR_MS),
        "seven_day" => Some(SEVEN_DAY_MS),
        DASHBOARD_WINDOW => Some(DASHBOARD_MS),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct MiniWindow {
    pub used: f64,
    pub resets_at: Option<i64>,
    pub length_ms: Option<i64>,
}

#[derive(Clone, Copy)]
pub(crate) struct Summary {
    pub used: f64,
    pub resets_at: Option<i64>,
    pub slack: Option<f64>,
    pub pace_factor: f64,
    pub at_risk: bool,
}

impl Summary {
    pub(crate) fn flat(used: f64, resets_at: Option<i64>) -> Self {
        Self {
            used,
            resets_at,
            slack: None,
            pace_factor: 1.0,
            at_risk: false,
        }
    }
}

/// One pass over resolved windows. Holds only the worst window and the longest open one.
#[derive(Clone, Copy)]
pub(crate) struct SummaryAcc {
    worst: Option<MiniWindow>,
    pace_window: Option<MiniWindow>,
}

impl SummaryAcc {
    pub(crate) const fn new() -> Self {
        Self {
            worst: None,
            pace_window: None,
        }
    }

    pub(crate) fn push(&mut self, window: MiniWindow) {
        self.worst = Some(match self.worst {
            None => window,
            Some(worst)
                if window.used > worst.used
                    || (window.used == worst.used
                        && later_reset(window.resets_at, worst.resets_at)) =>
            {
                window
            }
            Some(worst) => worst,
        });
        if window.length_ms.is_some() && window.resets_at.is_some() {
            self.pace_window = Some(match self.pace_window {
                None => window,
                Some(best) if window.length_ms.unwrap_or(0) > best.length_ms.unwrap_or(0) => window,
                Some(best) => best,
            });
        }
    }

    pub(crate) fn finish(self, pace: PaceRules, now: i64) -> Summary {
        let Some(worst) = self.worst else {
            return Summary::flat(0.0, None);
        };
        let Some(pace_window) = self.pace_window else {
            return Summary::flat(worst.used, worst.resets_at);
        };
        let (Some(length), Some(pace_reset)) = (pace_window.length_ms, pace_window.resets_at)
        else {
            return Summary::flat(worst.used, worst.resets_at);
        };
        if length == 0 {
            return Summary::flat(worst.used, worst.resets_at);
        }
        let left = clamp01((pace_reset - now) as f64 / length as f64);
        let slack = 1.0 - left - pace_window.used;
        let pace_factor = clamp(1.0 + pace.weight * slack, pace.min, pace.max);
        let at_risk =
            slack >= pace.at_risk_slack && left <= pace.at_risk_left && 1.0 - worst.used > 0.1;
        Summary {
            used: worst.used,
            resets_at: worst.resets_at,
            slack: Some(slack),
            pace_factor,
            at_risk,
        }
    }
}

/// Summarize windows already resolved at `now`.
pub(crate) fn summarize_at(windows: &[MiniWindow], pace: PaceRules, now: i64) -> Summary {
    let mut acc = SummaryAcc::new();
    for window in windows {
        acc.push(*window);
    }
    acc.finish(pace, now)
}

pub(crate) fn to_mini(
    name: &str,
    utilization: f64,
    resets_at: Option<i64>,
    now: i64,
) -> MiniWindow {
    let length_ms = length_ms_for(name);
    if resets_at.is_some_and(|resets| resets <= now) {
        return MiniWindow {
            used: 0.0,
            resets_at: None,
            length_ms,
        };
    }
    MiniWindow {
        used: clamp01(utilization),
        resets_at,
        length_ms,
    }
}

pub(crate) fn weight_used(weight: f64, units: f64) -> f64 {
    weight / units
}

/// The same local day and time `months` calendar months later, clamped to the month's last day.
/// Counted from one anchor so a cycle that starts on the 31st stays on the 31st after a short month.
pub fn add_months(ms: i64, months: i32) -> i64 {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    let Some(utc) = chrono::DateTime::from_timestamp_millis(ms) else {
        return ms;
    };
    let local = utc.with_timezone(&Local);
    let day = local.day();
    let month0 = local.month0() as i32 + months;
    let year = local.year() + month0.div_euclid(12);
    let month = (month0.rem_euclid(12) + 1) as u32;
    let last = last_day_of_month(year, month);
    let day = day.min(last);
    match Local.with_ymd_and_hms(
        year,
        month,
        day,
        local.hour(),
        local.minute(),
        local.second(),
    ) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
            dt.timestamp_millis() + i64::from(local.timestamp_subsec_millis())
        }
        chrono::LocalResult::None => ms,
    }
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    use chrono::Datelike;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|first| first.pred_opt())
        .map(|day| day.day())
        .unwrap_or(28)
}

/// `(mark, next_reset)` for a dashboard window at `now`.
/// Before the stored reset, the mark is the reading time. After it, the mark is the current
/// cycle start and `next_reset` is one calendar month later.
pub(crate) fn dashboard_cycle(
    resets_at: Option<i64>,
    observed_at: i64,
    now: i64,
) -> (i64, Option<i64>) {
    if resets_at.is_none_or(|resets| resets > now) {
        return (observed_at, resets_at);
    }
    let anchor = resets_at.unwrap_or(observed_at);
    let mut months = 1i32;
    let mut cycle_start = anchor;
    let mut next = add_months(anchor, months);
    while next <= now {
        cycle_start = next;
        months += 1;
        next = add_months(anchor, months);
    }
    (cycle_start, Some(next))
}

pub(crate) fn dashboard_mini(
    utilization: f64,
    weight: f64,
    units: Option<f64>,
    next_reset: Option<i64>,
    include_reading: bool,
) -> MiniWindow {
    let extra = units.map(|units| weight_used(weight, units)).unwrap_or(0.0);
    let used = if include_reading {
        clamp01(utilization + extra)
    } else {
        clamp01(extra)
    };
    MiniWindow {
        used,
        resets_at: next_reset,
        length_ms: Some(DASHBOARD_MS),
    }
}

/// Numbers a pick needs. Built on the stack.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PaceSnap {
    #[allow(dead_code)]
    pub used: f64,
    pub headroom: f64,
    pub pace_factor: f64,
    pub at_risk: bool,
    #[allow(dead_code)]
    pub resets_at: Option<i64>,
    #[allow(dead_code)]
    pub resting: bool,
}

impl PaceSnap {
    pub(crate) fn from_summary(summary: Summary, resting: bool) -> Self {
        let headroom = if resting {
            0.0
        } else {
            clamp01(1.0 - summary.used)
        };
        Self {
            used: summary.used,
            headroom,
            pace_factor: summary.pace_factor,
            at_risk: summary.at_risk,
            resets_at: summary.resets_at,
            resting,
        }
    }

    pub(crate) const fn fresh() -> Self {
        Self {
            used: 0.0,
            headroom: 1.0,
            pace_factor: 1.0,
            at_risk: false,
            resets_at: None,
            resting: false,
        }
    }
}

pub(crate) fn usage_from_summary(summary: Summary, windows: Vec<WindowState>) -> PlanUsage {
    PlanUsage {
        windows,
        used: summary.used,
        resets_at: summary.resets_at,
        slack: summary.slack,
        pace_factor: summary.pace_factor,
        at_risk: summary.at_risk,
    }
}

struct Scored {
    scores: [Score; MAX_ORDER],
    len: usize,
}

fn score_with(
    rules: &CompiledRules,
    job: JobType,
    candidates: &ProviderSet,
    exclude: &ProviderSet,
    snaps: &[PaceSnap; PROVIDER_COUNT],
) -> Scored {
    let pace = rules.pace;
    let empty = Score {
        provider: ProviderId::Local,
        fit: 0.0,
        headroom: 0.0,
        pace_factor: 0.0,
        at_risk: false,
        score: 0.0,
    };
    let mut scores = [empty; MAX_ORDER];
    let mut len = 0;
    for (i, provider) in rules.order(job).iter().copied().enumerate() {
        if !candidates.contains(provider) || exclude.contains(provider) {
            continue;
        }
        let snap = snaps[provider_index(provider)];
        let at_risk = snap.at_risk;
        let fit = if i == 0 || at_risk {
            1.0
        } else {
            rules.fallback_fit
        };
        let pace_factor = if at_risk { pace.max } else { snap.pace_factor };
        scores[len] = Score {
            provider,
            fit,
            headroom: snap.headroom,
            pace_factor,
            at_risk,
            score: fit * snap.headroom * pace_factor,
        };
        len += 1;
    }
    Scored { scores, len }
}

pub(crate) fn decide(
    rules: &CompiledRules,
    job: JobType,
    candidates: &ProviderSet,
    exclude: &ProviderSet,
    running: &RunningCounts,
    snaps: &[PaceSnap; PROVIDER_COUNT],
) -> Option<Pick> {
    let scored = score_with(rules, job, candidates, exclude, snaps);
    let rows = &scored.scores[..scored.len];
    let mut best_at: Option<usize> = None;
    for (index, row) in rows.iter().enumerate() {
        let best_score = best_at.map(|at| rows[at].score).unwrap_or(0.0);
        if row.score > best_score {
            best_at = Some(index);
        }
    }
    let best_at = best_at?;
    let limit = u32::try_from(rules.max_parallel.max(0)).unwrap_or(u32::MAX);
    let mut winner_at = best_at;
    if jobs_on_plan(running, rows[winner_at].provider) >= limit {
        for (index, row) in rows.iter().enumerate() {
            if index == winner_at || row.score <= 0.0 {
                continue;
            }
            if jobs_on_plan(running, row.provider) >= limit {
                continue;
            }
            if row.headroom <= 0.0 {
                continue;
            }
            winner_at = index;
            break;
        }
    }
    let winner = &rows[winner_at];
    Some(Pick {
        provider: winner.provider,
        reason: pick_reason(job, winner, rows, rules),
    })
}

fn pick_reason(
    job: JobType,
    winner: &Score,
    scores: &[Score],
    rules: &CompiledRules,
) -> &'static str {
    let another = scores
        .iter()
        .any(|row| row.provider != winner.provider && row.score > 0.0);
    if !another {
        return REASON_ONLY;
    }
    if winner.at_risk {
        return REASON_EXPIRING;
    }
    if Some(winner.provider) == rules.first(job) {
        return REASON_FIRST;
    }
    if winner.pace_factor > 1.05 {
        return REASON_BEHIND;
    }
    REASON_HEADROOM
}

pub(crate) fn scores_from(
    rules: &CompiledRules,
    job: JobType,
    candidates: &ProviderSet,
    exclude: &ProviderSet,
    snaps: &[PaceSnap; PROVIDER_COUNT],
) -> Vec<Score> {
    let scored = score_with(rules, job, candidates, exclude, snaps);
    scored.scores[..scored.len].to_vec()
}

fn runs_on_plan(
    source: &impl UsageSource,
    plan: ProviderId,
    since: i64,
) -> Result<Vec<RunRow>, RouterError> {
    let mut rows = Vec::new();
    for id in PROVIDERS {
        if legate_core::plan_of(id) != plan {
            continue;
        }
        rows.extend(source.runs_since(id, since)?);
    }
    rows.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.id.unwrap_or(0).cmp(&b.id.unwrap_or(0)))
    });
    Ok(rows)
}

fn weight_of_rows(rows: &[RunRow]) -> f64 {
    rows.iter()
        .map(|row| run_weight(row.cost_usd, row.tokens))
        .sum()
}

fn plan_summary(
    source: &impl UsageSource,
    provider: ProviderId,
    rules: &CompiledRules,
    now: i64,
) -> Result<(Summary, Vec<WindowState>, bool), RouterError> {
    let plan = legate_core::plan_of(provider);
    let resting = source.resting_until(plan)?.is_some_and(|until| until > now);
    let stored = source.windows(plan)?;
    if stored.is_empty() {
        let summary = match rules.allowance(plan) {
            Some(allowance) => {
                let since = now - allowance.window_ms;
                let weight = weight_of_rows(&runs_on_plan(source, plan, since)?);
                Summary::flat(clamp01(weight_used(weight, allowance.units)), None)
            }
            None => Summary::flat(0.0, None),
        };
        return Ok((summary, Vec::new(), resting));
    }
    let units = rules.allowance(plan).map(|row| row.units);
    let mut minis = Vec::with_capacity(stored.len());
    let mut windows = Vec::with_capacity(stored.len());
    for row in &stored {
        let mini = if row.name == DASHBOARD_WINDOW {
            let (mark, next_reset) = dashboard_cycle(row.resets_at, row.observed_at, now);
            let weight = weight_of_rows(&runs_on_plan(source, plan, mark)?);
            let include_reading = row.resets_at.is_none_or(|resets| resets > now);
            dashboard_mini(row.utilization, weight, units, next_reset, include_reading)
        } else {
            to_mini(&row.name, row.utilization, row.resets_at, now)
        };
        windows.push(WindowState {
            name: row.name.clone(),
            used: mini.used,
            resets_at: mini.resets_at,
            length_ms: mini.length_ms,
        });
        minis.push(mini);
    }
    Ok((summarize_at(&minis, rules.pace, now), windows, resting))
}

pub(crate) fn snaps_from_source(
    source: &impl UsageSource,
    rules: &CompiledRules,
    now: i64,
) -> Result<[PaceSnap; PROVIDER_COUNT], RouterError> {
    let mut snaps = [PaceSnap::fresh(); PROVIDER_COUNT];
    for provider in PROVIDERS {
        let (summary, _, resting) = plan_summary(source, provider, rules, now)?;
        snaps[provider_index(provider)] = PaceSnap::from_summary(summary, resting);
    }
    Ok(snaps)
}

/// Fraction of the provider's allowance still unused, in [0, 1]. Resting is 0.
pub fn headroom(
    provider: ProviderId,
    source: &impl UsageSource,
    rules: &RouterRules,
    now: i64,
) -> Result<f64, RouterError> {
    let compiled = CompiledRules::compile(rules);
    let (summary, _, resting) = plan_summary(source, provider, &compiled, now)?;
    Ok(PaceSnap::from_summary(summary, resting).headroom)
}

pub fn plan_usage(
    provider: ProviderId,
    source: &impl UsageSource,
    rules: &RouterRules,
    now: i64,
) -> Result<PlanUsage, RouterError> {
    let compiled = CompiledRules::compile(rules);
    let (summary, windows, _) = plan_summary(source, provider, &compiled, now)?;
    Ok(usage_from_summary(summary, windows))
}

pub fn score_providers(
    job: JobType,
    candidates: &[ProviderId],
    source: &impl UsageSource,
    rules: &RouterRules,
    now: i64,
    exclude: &[ProviderId],
) -> Result<Vec<Score>, RouterError> {
    let compiled = CompiledRules::compile(rules);
    let snaps = snaps_from_source(source, &compiled, now)?;
    Ok(scores_from(
        &compiled,
        job,
        &ProviderSet::from_slice(candidates),
        &ProviderSet::from_slice(exclude),
        &snaps,
    ))
}

pub fn pick_provider(
    job: JobType,
    candidates: &[ProviderId],
    source: &impl UsageSource,
    rules: &RouterRules,
    now: i64,
    exclude: &[ProviderId],
    running: &RunningCounts,
) -> Result<Option<ProviderId>, RouterError> {
    Ok(
        pick_with_reason(job, candidates, source, rules, now, exclude, running)?
            .map(|pick| pick.provider),
    )
}

pub fn pick_with_reason(
    job: JobType,
    candidates: &[ProviderId],
    source: &impl UsageSource,
    rules: &RouterRules,
    now: i64,
    exclude: &[ProviderId],
    running: &RunningCounts,
) -> Result<Option<Pick>, RouterError> {
    let compiled = CompiledRules::compile(rules);
    let snaps = snaps_from_source(source, &compiled, now)?;
    Ok(decide(
        &compiled,
        job,
        &ProviderSet::from_slice(candidates),
        &ProviderSet::from_slice(exclude),
        running,
        &snaps,
    ))
}

/// A dashboard reading. `None` is any non-number (a string, null, or NaN).
pub fn dashboard_reading(
    used_percent: Option<f64>,
    resets_at: Option<i64>,
    now: i64,
) -> Result<DashboardReading, &'static str> {
    let Some(used) = used_percent.filter(|value| value.is_finite()) else {
        return Err("Enter how much is used, as a percentage.");
    };
    if !(0.0..=100.0).contains(&used) {
        return Err("Usage must be between 0 and 100%.");
    }
    let Some(resets) = resets_at else {
        return Err("Enter the date your usage resets.");
    };
    if resets <= now {
        return Err("The reset date has to be in the future.");
    }
    Ok(DashboardReading {
        name: DASHBOARD_WINDOW,
        utilization: used / 100.0,
        resets_at: resets,
    })
}

/// JSON form of [`dashboard_reading`], for values copied off a page.
pub fn dashboard_reading_value(
    used_percent: &Value,
    resets_at: &Value,
    now: i64,
) -> Result<DashboardReading, &'static str> {
    let used = match used_percent {
        Value::Number(number) => number.as_f64(),
        _ => None,
    };
    let resets = match resets_at {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value as i64)),
        _ => None,
    };
    dashboard_reading(used, resets, now)
}
