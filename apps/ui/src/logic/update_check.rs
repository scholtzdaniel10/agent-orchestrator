//! When to look for CLI updates on its own, and the count the Settings badge shows. Saved in
//! local storage as `{"checkedAt": <epoch ms>, "newer": <count>}`.

use serde_json::{Value, json};

pub const UPDATES_KEY: &str = "orch.updates";

/// How long a check stays fresh before Legate looks again on start.
pub const CHECK_EVERY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SavedCheck {
    pub checked_at: Option<i64>,
    pub newer: usize,
}

pub fn parse_saved(raw: Option<&str>) -> SavedCheck {
    let Some(value) = raw.and_then(|text| serde_json::from_str::<Value>(text).ok()) else {
        return SavedCheck::default();
    };
    SavedCheck {
        checked_at: value.get("checkedAt").and_then(Value::as_i64),
        newer: value
            .get("newer")
            .and_then(Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0),
    }
}

pub fn saved_text(checked_at: i64, newer: usize) -> String {
    json!({ "checkedAt": checked_at, "newer": newer }).to_string()
}

/// Never checked, a day has passed, or the clock went backwards.
pub fn is_due(saved: SavedCheck, now: i64) -> bool {
    match saved.checked_at {
        None => true,
        Some(at) => now < at || now - at >= CHECK_EVERY_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_check_round_trips() {
        let saved = parse_saved(Some(&saved_text(1_000, 2)));
        assert_eq!(
            saved,
            SavedCheck {
                checked_at: Some(1_000),
                newer: 2
            }
        );
    }

    #[test]
    fn nothing_or_junk_saved_reads_as_never_checked() {
        assert_eq!(parse_saved(None), SavedCheck::default());
        assert_eq!(parse_saved(Some("not json")), SavedCheck::default());
        assert_eq!(parse_saved(Some(r#"{"newer":-1}"#)).newer, 0);
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let at = 10 * CHECK_EVERY_MS;
        let saved = SavedCheck {
            checked_at: Some(at),
            newer: 0,
        };
        assert!(is_due(SavedCheck::default(), at));
        assert!(!is_due(saved, at + CHECK_EVERY_MS - 1));
        assert!(is_due(saved, at + CHECK_EVERY_MS));
        assert!(is_due(saved, at - 1));
    }
}
