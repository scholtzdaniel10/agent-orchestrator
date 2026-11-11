use legate_core::SetupState;
use serde_json::json;

use crate::setup::{SetupTracker, parse_setup_commands, parse_worktree_name};

const NAME: &str = "Name must be 1 to 40 characters on one line.";
const COMMAND: &str = "each command must be 1 to 300 characters on one line";

#[test]
fn setup_tracker_runs_fails_and_finishes() {
    let mut tracker = SetupTracker::new();
    tracker.begin("abcd1234", false);
    assert_eq!(tracker.get("abcd1234"), Some(SetupState::Failed));

    tracker.begin("abcd1234", true);
    assert_eq!(tracker.get("abcd1234"), Some(SetupState::Running));
    tracker.on_terminal(Some("abcd1234"), "claude", true, "exited", Some(0));
    assert_eq!(tracker.get("abcd1234"), Some(SetupState::Running));

    tracker.on_terminal(Some("abcd1234"), "shell", false, "running", None);
    assert_eq!(tracker.get("abcd1234"), Some(SetupState::Running));
    tracker.on_terminal(Some("abcd1234"), "shell", true, "exited", Some(0));
    assert_eq!(tracker.get("abcd1234"), Some(SetupState::Failed));

    tracker.begin("abcd1234", true);
    tracker.on_terminal(Some("abcd1234"), "shell", false, "exited", Some(0));
    assert_eq!(tracker.get("abcd1234"), Some(SetupState::Done));
    tracker.on_terminal(Some("abcd1234"), "shell", true, "removed", None);
    assert_eq!(tracker.get("abcd1234"), Some(SetupState::Done));

    tracker.begin("abcd1234", true);
    tracker.on_terminal(Some("abcd1234"), "shell", false, "exited", Some(1));
    assert_eq!(tracker.get("abcd1234"), Some(SetupState::Failed));
    tracker.on_terminal(None, "shell", false, "exited", Some(0));
    tracker.clear("abcd1234");
    assert_eq!(tracker.get("abcd1234"), None);
}

#[test]
fn worktree_name_is_one_line_up_to_40() {
    assert_eq!(parse_worktree_name(None, false).unwrap(), None);
    assert_eq!(
        parse_worktree_name(None, true).unwrap_err().to_string(),
        NAME
    );
    assert_eq!(
        parse_worktree_name(Some("  login  "), false).unwrap(),
        Some("login".to_string())
    );
    assert_eq!(
        parse_worktree_name(Some("has\nline"), false)
            .unwrap_err()
            .to_string(),
        NAME
    );
    assert_eq!(parse_worktree_name(Some(""), false).unwrap(), None);
    assert_eq!(
        parse_worktree_name(Some("   "), true)
            .unwrap_err()
            .to_string(),
        NAME
    );
    let ok = "a".repeat(40);
    assert_eq!(parse_worktree_name(Some(&ok), true).unwrap(), Some(ok));
    let too_long = "a".repeat(41);
    assert_eq!(
        parse_worktree_name(Some(&too_long), false)
            .unwrap_err()
            .to_string(),
        NAME
    );
}

#[test]
fn setup_commands_are_a_short_list_of_one_line_strings() {
    assert_eq!(
        parse_setup_commands(&json!(["npm install"])).unwrap(),
        vec!["npm install".to_string()]
    );
    assert_eq!(
        parse_setup_commands(&json!("npm")).unwrap_err().to_string(),
        "commands must be an array"
    );
    assert_eq!(
        parse_setup_commands(&json!(["a", "b", "c", "d", "e", "f"]))
            .unwrap_err()
            .to_string(),
        "at most 5 setup commands"
    );
    assert_eq!(
        parse_setup_commands(&json!([""])).unwrap_err().to_string(),
        COMMAND
    );
    assert_eq!(
        parse_setup_commands(&json!(["has\nline"]))
            .unwrap_err()
            .to_string(),
        COMMAND
    );
}
