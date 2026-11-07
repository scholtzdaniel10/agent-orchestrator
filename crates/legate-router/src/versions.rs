//! CLI version ranking used by worker status. The provider crate owns the live check;
//! these tables match `src/core/providers/versions.ts` so the orchestrator can report the
//! same status without waiting on that crate.

use legate_core::{ProviderId, VersionStatus};

const CLAUDE_TESTED: &str = "2.1.292";
const CURSOR_TESTED: &str = "2026.10.01-e373342";

pub fn tested_version(provider: ProviderId) -> Option<&'static str> {
    match provider {
        ProviderId::Claude => Some(CLAUDE_TESTED),
        ProviderId::Cursor | ProviderId::Grok => Some(CURSOR_TESTED),
        ProviderId::Local => None,
    }
}

pub fn compare_to_tested(provider: ProviderId, version: Option<&str>) -> VersionStatus {
    let Some(version) = version else {
        return VersionStatus::Unknown;
    };
    let Some(tested) = tested_version(provider) else {
        return VersionStatus::Unknown;
    };
    if provider == ProviderId::Claude {
        compare_claude(version, tested)
    } else {
        compare_cursor(version, tested)
    }
}

pub fn unknown_format_message(provider: ProviderId, version: Option<&str>) -> String {
    let name = if provider == ProviderId::Claude {
        "claude"
    } else {
        "Cursor"
    };
    let shown = version.unwrap_or("unknown");
    let tested = tested_version(provider).unwrap_or("unknown");
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
    let n = left.len().max(right.len());
    for i in 0..n {
        let a = left.get(i).copied().unwrap_or(0);
        let b = right.get(i).copied().unwrap_or(0);
        if a > b {
            return VersionStatus::Newer;
        }
        if a < b {
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
        nums.push(part.parse().ok()?);
    }
    if nums.is_empty() { None } else { Some(nums) }
}

fn cursor_date(version: &str) -> Option<&str> {
    let bytes = version.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let date = &version[..10];
    let chars: Vec<char> = date.chars().collect();
    if chars.len() != 10 {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| chars[range].iter().all(|ch| ch.is_ascii_digit());
    if digits(0..4) && chars[4] == '.' && digits(5..7) && chars[7] == '.' && digits(8..10) {
        Some(date)
    } else {
        None
    }
}

/// True when stderr mentions a usage limit, matching the test adapter and the CLI adapters.
pub fn stderr_is_limit(stderr: &str) -> bool {
    stderr.to_lowercase().contains("limit")
}
