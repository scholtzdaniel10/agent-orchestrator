//! Ports of `src/core/router/router.test.ts`.
//!
//! The bundled rules append `local` to every job list. The TypeScript file does not.
//! `load_rules` on a path uses the file as written and does not append `local`.

use std::fs;
use std::path::PathBuf;

use chrono::{Local, TimeZone};
use legate_core::{
    DEFAULT_PACE, JobType, PaceRules, PlanAllowance, ProviderId, RouterRules, load_rules,
};
use legate_router::{
    DASHBOARD_WINDOW, RouterState, RunningCounts, add_months, dashboard_reading,
    dashboard_reading_value, headroom, pick_provider, pick_with_reason, plan_usage,
    score_providers,
};
use legate_store::{NewWindow, RunOutcome, RunRow, Store};
use serde_json::Value;

const NOW: i64 = 50_000_000;
const PACE_NOW: i64 = 1_800_000_000_000;
const WEEK_MS: i64 = 168 * 3_600_000;
const HOUR5_MS: i64 = 5 * 3_600_000;

fn memory() -> Store {
    Store::open(":memory:").unwrap()
}

fn rules() -> RouterRules {
    load_rules(None).unwrap()
}

fn insert(
    store: &Store,
    provider: ProviderId,
    started_at: i64,
    cost_usd: Option<f64>,
    tokens: Option<i64>,
) {
    store
        .insert_run(&RunRow {
            id: None,
            job_id: "job".into(),
            provider: provider.as_str().into(),
            job_type: "planning".into(),
            started_at,
            duration_ms: Some(1),
            cost_usd,
            tokens,
            utilization: None,
            outcome: RunOutcome::Ok,
            session_id: None,
        })
        .unwrap();
}

fn window(name: &str, utilization: f64, resets_at: i64) -> NewWindow {
    NewWindow {
        name: name.into(),
        utilization,
        resets_at: Some(resets_at),
    }
}

fn save(store: &Store, plan: ProviderId, windows: &[NewWindow], observed: i64) {
    store
        .save_windows(plan.as_str(), windows, observed)
        .unwrap();
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{actual} is not close to {expected}"
    );
}

