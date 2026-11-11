use legate_core::{ProviderId, VersionStatus};

/// CLI builds this app was tested against. Grok is the Cursor CLI.
pub struct TestedVersions {
    pub claude: &'static str,
    pub codex: &'static str,
    pub cursor: &'static str,
    pub grok: &'static str,
}

pub const TESTED_VERSIONS: TestedVersions = TestedVersions {
    claude: "2.1.292",
    codex: "0.167.0-alpha.3.2",
    cursor: "2026.10.01-e373342",
    grok: "2026.10.01-e373342",
};

/// First whitespace-separated token that starts with a digit.
pub fn parse_cli_version(_provider: ProviderId, stdout: &str) -> Option<String> {
    for token in stdout.split_whitespace() {
        if token.starts_with(|c: char| c.is_ascii_digit()) {
            return Some(token.to_string());
        }
    }
    None
}

pub fn compare_to_tested(provider: ProviderId, version: Option<&str>) -> VersionStatus {
    let Some(version) = version else {
        return VersionStatus::Unknown;
    };
    match provider {
        ProviderId::Claude => compare_claude(version, TESTED_VERSIONS.claude),
        ProviderId::Codex => {
            if version == TESTED_VERSIONS.codex {
                VersionStatus::Tested
            } else {
                VersionStatus::Unknown
            }
        }
        ProviderId::Cursor => compare_cursor(version, TESTED_VERSIONS.cursor),
        ProviderId::Grok => compare_cursor(version, TESTED_VERSIONS.grok),
        // No tested build: the on-device provider is not a CLI version.
        ProviderId::Local => VersionStatus::Unknown,
    }
}

pub fn unknown_format_message(provider: ProviderId, version: Option<&str>) -> String {
    let (name, tested) = match provider {
        ProviderId::Claude => ("claude", TESTED_VERSIONS.claude),
        ProviderId::Codex => ("Codex", TESTED_VERSIONS.codex),
        ProviderId::Cursor => ("Cursor", TESTED_VERSIONS.cursor),
        ProviderId::Grok => ("Cursor", TESTED_VERSIONS.grok),
        ProviderId::Local => ("local", "unknown"),
    };
    let shown = version.unwrap_or("unknown");
    format!(
        "The {name} CLI (version {shown}) sent output this app does not understand. \
This app was tested with {tested}. Update the app, or install the tested CLI version."
    )
}

fn compare_claude(version: &str, tested: &str) -> VersionStatus {
    let Some(left) = numeric_parts(version) else {
        return VersionStatus::Unknown;
    };
    let Some(right) = numeric_parts(tested) else {
        return VersionStatus::Unknown;
    };
    let width = left.len().max(right.len());
    for index in 0..width {
        let left_part = left.get(index).copied().unwrap_or(0);
        let right_part = right.get(index).copied().unwrap_or(0);
        if left_part > right_part {
            return VersionStatus::Newer;
        }
        if left_part < right_part {
            return VersionStatus::Older;
        }
    }
    VersionStatus::Tested
}

fn compare_cursor(version: &str, tested: &str) -> VersionStatus {
    let Some(left) = cursor_date(version) else {
        return VersionStatus::Unknown;
    };
    let Some(right) = cursor_date(tested) else {
        return VersionStatus::Unknown;
    };
    match left.cmp(right) {
        std::cmp::Ordering::Greater => VersionStatus::Newer,
        std::cmp::Ordering::Less => VersionStatus::Older,
        std::cmp::Ordering::Equal => VersionStatus::Tested,
    }
}

fn numeric_parts(version: &str) -> Option<Vec<u64>> {
    if version.is_empty() {
        return None;
    }
    let mut nums = Vec::new();
    for part in version.split('.') {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        nums.push(part.parse::<u64>().ok()?);
    }
    if nums.is_empty() { None } else { Some(nums) }
}

fn cursor_date(version: &str) -> Option<&str> {
    let bytes = version.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let date = &version[..10];
    let bytes = date.as_bytes();
    if bytes[4] != b'.' || bytes[7] != b'.' {
        return None;
    }
    if !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    Some(date)
}
