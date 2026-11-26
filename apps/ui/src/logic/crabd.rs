//! The Crabd theme: a window shaped like a pixel crab. Its four legs are usage meters, each
//! set to one plan's window (Claude's 5-hour, Codex's weekly, Cursor's month, ...).

use legate_core::{PlanStatus, ProviderId};

use crate::logic::usage_format::window_label;

pub const THEME: &str = "crabd";
pub const LEGS_KEY: &str = "orch.crabdLegs";
pub const LEG_COUNT: usize = 4;

/// Where each leg hangs, as a fraction of the body's width from its left edge, and how wide a
/// leg is: the crab's own proportions.
pub const LEG_LEFTS: [f64; LEG_COUNT] = [0.084, 0.251, 0.671, 0.838];
pub const LEG_WIDTH: f64 = 0.084;

/// One leg's choice: `<plan>:<window name>`, such as `claude:five_hour`.
pub fn leg_key(plan: ProviderId, window: &str) -> String {
    format!("{}:{window}", plan.as_str())
}

pub fn plan_name(plan: ProviderId) -> &'static str {
    match plan {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Cursor => "Cursor",
        ProviderId::Grok => "Grok",
        ProviderId::Local => "Local",
    }
}

/// The short window name a leg has room for: "5h", "7d", "30d".
pub fn short_window(name: &str) -> String {
    match name {
        "five_hour" => "5h".into(),
        "seven_day" => "7d".into(),
        "dashboard" => "30d".into(),
        _ if name.starts_with("seven_day_") => "7d".into(),
        _ => {
            let mut parts = name.splitn(2, '_');
            match (parts.next(), parts.next()) {
                (Some(count), Some(unit))
                    if !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()) =>
                {
                    format!("{count}{}", unit.chars().next().unwrap_or(' '))
                }
                _ => name.chars().take(3).collect(),
            }
        }
    }
}

/// Every window a leg can show, from the plans Legate reads usage for: (key, "Claude · 5-hour").
pub fn leg_options(plans: &[PlanStatus]) -> Vec<(String, String)> {
    plans
        .iter()
        .flat_map(|plan| {
            plan.windows.iter().map(move |window| {
                (
                    leg_key(plan.id, &window.name),
                    format!("{} · {}", plan_name(plan.id), window_label(&window.name)),
                )
            })
        })
        .collect()
}

/// The saved choices, or the first windows on offer for legs never set. A saved choice whose
/// window is not on offer right now (a plan signed out) is kept, so it comes back.
pub fn legs(saved: Option<&str>, options: &[(String, String)]) -> [Option<String>; LEG_COUNT] {
    let stored: Vec<Option<String>> = saved
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let mut unused = options
        .iter()
        .map(|(key, _)| key.clone())
        .filter(|key| !stored.iter().flatten().any(|taken| taken == key));
    std::array::from_fn(|index| match stored.get(index) {
        Some(choice) => choice.clone(),
        None => unused.next(),
    })
}

pub fn save_legs(legs: &[Option<String>; LEG_COUNT]) -> String {
    serde_json::to_string(legs).unwrap_or_else(|_| "[]".into())
}

/// What a leg shows: its short window name, the plan's name, and the fraction used (0..1).
#[derive(Debug, Clone, PartialEq)]
pub struct LegReading {
    pub plan: &'static str,
    pub short: String,
    pub title: String,
    pub used: f64,
}

pub fn reading(plans: &[PlanStatus], key: &str) -> Option<LegReading> {
    let (plan, window) = key.split_once(':')?;
    let plan = plans.iter().find(|item| item.id.as_str() == plan)?;
    let found = plan.windows.iter().find(|item| item.name == window)?;
    Some(LegReading {
        plan: plan_name(plan.id),
        short: short_window(window),
        title: format!(
            "{} · {}: {}% used",
            plan_name(plan.id),
            window_label(window),
            (found.used.clamp(0.0, 1.0) * 100.0).round()
        ),
        used: found.used.clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use legate_core::{PlanWindow, VersionStatus};

    fn plan(id: ProviderId, windows: &[(&str, f64)]) -> PlanStatus {
        PlanStatus {
            id,
            available: true,
            problem: None,
            used: 0.0,
            resting_until: None,
            resets_at: None,
            at_risk: false,
            busy: false,
            running: 0,
            queued: 0,
            model: None,
            windows: windows
                .iter()
                .map(|(name, used)| PlanWindow {
                    name: (*name).into(),
                    used: *used,
                    resets_at: None,
                })
                .collect(),
            usage_at: None,
            version: None,
            version_status: VersionStatus::Unknown,
            tested_version: String::new(),
        }
    }

    fn plans() -> Vec<PlanStatus> {
        vec![
            plan(
                ProviderId::Claude,
                &[("five_hour", 0.58), ("seven_day", 0.26)],
            ),
            plan(ProviderId::Cursor, &[("dashboard", 0.45)]),
            plan(
                ProviderId::Codex,
                &[("five_hour", 0.15), ("seven_day", 1.4)],
            ),
        ]
    }

    #[test]
    fn every_plan_window_is_a_choice() {
        let options = leg_options(&plans());
        assert_eq!(options.len(), 5);
        assert_eq!(
            options[0],
            ("claude:five_hour".into(), "Claude · 5-hour".into())
        );
        assert_eq!(
            options[2],
            ("cursor:dashboard".into(), "Cursor · This month".into())
        );
    }

    #[test]
    fn unset_legs_take_the_first_windows_not_already_shown() {
        let options = leg_options(&plans());
        let fresh = legs(None, &options);
        assert_eq!(fresh[0].as_deref(), Some("claude:five_hour"));
        assert_eq!(fresh[3].as_deref(), Some("codex:five_hour"));
        let saved = r#"["codex:seven_day", null]"#;
        let picked = legs(Some(saved), &options);
        assert_eq!(picked[0].as_deref(), Some("codex:seven_day"));
        assert_eq!(picked[1], None);
        assert_eq!(picked[2].as_deref(), Some("claude:five_hour"));
        assert_eq!(picked[3].as_deref(), Some("claude:seven_day"));
        assert_eq!(legs(Some(&save_legs(&picked)), &options), picked);
    }

    #[test]
    fn a_leg_reads_its_window_and_caps_at_full() {
        let plans = plans();
        let five = reading(&plans, "claude:five_hour").unwrap();
        assert_eq!(five.short, "5h");
        assert_eq!(five.plan, "Claude");
        assert_eq!(five.title, "Claude · 5-hour: 58% used");
        assert_eq!(reading(&plans, "cursor:dashboard").unwrap().short, "30d");
        assert_eq!(reading(&plans, "codex:seven_day").unwrap().used, 1.0);
        assert_eq!(reading(&plans, "grok:five_hour"), None);
        assert_eq!(reading(&plans, "nonsense"), None);
    }

    #[test]
    fn short_names_fit_a_leg() {
        assert_eq!(short_window("seven_day_opus"), "7d");
        assert_eq!(short_window("12_hour"), "12h");
        assert_eq!(short_window("monthly"), "mon");
    }
}
