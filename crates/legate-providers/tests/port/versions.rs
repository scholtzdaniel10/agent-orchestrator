use legate_providers::{
    ProviderId, TESTED_VERSIONS, VersionStatus, compare_to_tested, parse_cli_version,
    unknown_format_message,
};

#[test]
fn parse_cli_version_reads_the_real_version_strings() {
    assert_eq!(
        parse_cli_version(ProviderId::Claude, "2.1.292 (Claude Code)").as_deref(),
        Some("2.1.292")
    );
    assert_eq!(
        parse_cli_version(ProviderId::Cursor, "2026.10.01-e373342").as_deref(),
        Some("2026.10.01-e373342")
    );
}

#[test]
fn parse_cli_version_takes_the_first_token_that_starts_with_a_digit() {
    assert_eq!(
        parse_cli_version(ProviderId::Claude, "  warning 2.1.292 extra").as_deref(),
        Some("2.1.292")
    );
    assert_eq!(
        parse_cli_version(ProviderId::Cursor, "agent 2026.10.01-e373342").as_deref(),
        Some("2026.10.01-e373342")
    );
}

#[test]
fn parse_cli_version_returns_none_for_junk_and_empty_output() {
    assert_eq!(parse_cli_version(ProviderId::Claude, ""), None);
    assert_eq!(parse_cli_version(ProviderId::Claude, "   \n"), None);
    assert_eq!(parse_cli_version(ProviderId::Cursor, "not-a-version"), None);
    assert_eq!(parse_cli_version(ProviderId::Claude, "(Claude Code)"), None);
}

#[test]
fn compare_to_tested_treats_equal_claude_numeric_versions_as_tested() {
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some(TESTED_VERSIONS.claude)),
        VersionStatus::Tested
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("2.1.292")),
        VersionStatus::Tested
    );
}

#[test]
fn compare_to_tested_ranks_claude_by_numeric_dot_parts() {
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("2.2.0")),
        VersionStatus::Newer
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("2.1.293")),
        VersionStatus::Newer
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("3.0.0")),
        VersionStatus::Newer
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("2.0.1")),
        VersionStatus::Older
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("2.1.291")),
        VersionStatus::Older
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("2.1")),
        VersionStatus::Older
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("2.1.292.1")),
        VersionStatus::Newer
    );
}

#[test]
fn compare_to_tested_ranks_cursor_by_the_leading_date() {
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some(TESTED_VERSIONS.cursor)),
        VersionStatus::Tested
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some("2026.10.01")),
        VersionStatus::Tested
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some("2026.10.01-ffffff")),
        VersionStatus::Tested
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some("2026.10.02-abc")),
        VersionStatus::Newer
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some("2026.11.01-6caf4ff")),
        VersionStatus::Newer
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some("2026.09.30-6caf4ff")),
        VersionStatus::Older
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some("2025.12.31-zzzz")),
        VersionStatus::Older
    );
}

#[test]
fn compare_to_tested_is_unknown_when_the_version_cannot_be_parsed() {
    assert_eq!(
        compare_to_tested(ProviderId::Claude, None),
        VersionStatus::Unknown
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, None),
        VersionStatus::Unknown
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("not-a-version")),
        VersionStatus::Unknown
    );
    assert_eq!(
        compare_to_tested(ProviderId::Claude, Some("2.1.292-beta")),
        VersionStatus::Unknown
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some("2.1.292")),
        VersionStatus::Unknown
    );
    assert_eq!(
        compare_to_tested(ProviderId::Cursor, Some("2026.9.8-6caf4ff")),
        VersionStatus::Unknown
    );
}

#[test]
fn unknown_format_message_names_the_cli_and_the_tested_version() {
    assert_eq!(
        unknown_format_message(ProviderId::Claude, Some("2.1.292")),
        "The claude CLI (version 2.1.292) sent output this app does not understand. \
This app was tested with 2.1.292. Update the app, or install the tested CLI version."
    );
    assert_eq!(
        unknown_format_message(ProviderId::Cursor, None),
        "The Cursor CLI (version unknown) sent output this app does not understand. \
This app was tested with 2026.10.01-e373342. Update the app, or install the tested CLI version."
    );
}