fn local_ms(year: i32, month: u32, day: u32, hour: u32) -> i64 {
    Local
        .with_ymd_and_hms(year, month, day, hour, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis()
}

/// The pure functions and [`RouterState`] answer the same question.
fn agree(store: &Store, rules: &RouterRules, now: i64) {
    let mut state = RouterState::hydrate(store, rules, now).unwrap();
    let running = RunningCounts::default();
    let candidates = [
        ProviderId::Claude,
        ProviderId::Cursor,
        ProviderId::Grok,
        ProviderId::Local,
    ];
    for id in candidates {
        near(
            headroom(id, store, rules, now).unwrap(),
            state.headroom(id, now),
        );
        let pure = plan_usage(id, store, rules, now).unwrap();
        let hot = state.plan_usage(id, now);
        near(pure.used, hot.used);
        assert_eq!(pure.resets_at, hot.resets_at);
        assert_eq!(pure.at_risk, hot.at_risk);
        assert_eq!(pure.windows.len(), hot.windows.len());
        match (pure.slack, hot.slack) {
            (None, None) => {}
            (Some(left), Some(right)) => near(left, right),
            (left, right) => panic!("slack {left:?} vs {right:?}"),
        }
        near(pure.pace_factor, hot.pace_factor);
    }
    for job in JobType::ALL {
        let pure = pick_with_reason(job, &candidates, store, rules, now, &[], &running).unwrap();
        let hot = state.pick_with_reason(job, &candidates, now, &[], &running);
        assert_eq!(pure, hot);
        assert_eq!(
            hot,
            state.pick_with_reason(job, &candidates, now, &[], &running)
        );
    }
}

fn temp_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("legate-router-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn load_rules_parses_the_bundled_file() {
    let rules = rules();
    assert_eq!(rules.fallback_fit, 0.5);
    assert_eq!(rules.default_rest_hours, 5.0);
    assert_eq!(rules.max_parallel, 3);
    assert_eq!(
        rules.allowance.get(&ProviderId::Claude),
        Some(&PlanAllowance {
            window_hours: 5.0,
            units: 15.0
        })
    );
    assert_eq!(
        rules.allowance.get(&ProviderId::Cursor),
        Some(&PlanAllowance {
            window_hours: 720.0,
            units: 20_000_000.0
        })
    );
    let mut keys: Vec<_> = rules.allowance.keys().map(|id| id.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["claude", "cursor"]);
    // Bundled Rust rules append local. The TypeScript lists stop at grok.
    assert_eq!(
        rules.rules.planning,
        vec![
            ProviderId::Claude,
            ProviderId::Codex,
            ProviderId::Cursor,
            ProviderId::Grok,
            ProviderId::Local
        ]
    );
    assert_eq!(rules.rules.review, rules.rules.planning);
    assert_eq!(
        rules.rules.debugging,
        vec![
            ProviderId::Grok,
            ProviderId::Cursor,
            ProviderId::Codex,
            ProviderId::Claude,
            ProviderId::Local
        ]
    );
    assert_eq!(rules.rules.refactor, rules.rules.debugging);
    assert_eq!(rules.rules.boilerplate, rules.rules.debugging);
    assert_eq!(rules.pace, Some(DEFAULT_PACE));
    for job in JobType::ALL {
        assert!(!rules.rules.get(job).is_empty());
    }
}

#[test]
fn load_rules_defaults_and_clamps_max_parallel() {
    let dir = Temp(temp_dir());
    let path = dir.0.join("rules.json");
    let bundled = serde_json::to_value(rules()).unwrap();
    let mut without = bundled.clone();
    without.as_object_mut().unwrap().remove("maxParallel");
    fs::write(&path, serde_json::to_string(&without).unwrap()).unwrap();
    assert_eq!(load_rules(Some(&path)).unwrap().max_parallel, 3);

    for (value, expect) in [(0, 1), (99, 8), (2, 2)] {
        let mut body = bundled.clone();
        body["maxParallel"] = Value::from(value);
        fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
        assert_eq!(load_rules(Some(&path)).unwrap().max_parallel, expect);
    }
    for bad in [
        Value::from(1.5),
        Value::String("3".into()),
        Value::Null,
        Value::Bool(true),
    ] {
        let mut body = bundled.clone();
        body["maxParallel"] = bad;
        fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
        assert_eq!(load_rules(Some(&path)).unwrap().max_parallel, 3);
    }
}

#[test]
fn load_rules_path_loads_an_edited_copy() {
    let dir = Temp(temp_dir());
    let path = dir.0.join("rules.json");
    let mut edited = rules();
    edited.fallback_fit = 0.9;
    edited.rules.boilerplate = vec![ProviderId::Claude, ProviderId::Cursor];
    fs::write(&path, serde_json::to_string(&edited).unwrap()).unwrap();
    let loaded = load_rules(Some(&path)).unwrap();
    assert_eq!(loaded.fallback_fit, 0.9);
    assert_eq!(
        loaded.rules.boilerplate,
        vec![ProviderId::Claude, ProviderId::Cursor]
    );
    assert_eq!(
        rules().rules.boilerplate,
        vec![
            ProviderId::Grok,
            ProviderId::Cursor,
            ProviderId::Codex,
            ProviderId::Claude,
            ProviderId::Local
        ]
    );
}

#[test]
fn load_rules_rejects_a_file_missing_a_job_type() {
    let dir = Temp(temp_dir());
    let path = dir.0.join("rules.json");
    let body = serde_json::json!({
        "rules": { "planning": ["claude"] },
        "fallbackFit": 0.5,
        "allowance": { "claude": { "windowHours": 5, "units": 15 } },
        "defaultRestHours": 5
    });
    fs::write(&path, body.to_string()).unwrap();
    let err = load_rules(Some(&path)).unwrap_err();
    assert!(err.to_string().contains("debugging"), "{err}");
}

#[test]
fn load_rules_pace_block() {
    let dir = Temp(temp_dir());
    let path = dir.0.join("rules.json");
    let bundled = serde_json::to_value(rules()).unwrap();
    let mut without = bundled.clone();
    without.as_object_mut().unwrap().remove("pace");
    fs::write(&path, without.to_string()).unwrap();
    let loaded = load_rules(Some(&path)).unwrap();
    assert_eq!(loaded.pace, None);
    assert_eq!(loaded.fallback_fit, 0.5);

    let mut boundary = bundled.clone();
    boundary["pace"] = serde_json::json!({
        "weight": 0, "min": 1, "max": 1, "atRiskSlack": 0, "atRiskLeft": 1
    });
    fs::write(&path, boundary.to_string()).unwrap();
    assert_eq!(
        load_rules(Some(&path)).unwrap().pace,
        Some(PaceRules {
            weight: 0.0,
            min: 1.0,
            max: 1.0,
            at_risk_slack: 0.0,
            at_risk_left: 1.0
        })
    );

    let bad = [
        serde_json::json!({"weight": -1, "min": 0.5, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
        serde_json::json!({"weight": 1.5, "min": 0, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
        serde_json::json!({"weight": 1.5, "min": 1.1, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
        serde_json::json!({"weight": 1.5, "min": 0.5, "max": 0.9, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
        serde_json::json!({"weight": null, "min": 0.5, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
        serde_json::json!({"weight": 1.5, "min": 0.5, "max": 2, "atRiskSlack": 1.1, "atRiskLeft": 0.15}),
        serde_json::json!({"weight": 1.5, "min": 0.5, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": -0.01}),
        serde_json::json!({"min": 0.5, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
        Value::Null,
        Value::String("fast".into()),
    ];
    for pace in bad {
        let mut body = bundled.clone();
        body["pace"] = pace;
        fs::write(&path, body.to_string()).unwrap();
        assert_eq!(
            load_rules(Some(&path)).unwrap_err().to_string(),
            "router rules: invalid pace settings"
        );
    }
}

#[test]
fn headroom_uses_windows_else_weight_and_rest_is_zero() {
    let rules = rules();
    let store = memory();
    insert(&store, ProviderId::Claude, NOW - 1, Some(0.0), Some(100));
    insert(&store, ProviderId::Claude, NOW - 1, None, Some(5));
    insert(&store, ProviderId::Claude, NOW - 1, None, None);
    assert_eq!(
        headroom(ProviderId::Claude, &store, &rules, NOW).unwrap(),
        1.0 - 6.0 / 15.0
    );
    agree(&store, &rules, NOW);

    let store = memory();
    let window_ms = (rules.allowance[&ProviderId::Claude].window_hours * 3_600_000.0) as i64;
    insert(
        &store,
        ProviderId::Claude,
        NOW - window_ms - 1,
        Some(15.0),
        None,
    );
    insert(&store, ProviderId::Claude, NOW - window_ms, Some(3.0), None);
    assert_eq!(
        headroom(ProviderId::Claude, &store, &rules, NOW).unwrap(),
        1.0 - 3.0 / 15.0
    );

    save(
        &store,
        ProviderId::Claude,
        &[window("reported", 0.0, NOW + 1)],
        NOW,
    );
    assert_eq!(
        headroom(ProviderId::Claude, &store, &rules, NOW).unwrap(),
        1.0
    );
    save(
        &store,
        ProviderId::Cursor,
        &[window("reported", 0.75, NOW + 1)],
        NOW,
    );
    assert_eq!(
        headroom(ProviderId::Cursor, &store, &rules, NOW).unwrap(),
        0.25
    );
    store.set_resting("cursor", NOW + 1).unwrap();
    assert_eq!(
        headroom(ProviderId::Cursor, &store, &rules, NOW).unwrap(),
        0.0
    );
    store.set_resting("cursor", NOW).unwrap();
    assert_eq!(
        headroom(ProviderId::Cursor, &store, &rules, NOW).unwrap(),
        0.25
    );
    agree(&store, &rules, NOW);
}

#[test]
fn local_without_allowance_is_unlimited_headroom() {
    let rules = rules();
    assert!(!rules.allowance.contains_key(&ProviderId::Local));
    let store = memory();
    assert_eq!(
        headroom(ProviderId::Local, &store, &rules, NOW).unwrap(),
        1.0
    );
    insert(&store, ProviderId::Local, NOW - 1, Some(1_000_000.0), None);
    assert_eq!(
        headroom(ProviderId::Local, &store, &rules, NOW).unwrap(),
        1.0
    );
    save(
        &store,
        ProviderId::Local,
        &[window("reported", 0.4, NOW + 1)],
        NOW,
    );
    assert_eq!(
        headroom(ProviderId::Local, &store, &rules, NOW).unwrap(),
        0.6
    );
    store.set_resting("local", NOW + 1).unwrap();
    assert_eq!(
        headroom(ProviderId::Local, &store, &rules, NOW).unwrap(),
        0.0
    );
    agree(&store, &rules, NOW);
}

#[test]
fn grok_shares_cursor_headroom_rest_and_parallel_limit() {
    let rules = rules();
    let mut limited = rules.clone();
    limited.max_parallel = 1;
    let store = memory();
    assert_eq!(
        headroom(ProviderId::Grok, &store, &rules, NOW).unwrap(),
        headroom(ProviderId::Cursor, &store, &rules, NOW).unwrap()
    );
    insert(&store, ProviderId::Cursor, NOW - 1, Some(5_000_000.0), None);
    insert(&store, ProviderId::Grok, NOW - 1, Some(5_000_000.0), None);
    let shared = 1.0 - 10_000_000.0 / 20_000_000.0;
    assert_eq!(
        headroom(ProviderId::Cursor, &store, &rules, NOW).unwrap(),
        shared
    );
    assert_eq!(
        headroom(ProviderId::Grok, &store, &rules, NOW).unwrap(),
        shared
    );
    save(
        &store,
        ProviderId::Cursor,
        &[window("reported", 0.25, NOW + 1)],
        NOW,
    );
    assert_eq!(
        headroom(ProviderId::Grok, &store, &rules, NOW).unwrap(),
        0.75
    );
    store.set_resting("cursor", NOW + 1).unwrap();
    assert_eq!(
        headroom(ProviderId::Grok, &store, &rules, NOW).unwrap(),
        0.0
    );
    assert_eq!(
        headroom(ProviderId::Cursor, &store, &rules, NOW).unwrap(),
        0.0
    );
    store.clear_resting("cursor").unwrap();

    let both = [ProviderId::Claude, ProviderId::Cursor, ProviderId::Grok];
    let mut cursor_busy = RunningCounts::default();
    cursor_busy.set(ProviderId::Cursor, 1);
    assert_eq!(
        pick_provider(
            JobType::Debugging,
            &both,
            &store,
            &limited,
            NOW,
            &[],
            &cursor_busy
        )
        .unwrap(),
        Some(ProviderId::Claude)
    );
    let mut grok_busy = RunningCounts::default();
    grok_busy.set(ProviderId::Grok, 1);
    assert_eq!(
        pick_provider(
            JobType::Debugging,
            &both,
            &store,
            &limited,
            NOW,
            &[],
            &grok_busy
        )
        .unwrap(),
        Some(ProviderId::Claude)
    );
    let mut state = RouterState::hydrate(&store, &limited, NOW).unwrap();
    assert_eq!(
        state.pick_provider(JobType::Debugging, &both, NOW, &[], &grok_busy),
        Some(ProviderId::Claude)
    );
    agree(&store, &rules, NOW);
}

#[test]
fn pick_prefers_first_choice_then_fit_times_headroom() {
    let rules = rules();
    let both = [ProviderId::Claude, ProviderId::Cursor];
    let none = RunningCounts::default();
    let store = memory();
    assert_eq!(
        pick_provider(JobType::Planning, &both, &store, &rules, NOW, &[], &none).unwrap(),
        Some(ProviderId::Claude)
    );
    assert_eq!(
        pick_provider(JobType::Boilerplate, &both, &store, &rules, NOW, &[], &none).unwrap(),
        Some(ProviderId::Cursor)
    );
    save(
        &store,
        ProviderId::Claude,
        &[window("reported", 0.75, NOW + 1)],
        NOW,
    );
    assert_eq!(
        pick_provider(JobType::Planning, &both, &store, &rules, NOW, &[], &none).unwrap(),
        Some(ProviderId::Cursor)
    );
    save(
        &store,
        ProviderId::Claude,
        &[window("reported", 0.5, NOW + 1)],
        NOW,
    );
    assert_eq!(
        pick_provider(JobType::Planning, &both, &store, &rules, NOW, &[], &none).unwrap(),
        Some(ProviderId::Claude)
    );
    store.set_resting("claude", NOW + 5_000).unwrap();
    assert_eq!(
        pick_provider(JobType::Planning, &both, &store, &rules, NOW, &[], &none).unwrap(),
        Some(ProviderId::Cursor)
    );
    store.set_resting("cursor", NOW + 5_000).unwrap();
    assert_eq!(
        pick_provider(JobType::Planning, &both, &store, &rules, NOW, &[], &none).unwrap(),
        None
    );
    store.clear_resting("claude").unwrap();
    store.clear_resting("cursor").unwrap();
    assert_eq!(
        pick_provider(
            JobType::Planning,
            &both,
            &store,
            &rules,
            NOW,
            &[ProviderId::Claude],
            &none
        )
        .unwrap(),
        Some(ProviderId::Cursor)
    );
    assert_eq!(
        pick_provider(
            JobType::Planning,
            &both,
            &store,
            &rules,
            NOW,
            &[ProviderId::Claude, ProviderId::Cursor],
            &none
        )
        .unwrap(),
        None
    );
    let scores = score_providers(
        JobType::Planning,
        &both,
        &store,
        &rules,
        NOW,
        &[ProviderId::Cursor],
    )
    .unwrap();
    assert_eq!(
        scores.iter().map(|row| row.provider).collect::<Vec<_>>(),
        vec![ProviderId::Claude]
    );
    save(
        &store,
        ProviderId::Claude,
        &[window("reported", 1.0, NOW + 1)],
        NOW,
    );
    save(
        &store,
        ProviderId::Cursor,
        &[window("reported", 1.0, NOW + 1)],
        NOW,
    );
    assert_eq!(
        pick_provider(JobType::Planning, &both, &store, &rules, NOW, &[], &none).unwrap(),
        None
    );
    agree(&store, &rules, NOW);
}

#[test]
fn plan_usage_estimates_weight_when_no_windows_are_stored() {
    let rules = rules();
    let store = memory();
    insert(
        &store,
        ProviderId::Claude,
        PACE_NOW - 1,
        Some(0.0),
        Some(100),
    );
    insert(&store, ProviderId::Claude, PACE_NOW - 1, None, Some(5));
    insert(&store, ProviderId::Claude, PACE_NOW - 1, None, None);
    let usage = plan_usage(ProviderId::Claude, &store, &rules, PACE_NOW).unwrap();
    assert!(usage.windows.is_empty());
    assert_eq!(usage.used, 6.0 / 15.0);
    assert_eq!(usage.resets_at, None);
    assert_eq!(usage.slack, None);
    assert_eq!(usage.pace_factor, 1.0);
    assert!(!usage.at_risk);
    agree(&store, &rules, PACE_NOW);
}

#[test]
fn plan_usage_reads_windows_pace_rollover_and_custom_pace() {
    let rules = rules();
    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[
            window("five_hour", 0.2, PACE_NOW + HOUR5_MS / 2),
            window("seven_day", 0.45, PACE_NOW + (WEEK_MS * 2) / 5),
        ],
        PACE_NOW,
    );
    let paced = plan_usage(ProviderId::Claude, &store, &rules, PACE_NOW).unwrap();
    assert_eq!(paced.used, 0.45);
    assert_eq!(paced.resets_at, Some(PACE_NOW + (WEEK_MS * 2) / 5));
    near(paced.slack.unwrap(), 0.15);
    near(paced.pace_factor, 1.225);
    assert!(!paced.at_risk);
    assert_eq!(
        paced
            .windows
            .iter()
            .map(|window| window.name.as_str())
            .collect::<Vec<_>>(),
        ["five_hour", "seven_day"]
    );
    agree(&store, &rules, PACE_NOW);

    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[window("seven_day", 0.8, PACE_NOW)],
        PACE_NOW,
    );
    let rolled = plan_usage(ProviderId::Claude, &store, &rules, PACE_NOW).unwrap();
    assert_eq!(rolled.used, 0.0);
    assert_eq!(rolled.windows.len(), 1);
    assert_eq!(rolled.windows[0].name, "seven_day");
    assert_eq!(rolled.windows[0].used, 0.0);
    assert_eq!(rolled.windows[0].resets_at, None);
    assert_eq!(rolled.windows[0].length_ms, Some(WEEK_MS));
    assert_eq!(rolled.slack, None);
    assert_eq!(rolled.pace_factor, 1.0);
    assert!(!rolled.at_risk);

    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[window("seven_day", 0.9, PACE_NOW + WEEK_MS / 2)],
        PACE_NOW,
    );
    assert_eq!(
        plan_usage(ProviderId::Claude, &store, &rules, PACE_NOW)
            .unwrap()
            .pace_factor,
        0.5
    );

    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[window("seven_day", 0.5, PACE_NOW + WEEK_MS / 10)],
        PACE_NOW,
    );
    assert!(
        plan_usage(ProviderId::Claude, &store, &rules, PACE_NOW)
            .unwrap()
            .at_risk
    );

    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[
            window("five_hour", 0.9375, PACE_NOW + HOUR5_MS / 2),
            window("seven_day", 0.5, PACE_NOW + WEEK_MS / 10),
        ],
        PACE_NOW,
    );
    let tight = plan_usage(ProviderId::Claude, &store, &rules, PACE_NOW).unwrap();
    assert_eq!(tight.used, 0.9375);
    near(tight.slack.unwrap(), 0.4);
    assert!(!tight.at_risk);

    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[
            window("session", 0.8, PACE_NOW + 1000),
            window("seven_day", 0.25, PACE_NOW + WEEK_MS / 2),
        ],
        PACE_NOW,
    );
    let mixed = plan_usage(ProviderId::Claude, &store, &rules, PACE_NOW).unwrap();
    assert_eq!(mixed.used, 0.8);
    assert_eq!(
        mixed
            .windows
            .iter()
            .find(|window| window.name == "session")
            .unwrap()
            .length_ms,
        None
    );
    near(mixed.slack.unwrap(), 0.25);
    near(mixed.pace_factor, 1.375);
    assert!(!mixed.at_risk);

    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[window("seven_day", 0.5, PACE_NOW + WEEK_MS / 10)],
        PACE_NOW,
    );
    let mut custom = rules.clone();
    custom.pace = Some(PaceRules {
        weight: 4.0,
        min: 0.2,
        max: 1.1,
        at_risk_slack: 0.99,
        at_risk_left: 0.05,
    });
    let usage = plan_usage(ProviderId::Claude, &store, &custom, PACE_NOW).unwrap();
    assert_eq!(usage.pace_factor, 1.1);
    assert!(!usage.at_risk);
    agree(&store, &custom, PACE_NOW);
}

#[test]
fn pace_changes_who_wins_and_why() {
    let rules = rules();
    let both = [ProviderId::Claude, ProviderId::Cursor];
    let none = RunningCounts::default();
    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[window("seven_day", 0.0, PACE_NOW + WEEK_MS / 2)],
        PACE_NOW,
    );
    let pick = pick_with_reason(
        JobType::Planning,
        &both,
        &store,
        &rules,
        PACE_NOW,
        &[],
        &none,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pick.provider, ProviderId::Claude);
    assert_eq!(pick.reason, "first choice");
    let scores = score_providers(JobType::Planning, &both, &store, &rules, PACE_NOW, &[]).unwrap();
    assert_eq!(scores[0].pace_factor, 1.75);
    assert_eq!(scores[0].score, 1.75);
    assert_eq!(scores[1].score, 0.5);
    agree(&store, &rules, PACE_NOW);

    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[window("reported", 0.5, PACE_NOW + 1000)],
        PACE_NOW,
    );
    save(
        &store,
        ProviderId::Cursor,
        &[window("seven_day", 0.0, PACE_NOW + WEEK_MS / 2)],
        PACE_NOW,
    );
    let pick = pick_with_reason(
        JobType::Planning,
        &both,
        &store,
        &rules,
        PACE_NOW,
        &[],
        &none,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pick.provider, ProviderId::Cursor);
    assert_eq!(pick.reason, "behind pace");
    let scores = score_providers(JobType::Planning, &both, &store, &rules, PACE_NOW, &[]).unwrap();
    assert_eq!(scores[1].score, 0.875);

    let store = memory();
    save(
        &store,
        ProviderId::Cursor,
        &[window("seven_day", 0.25, PACE_NOW + WEEK_MS / 8)],
        PACE_NOW,
    );
    let pick = pick_with_reason(
        JobType::Planning,
        &both,
        &store,
        &rules,
        PACE_NOW,
        &[],
        &none,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pick.provider, ProviderId::Cursor);
    assert_eq!(pick.reason, "allowance expiring");
    let scores = score_providers(JobType::Planning, &both, &store, &rules, PACE_NOW, &[]).unwrap();
    assert!(scores[1].at_risk);
    assert_eq!(scores[1].pace_factor, 2.0);
    assert_eq!(scores[1].score, 1.5);

    let store = memory();
    let pick = pick_with_reason(
        JobType::Planning,
        &[ProviderId::Claude],
        &store,
        &rules,
        PACE_NOW,
        &[],
        &none,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pick.reason, "only plan available");
    store.set_resting("cursor", PACE_NOW + 1).unwrap();
    let pick = pick_with_reason(
        JobType::Planning,
        &both,
        &store,
        &rules,
        PACE_NOW,
        &[],
        &none,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pick.provider, ProviderId::Claude);
    assert_eq!(pick.reason, "only plan available");

    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[window("reported", 0.75, PACE_NOW + 1)],
        PACE_NOW,
    );
    let pick = pick_with_reason(
        JobType::Planning,
        &both,
        &store,
        &rules,
        PACE_NOW,
        &[],
        &none,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pick.provider, ProviderId::Cursor);
    assert_eq!(pick.reason, "more headroom");
    agree(&store, &rules, PACE_NOW);
}

#[test]
fn dashboard_reading_counts_plus_later_runs() {
    let rules = rules();
    let units = rules.allowance[&ProviderId::Cursor].units;
    let read = local_ms(2026, 10, 21, 12);
    let resets = local_ms(2026, 11, 5, 0);
    let store = memory();
    insert(
        &store,
        ProviderId::Cursor,
        read - 1,
        None,
        Some((units / 2.0) as i64),
    );
    save(
        &store,
        ProviderId::Cursor,
        &[window(DASHBOARD_WINDOW, 0.4, resets)],
        read,
    );
    near(
        plan_usage(ProviderId::Cursor, &store, &rules, read + 1)
            .unwrap()
            .used,
        0.4,
    );
    insert(
        &store,
        ProviderId::Grok,
        read + 10,
        None,
        Some((units / 10.0) as i64),
    );
    insert(
        &store,
        ProviderId::Cursor,
        read + 20,
        None,
        Some((units / 10.0) as i64),
    );
    let usage = plan_usage(ProviderId::Cursor, &store, &rules, read + 30).unwrap();
    near(usage.used, 0.6);
    assert_eq!(usage.resets_at, Some(resets));
    assert_eq!(usage.windows[0].name, DASHBOARD_WINDOW);
    near(
        headroom(ProviderId::Cursor, &store, &rules, read + 30).unwrap(),
        0.4,
    );
    agree(&store, &rules, read + 30);
}

#[test]
fn after_reset_dashboard_counts_only_the_new_cycle() {
    let rules = rules();
    let units = rules.allowance[&ProviderId::Cursor].units;
    let read = local_ms(2026, 10, 21, 12);
    let resets = local_ms(2026, 11, 5, 0);
    let store = memory();
    save(
        &store,
        ProviderId::Cursor,
        &[window(DASHBOARD_WINDOW, 0.9, resets)],
        read,
    );
    insert(
        &store,
        ProviderId::Cursor,
        read + 10,
        None,
        Some((units / 10.0) as i64),
    );
    insert(
        &store,
        ProviderId::Cursor,
        resets + 10,
        None,
        Some((units / 20.0) as i64),
    );
    let usage = plan_usage(ProviderId::Cursor, &store, &rules, resets + 60_000).unwrap();
    near(usage.used, 0.05);
    assert_eq!(usage.resets_at, Some(local_ms(2026, 12, 5, 0)));
    let later = plan_usage(ProviderId::Cursor, &store, &rules, local_ms(2027, 1, 6, 0)).unwrap();
    assert_eq!(later.used, 0.0);
    assert_eq!(later.resets_at, Some(local_ms(2027, 2, 5, 0)));
    agree(&store, &rules, local_ms(2027, 1, 6, 0));
}

#[test]
fn add_months_keeps_the_anchor_day_through_short_months() {
    let jan31 = local_ms(2027, 1, 31, 9);
    assert_eq!(add_months(jan31, 1), local_ms(2027, 2, 28, 9));
    assert_eq!(add_months(jan31, 2), local_ms(2027, 3, 31, 9));
    assert_eq!(
        add_months(local_ms(2026, 12, 15, 0), 1),
        local_ms(2027, 1, 15, 0)
    );
}

#[test]
fn dashboard_reading_checks_the_percentage_and_the_reset_date() {
    let now = local_ms(2026, 10, 21, 0);
    let next = local_ms(2026, 11, 5, 0);
    let reading = dashboard_reading(Some(37.5), Some(next), now).unwrap();
    assert_eq!(reading.name, DASHBOARD_WINDOW);
    assert_eq!(reading.utilization, 0.375);
    assert_eq!(reading.resets_at, next);
    assert_eq!(
        dashboard_reading(Some(0.0), Some(next), now)
            .unwrap()
            .utilization,
        0.0
    );
    assert_eq!(
        dashboard_reading(Some(100.0), Some(next), now)
            .unwrap()
            .utilization,
        1.0
    );
    assert_eq!(
        dashboard_reading_value(&Value::String("40".into()), &Value::from(next), now).unwrap_err(),
        "Enter how much is used, as a percentage."
    );
    assert_eq!(
        dashboard_reading(Some(f64::NAN), Some(next), now).unwrap_err(),
        "Enter how much is used, as a percentage."
    );
    assert_eq!(
        dashboard_reading(Some(101.0), Some(next), now).unwrap_err(),
        "Usage must be between 0 and 100%."
    );
    assert_eq!(
        dashboard_reading(Some(-1.0), Some(next), now).unwrap_err(),
        "Usage must be between 0 and 100%."
    );
    assert_eq!(
        dashboard_reading(Some(40.0), None, now).unwrap_err(),
        "Enter the date your usage resets."
    );
    assert_eq!(
        dashboard_reading(Some(40.0), Some(now), now).unwrap_err(),
        "The reset date has to be in the future."
    );
}

#[test]
fn delete_window_removes_one_window_and_leaves_the_rest() {
    // Window deletion is store behavior. The router reads whatever rows remain.
    let store = memory();
    save(
        &store,
        ProviderId::Claude,
        &[
            window("five_hour", 0.2, NOW + 1),
            window(DASHBOARD_WINDOW, 0.3, NOW + 1),
        ],
        NOW,
    );
    store.delete_window("claude", DASHBOARD_WINDOW).unwrap();
    let names: Vec<_> = store
        .windows("claude")
        .unwrap()
        .into_iter()
        .map(|window| window.name)
        .collect();
    assert_eq!(names, ["five_hour"]);
}
