//! When a goal starts by itself: a time of day, every few hours, or a GitHub watch.

use chrono::{Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::LeadError;
use crate::text::fresh_id;

pub const TRIGGER_KINDS: [&str; 5] = ["daily", "hourly", "ci-failed", "new-issue", "main-broken"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriggerKind {
    Daily,
    Hourly,
    CiFailed,
    NewIssue,
    MainBroken,
}

impl TriggerKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Hourly => "hourly",
            Self::CiFailed => "ci-failed",
            Self::NewIssue => "new-issue",
            Self::MainBroken => "main-broken",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "daily" => Some(Self::Daily),
            "hourly" => Some(Self::Hourly),
            "ci-failed" => Some(Self::CiFailed),
            "new-issue" => Some(Self::NewIssue),
            "main-broken" => Some(Self::MainBroken),
            _ => None,
        }
    }
}

pub fn is_trigger_kind(value: &str) -> bool {
    TriggerKind::parse(value).is_some()
}

pub fn is_scheduled(kind: TriggerKind) -> bool {
    matches!(kind, TriggerKind::Daily | TriggerKind::Hourly)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    pub id: String,
    pub project: String,
    pub kind: TriggerKind,
    pub at: Option<String>,
    pub hours: Option<i64>,
    pub text: String,
    pub check: Option<String>,
    pub max_rounds: i64,
    pub enabled: bool,
    pub last_fired: Option<i64>,
    pub seen: Vec<String>,
    pub primed: bool,
    pub last_error: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullFact {
    pub number: i64,
    pub title: String,
    pub branch: String,
    pub sha: String,
    pub failing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFact {
    pub number: i64,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFact {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub conclusion: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GithubFacts {
    pub branch: Option<String>,
    pub default_branch: Option<String>,
    pub pulls: Vec<PullFact>,
    pub issues: Vec<IssueFact>,
    pub main_runs: Vec<RunFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Firing {
    pub key: String,
    pub text: String,
    pub run_id: Option<i64>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewTrigger {
    pub project: String,
    pub kind: TriggerKind,
    pub at: Option<String>,
    pub hours: Option<f64>,
    pub text: Option<String>,
    pub check: Option<String>,
    pub max_rounds: Option<f64>,
    pub now: i64,
}

/// When a scheduled trigger fires next, in epoch ms. `None` for a GitHub watch or a bad clock time.
pub fn next_due(trigger: &Trigger, now: i64) -> Option<i64> {
    if trigger.kind == TriggerKind::Hourly {
        let hours = trigger.hours.unwrap_or(24).max(1);
        let from = trigger.last_fired.unwrap_or(trigger.created_at);
        return Some(from + hours * 3_600_000);
    }
    if trigger.kind != TriggerKind::Daily {
        return None;
    }
    let (hour, minute) = clock(trigger.at.as_deref().unwrap_or(""))?;
    if hour > 23 || minute > 59 {
        return None;
    }
    let now_dt = Local.timestamp_millis_opt(now).single()?;
    let mut due = Local
        .with_ymd_and_hms(now_dt.year(), now_dt.month(), now_dt.day(), hour, minute, 0)
        .single()?;
    let due_ms = due.timestamp_millis();
    let roll = match trigger.last_fired {
        Some(last) => last >= due_ms,
        None => trigger.created_at > due_ms,
    };
    if roll {
        let next = due.date_naive() + chrono::Days::new(1);
        due = Local
            .with_ymd_and_hms(next.year(), next.month(), next.day(), hour, minute, 0)
            .single()?;
    }
    Some(due.timestamp_millis())
}

pub fn is_due(trigger: &Trigger, now: i64) -> bool {
    if !trigger.enabled || !is_scheduled(trigger.kind) {
        return false;
    }
    next_due(trigger, now).is_some_and(|due| now >= due)
}

pub fn github_firings(trigger: &Trigger, facts: &GithubFacts) -> Vec<Firing> {
    let extra = if trigger.text.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nAlso: {}", trigger.text.trim())
    };
    match trigger.kind {
        TriggerKind::CiFailed => {
            let Some(branch) = &facts.branch else {
                return Vec::new();
            };
            facts
                .pulls
                .iter()
                .filter(|pull| pull.failing && &pull.branch == branch)
                .map(|pull| Firing {
                    key: format!("pr:{}:{}", pull.number, pull.sha),
                    text: format!(
                        "CI fails on pull request #{} \"{}\" (branch {}). Find why and make the failing checks pass. Do not weaken or skip tests.{extra}",
                        pull.number, pull.title, pull.branch
                    ),
                    run_id: None,
                    branch: Some(pull.branch.clone()),
                })
                .collect()
        }
        TriggerKind::NewIssue => facts
            .issues
            .iter()
            .map(|issue| {
                let body: String = issue.body.trim().chars().take(4000).collect();
                Firing {
                    key: format!("issue:{}", issue.number),
                    text: format!(
                        "Resolve GitHub issue #{}: {}\n\n{body}{extra}",
                        issue.number, issue.title
                    ),
                    run_id: None,
                    branch: None,
                }
            })
            .collect(),
        TriggerKind::MainBroken => {
            let Some(default_branch) = &facts.default_branch else {
                return Vec::new();
            };
            if facts.branch.as_deref() != Some(default_branch.as_str()) {
                return Vec::new();
            }
            let Some(newest) = facts.main_runs.iter().find(|run| run.status == "completed") else {
                return Vec::new();
            };
            if newest.conclusion != "failure" {
                return Vec::new();
            }
            vec![Firing {
                key: format!("run:{}", newest.id),
                text: format!(
                    "The \"{}\" run fails on {default_branch}. Find why and fix it. Do not weaken or skip tests.{extra}",
                    newest.name
                ),
                run_id: Some(newest.id),
                branch: Some(default_branch.clone()),
            }]
        }
        TriggerKind::Daily | TriggerKind::Hourly => Vec::new(),
    }
}

pub fn fresh_firings(trigger: &Trigger, facts: &GithubFacts) -> (Vec<Firing>, Vec<String>) {
    let all = github_firings(trigger, facts);
    let mut seen = trigger.seen.clone();
    let fire = if trigger.primed {
        all.iter()
            .filter(|firing| !seen.iter().any(|key| key == &firing.key))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    for firing in &all {
        if !seen.iter().any(|key| key == &firing.key) {
            seen.push(firing.key.clone());
        }
    }
    if seen.len() > 300 {
        seen = seen.split_off(seen.len() - 300);
    }
    (fire, seen)
}

pub fn new_trigger(input: NewTrigger) -> Result<Trigger, LeadError> {
    let text = input.text.unwrap_or_default().trim().to_string();
    if is_scheduled(input.kind) && text.is_empty() {
        return Err(LeadError::message("Say what the scheduled goal is."));
    }
    if input.kind == TriggerKind::Daily && !is_clock(input.at.as_deref().unwrap_or("")) {
        return Err(LeadError::message("Give a time such as 07:30."));
    }
    let check = input.check.unwrap_or_default().trim().to_string();
    Ok(Trigger {
        id: fresh_id(),
        project: input.project,
        kind: input.kind,
        at: if input.kind == TriggerKind::Daily {
            input.at
        } else {
            None
        },
        hours: if input.kind == TriggerKind::Hourly {
            Some(clamp_round(input.hours.unwrap_or(24.0), 1, 168))
        } else {
            None
        },
        text,
        check: if check.is_empty() { None } else { Some(check) },
        max_rounds: clamp_round(input.max_rounds.unwrap_or(5.0), 1, 20),
        enabled: true,
        last_fired: None,
        seen: Vec::new(),
        primed: is_scheduled(input.kind),
        last_error: None,
        created_at: input.now,
    })
}

fn clamp_round(value: f64, min: i64, max: i64) -> i64 {
    if !value.is_finite() {
        return min;
    }
    (value.round() as i64).clamp(min, max)
}

fn is_clock(value: &str) -> bool {
    clock(value).is_some()
}

/// `HH:MM` with one or two hour digits. Does not check the hour is under 24; [`next_due`] does.
fn clock(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.split_once(':')?;
    if !(1..=2).contains(&hour.len()) || minute.len() != 2 {
        return None;
    }
    if !hour.bytes().all(|byte| byte.is_ascii_digit())
        || !minute.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some((hour.parse().ok()?, minute.parse().ok()?))
}

pub fn parse_pulls(json: &str) -> Vec<PullFact> {
    parse_array(json)
        .into_iter()
        .filter_map(|row| {
            let number = json_int(row.get("number")?)?;
            let branch = row.get("headRefName")?.as_str()?.to_string();
            let checks = row
                .get("statusCheckRollup")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let failing = checks.iter().any(|check| {
                let Some(object) = check.as_object() else {
                    return false;
                };
                let verdict = match object.get("conclusion") {
                    Some(Value::Null) | None => object.get("state").unwrap_or(&Value::Null),
                    Some(other) => other,
                };
                let text = js_string(verdict).to_uppercase();
                text == "FAILURE" || text == "ERROR" || text == "TIMED_OUT"
            });
            Some(PullFact {
                number,
                title: row
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                branch,
                sha: row
                    .get("headRefOid")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                failing,
            })
        })
        .collect()
}

pub fn parse_issues(json: &str) -> Vec<IssueFact> {
    parse_array(json)
        .into_iter()
        .filter_map(|row| {
            Some(IssueFact {
                number: json_int(row.get("number")?)?,
                title: row
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                body: row
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

pub fn parse_runs(json: &str) -> Vec<RunFact> {
    parse_array(json)
        .into_iter()
        .filter_map(|row| {
            Some(RunFact {
                id: json_int(row.get("databaseId")?)?,
                name: row
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("workflow")
                    .to_string(),
                status: row
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                conclusion: row
                    .get("conclusion")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

fn parse_array(json: &str) -> Vec<serde_json::Map<String, Value>> {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(rows) = value.as_array() else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| row.as_object().cloned())
        .collect()
}

fn json_int(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        let number = value.as_f64()?;
        if number.fract() == 0.0 && number >= i64::MIN as f64 && number <= i64::MAX as f64 {
            Some(number as i64)
        } else {
            None
        }
    })
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Array(_) | Value::Object(_) => "[object Object]".to_string(),
    }
}

/// A trigger row the store can keep. Bad JSON and unknown kinds are skipped.
pub fn parse_stored(raw: &str) -> Option<Trigger> {
    let trigger: Trigger = serde_json::from_str(raw).ok()?;
    TriggerKind::parse(trigger.kind.as_str())?;
    Some(trigger)
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};

    use super::{
        Firing, GithubFacts, IssueFact, NewTrigger, PullFact, RunFact, Trigger, TriggerKind,
        fresh_firings, github_firings, is_due, new_trigger, next_due, parse_issues, parse_pulls,
        parse_runs,
    };

    const H: i64 = 3_600_000;

    fn at(day: u32, hour: u32, minute: u32) -> i64 {
        Local
            .with_ymd_and_hms(2026, 10, day, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    fn make(kind: TriggerKind, now: i64) -> Trigger {
        new_trigger(NewTrigger {
            project: "/p".to_string(),
            kind,
            at: if kind == TriggerKind::Daily {
                Some("07:30".to_string())
            } else {
                None
            },
            hours: if kind == TriggerKind::Hourly {
                Some(6.0)
            } else {
                None
            },
            text: Some("x".to_string()),
            check: None,
            max_rounds: None,
            now,
        })
        .unwrap()
    }

    fn facts() -> GithubFacts {
        GithubFacts {
            branch: Some("feature".to_string()),
            default_branch: Some("main".to_string()),
            ..GithubFacts::default()
        }
    }

    #[test]
    fn a_daily_trigger_is_due_at_its_time_once_per_day() {
        let trigger = new_trigger(NewTrigger {
            project: "/p".into(),
            kind: TriggerKind::Daily,
            at: Some("07:30".into()),
            hours: None,
            text: Some("Update dependencies and run the tests".into()),
            check: None,
            max_rounds: None,
            now: at(17, 6, 0),
        })
        .unwrap();
        assert!(!is_due(&trigger, at(17, 7, 29)));
        assert!(is_due(&trigger, at(17, 7, 30)));
        let mut fired = trigger.clone();
        fired.last_fired = Some(at(17, 7, 31));
        assert!(!is_due(&fired, at(17, 23, 0)));
        assert!(is_due(&fired, at(18, 7, 30)));
    }

    #[test]
    fn a_daily_trigger_made_after_todays_time_waits_for_tomorrow() {
        let trigger = make(TriggerKind::Daily, at(17, 9, 0));
        assert!(!is_due(&trigger, at(17, 10, 0)));
        assert_eq!(next_due(&trigger, at(17, 10, 0)), Some(at(18, 7, 30)));
    }

    #[test]
    fn an_hourly_trigger_is_due_every_few_hours_a_disabled_one_never() {
        let trigger = new_trigger(NewTrigger {
            project: "/p".into(),
            kind: TriggerKind::Hourly,
            at: None,
            hours: Some(6.0),
            text: Some("x".into()),
            check: None,
            max_rounds: None,
            now: 0,
        })
        .unwrap();
        assert!(!is_due(&trigger, 5 * H));
        assert!(is_due(&trigger, 6 * H));
        let mut later = trigger.clone();
        later.last_fired = Some(6 * H);
        assert!(!is_due(&later, 11 * H));
        let mut off = trigger;
        off.enabled = false;
        assert!(!is_due(&off, 99 * H));
    }

    #[test]
    fn new_triggers_check_their_input() {
        let bad_time = new_trigger(NewTrigger {
            project: "/p".into(),
            kind: TriggerKind::Daily,
            at: Some("soon".into()),
            hours: None,
            text: Some("x".into()),
            check: None,
            max_rounds: None,
            now: 0,
        });
        assert!(bad_time.unwrap_err().to_string().contains("Give a time"));
        let blank = new_trigger(NewTrigger {
            project: "/p".into(),
            kind: TriggerKind::Hourly,
            at: None,
            hours: None,
            text: Some(" ".into()),
            check: None,
            max_rounds: None,
            now: 0,
        });
        assert!(blank.unwrap_err().to_string().contains("scheduled goal"));
        let github = new_trigger(NewTrigger {
            project: "/p".into(),
            kind: TriggerKind::NewIssue,
            at: None,
            hours: None,
            text: None,
            check: None,
            max_rounds: None,
            now: 0,
        })
        .unwrap();
        assert!(!github.primed);
        assert_eq!(github.text, "");
    }

    #[test]
    fn ci_failures_fire_only_for_the_checked_out_branch_once_per_commit() {
        let mut trigger = new_trigger(NewTrigger {
            project: "/p".into(),
            kind: TriggerKind::CiFailed,
            at: None,
            hours: None,
            text: None,
            check: None,
            max_rounds: None,
            now: 0,
        })
        .unwrap();
        trigger.primed = true;
        let pulls = vec![
            PullFact {
                number: 12,
                title: "Login".into(),
                branch: "feature".into(),
                sha: "aaa".into(),
                failing: true,
            },
            PullFact {
                number: 13,
                title: "Other".into(),
                branch: "other".into(),
                sha: "bbb".into(),
                failing: true,
            },
            PullFact {
                number: 14,
                title: "Green".into(),
                branch: "feature".into(),
                sha: "ccc".into(),
                failing: false,
            },
        ];
        let mut rows = facts();
        rows.pulls = pulls.clone();
        let (fire, seen) = fresh_firings(&trigger, &rows);
        assert_eq!(
            fire.iter()
                .map(|firing| firing.key.as_str())
                .collect::<Vec<_>>(),
            vec!["pr:12:aaa"]
        );
        assert!(fire[0].text.contains("#12 \"Login\""));
        trigger.seen = seen;
        let (again, _) = fresh_firings(&trigger, &rows);
        assert!(again.is_empty());
        rows.pulls = vec![PullFact {
            sha: "ddd".into(),
            ..pulls[0].clone()
        }];
        let (pushed, _) = fresh_firings(&trigger, &rows);
        assert_eq!(
            pushed
                .iter()
                .map(|firing| firing.key.as_str())
                .collect::<Vec<_>>(),
            vec!["pr:12:ddd"]
        );
    }

    #[test]
    fn the_first_look_at_issues_only_remembers_them() {
        let trigger = new_trigger(NewTrigger {
            project: "/p".into(),
            kind: TriggerKind::NewIssue,
            at: None,
            hours: None,
            text: Some("Keep it small".into()),
            check: None,
            max_rounds: None,
            now: 0,
        })
        .unwrap();
        let old = vec![IssueFact {
            number: 1,
            title: "Old".into(),
            body: String::new(),
        }];
        let mut rows = facts();
        rows.issues = old.clone();
        let (primed, seen) = fresh_firings(&trigger, &rows);
        assert!(primed.is_empty());
        let mut next_trigger = trigger;
        next_trigger.primed = true;
        next_trigger.seen = seen;
        rows.issues.push(IssueFact {
            number: 2,
            title: "Crash on save".into(),
            body: "Steps…".into(),
        });
        let (next, _) = fresh_firings(&next_trigger, &rows);
        assert_eq!(next.len(), 1);
        assert!(next[0].text.contains("issue #2: Crash on save"));
        assert!(next[0].text.contains("Also: Keep it small"));
        let _ = Firing {
            key: String::new(),
            text: String::new(),
            run_id: None,
            branch: None,
        };
    }

    #[test]
    fn main_broken_fires_for_a_failed_newest_run_on_the_default_branch() {
        let mut trigger = new_trigger(NewTrigger {
            project: "/p".into(),
            kind: TriggerKind::MainBroken,
            at: None,
            hours: None,
            text: None,
            check: None,
            max_rounds: None,
            now: 0,
        })
        .unwrap();
        trigger.primed = true;
        let main_runs = vec![
            RunFact {
                id: 9,
                name: "CI".into(),
                status: "completed".into(),
                conclusion: "failure".into(),
            },
            RunFact {
                id: 8,
                name: "CI".into(),
                status: "completed".into(),
                conclusion: "success".into(),
            },
        ];
        let mut on_main = facts();
        on_main.branch = Some("main".into());
        on_main.main_runs = main_runs.clone();
        let firing = &github_firings(&trigger, &on_main)[0];
        assert_eq!(firing.key, "run:9");
        assert_eq!(firing.run_id, Some(9));
        let mut other = facts();
        other.main_runs = main_runs.clone();
        assert!(github_firings(&trigger, &other).is_empty());
        on_main.main_runs = vec![main_runs[1].clone()];
        assert!(github_firings(&trigger, &on_main).is_empty());
    }

    #[test]
    fn gh_json_is_read_safely() {
        let pulls = parse_pulls(
            &serde_json::json!([{
                "number": 3,
                "title": "T",
                "headRefName": "b",
                "headRefOid": "s",
                "statusCheckRollup": [{ "conclusion": "SUCCESS" }, { "state": "FAILURE" }]
            }])
            .to_string(),
        );
        assert_eq!(
            pulls,
            vec![PullFact {
                number: 3,
                title: "T".into(),
                branch: "b".into(),
                sha: "s".into(),
                failing: true
            }]
        );
        assert_eq!(
            parse_issues(r#"[{"number":4,"title":"I"}]"#),
            vec![IssueFact {
                number: 4,
                title: "I".into(),
                body: String::new()
            }]
        );
        assert_eq!(
            parse_runs(
                r#"[{"databaseId":5,"name":"CI","status":"completed","conclusion":"failure"}]"#
            ),
            vec![RunFact {
                id: 5,
                name: "CI".into(),
                status: "completed".into(),
                conclusion: "failure".into()
            }]
        );
        assert!(parse_pulls("not json").is_empty());
    }
}
