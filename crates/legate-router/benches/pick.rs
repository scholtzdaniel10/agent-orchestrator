//! `pick_with_reason` with 4 providers, 10_000 runs, and 2 windows per plan.
//!
//! The pure path recomputes from an in-memory [`UsageSource`]. [`RouterState`] was hydrated
//! once and then only slides its cache. Windows are not the dashboard, so neither path scans
//! the 10_000 runs; the cache still avoids the recompute.

use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use legate_core::{JobType, ProviderId, load_rules};
use legate_router::{RouterState, RunningCounts, UsageSource, pick_with_reason};
use legate_store::{NewWindow, RunOutcome, RunRow, Store, StoredWindow};

const NOW: i64 = 1_800_000_000_000;
const RUNS: i64 = 10_000;

struct Mem {
    runs: Vec<RunRow>,
    windows: HashMap<ProviderId, Vec<StoredWindow>>,
}

impl UsageSource for Mem {
    fn resting_until(&self, _plan: ProviderId) -> Result<Option<i64>, legate_router::RouterError> {
        Ok(None)
    }

    fn windows(&self, plan: ProviderId) -> Result<Vec<StoredWindow>, legate_router::RouterError> {
        Ok(self.windows.get(&plan).cloned().unwrap_or_default())
    }

    fn runs_since(
        &self,
        provider: ProviderId,
        since: i64,
    ) -> Result<Vec<RunRow>, legate_router::RouterError> {
        Ok(self
            .runs
            .iter()
            .filter(|row| row.provider == provider.as_str() && row.started_at >= since)
            .cloned()
            .collect())
    }
}

fn windows(now: i64) -> Vec<StoredWindow> {
    vec![
        StoredWindow {
            name: "five_hour".into(),
            utilization: 0.2,
            resets_at: Some(now + 3_600_000),
            observed_at: now,
        },
        StoredWindow {
            name: "seven_day".into(),
            utilization: 0.45,
            resets_at: Some(now + 168 * 3_600_000),
            observed_at: now,
        },
    ]
}

fn provider_name(index: i64) -> &'static str {
    match index % 4 {
        0 => "claude",
        1 => "cursor",
        2 => "grok",
        _ => "local",
    }
}

fn runs(now: i64) -> Vec<RunRow> {
    (0..RUNS)
        .map(|i| RunRow {
            id: Some(i),
            job_id: format!("job-{i}"),
            provider: provider_name(i).to_string(),
            job_type: "planning".to_string(),
            started_at: now - i,
            duration_ms: Some(1),
            cost_usd: Some(1.0),
            tokens: Some(10),
            utilization: None,
            outcome: RunOutcome::Ok,
            session_id: None,
        })
        .collect()
}

fn memory(now: i64) -> Mem {
    let rows = runs(now);
    let mut windows_by_plan = HashMap::new();
    for plan in [ProviderId::Claude, ProviderId::Cursor, ProviderId::Local] {
        windows_by_plan.insert(plan, windows(now));
    }
    Mem {
        runs: rows,
        windows: windows_by_plan,
    }
}

fn hydrated(now: i64) -> RouterState {
    let store = Store::open(":memory:").expect("memory store");
    for row in runs(now) {
        store.insert_run(&row).expect("insert");
    }
    let saved: Vec<NewWindow> = windows(now)
        .into_iter()
        .map(|window| NewWindow {
            name: window.name,
            utilization: window.utilization,
            resets_at: window.resets_at,
        })
        .collect();
    for plan in ["claude", "cursor", "local"] {
        store.save_windows(plan, &saved, now).expect("windows");
    }
    let rules = load_rules(None).expect("rules");
    RouterState::hydrate(&store, &rules, now).expect("hydrate")
}

fn pick(c: &mut Criterion) {
    let rules = load_rules(None).expect("rules");
    let mem = memory(NOW);
    let mut state = hydrated(NOW);
    let candidates = [
        ProviderId::Claude,
        ProviderId::Cursor,
        ProviderId::Grok,
        ProviderId::Local,
    ];
    let running = RunningCounts::default();
    let mut group = c.benchmark_group("pick_with_reason");
    group.sample_size(50);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("pure", |b| {
        b.iter(|| {
            black_box(
                pick_with_reason(
                    JobType::Planning,
                    &candidates,
                    &mem,
                    &rules,
                    NOW,
                    &[],
                    &running,
                )
                .expect("pick"),
            )
        });
    });
    group.bench_function("router_state", |b| {
        b.iter(|| {
            black_box(state.pick_with_reason(JobType::Planning, &candidates, NOW, &[], &running))
        });
    });
    group.finish();
}

criterion_group!(benches, pick);
criterion_main!(benches);
