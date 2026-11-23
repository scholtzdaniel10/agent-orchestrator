//! Update checks for the CLIs Setup finds. The latest version comes from each tool's public
//! release feed (npm, GitHub releases); nothing is signed in and no token is read. Updating
//! runs each tool's own updater, so Legate never installs anything itself.

use std::cmp::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::setup::{Requirement, RequirementId, RequirementState};

/// One CLI in Settings → Updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliUpdate {
    pub id: RequirementId,
    pub label: String,
    /// What `--version` printed, as Setup read it. `None` when the tool is not installed.
    pub installed: Option<String>,
    /// The newest release, or `None` when its feed could not be read or it has none.
    pub latest: Option<String>,
    /// `latest` is a higher version than `installed`.
    pub newer: bool,
    /// Installed, so its updater can run. Cursor has no public feed, so its own updater checks.
    pub can_update: bool,
}

/// Where a tool's latest version is published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Npm(&'static str),
    GithubRelease(&'static str),
}

pub const fn feed_of(id: RequirementId) -> Option<Feed> {
    match id {
        RequirementId::Claude => Some(Feed::Npm("@anthropic-ai/claude-code")),
        RequirementId::Codex => Some(Feed::Npm("@openai/codex")),
        RequirementId::Gh => Some(Feed::GithubRelease("cli/cli")),
        RequirementId::Git => Some(Feed::GithubRelease("git-for-windows/git")),
        RequirementId::Cursor => None,
    }
}

impl Feed {
    pub fn url(self) -> String {
        match self {
            Self::Npm(package) => format!("https://registry.npmjs.org/{package}/latest"),
            Self::GithubRelease(repo) => {
                format!("https://api.github.com/repos/{repo}/releases/latest")
            }
        }
    }

    /// The version in the feed's JSON: npm's `version`, or a release's `tag_name` without `v`.
    pub fn parse(self, body: &str) -> Option<String> {
        let json: serde_json::Value = serde_json::from_str(body).ok()?;
        let raw = match self {
            Self::Npm(_) => json.get("version")?.as_str()?,
            Self::GithubRelease(_) => json.get("tag_name")?.as_str()?,
        };
        let version = raw.trim().trim_start_matches('v');
        (!version.is_empty()).then(|| version.to_string())
    }
}

/// The first dotted number in `text` ("git version 2.47.1.windows.2" → [2, 47, 1]).
pub fn version_numbers(text: &str) -> Option<Vec<u64>> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let token: String = text[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let parts: Vec<u64> = token
        .split('.')
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (!parts.is_empty()).then_some(parts)
}

/// Whether `latest` is a higher version than `installed`. Unreadable versions are never newer.
pub fn is_newer(installed: &str, latest: &str) -> bool {
    let (Some(have), Some(next)) = (version_numbers(installed), version_numbers(latest)) else {
        return false;
    };
    let width = have.len().max(next.len());
    let pad = |list: &[u64]| {
        let mut out = list.to_vec();
        out.resize(width, 0);
        out
    };
    pad(&next).cmp(&pad(&have)) == Ordering::Greater
}

/// Builds the Updates list from Setup's rows and the latest versions found.
pub fn updates_from(
    rows: &[Requirement],
    latest: impl Fn(RequirementId) -> Option<String>,
) -> Vec<CliUpdate> {
    rows.iter()
        .map(|row| {
            let installed = (row.state != RequirementState::Missing)
                .then(|| row.version.clone())
                .flatten();
            let latest = latest(row.id);
            let newer = match (&installed, &latest) {
                (Some(have), Some(next)) => is_newer(have, next),
                _ => false,
            };
            CliUpdate {
                id: row.id,
                label: row.label.clone(),
                can_update: installed.is_some(),
                installed,
                latest,
                newer,
            }
        })
        .collect()
}

/// Reads one feed. A slow or failing feed gives `None`; it never fails the whole check.
pub async fn fetch_latest(client: &reqwest::Client, feed: Feed) -> Option<String> {
    let response = client
        .get(feed.url())
        .header(reqwest::header::USER_AGENT, "Legate")
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    feed.parse(&response.text().await.ok()?)
}

/// Looks up the latest version of every installed tool that has a feed, all at once.
pub async fn check_updates(rows: &[Requirement]) -> Vec<CliUpdate> {
    let client = reqwest::Client::new();
    let wanted: Vec<(RequirementId, Feed)> = rows
        .iter()
        .filter(|row| row.state != RequirementState::Missing)
        .filter_map(|row| feed_of(row.id).map(|feed| (row.id, feed)))
        .collect();
    let mut tasks = tokio::task::JoinSet::new();
    for (id, feed) in wanted {
        let client = client.clone();
        tasks.spawn(async move { (id, fetch_latest(&client, feed).await) });
    }
    let mut found = Vec::new();
    while let Some(done) = tasks.join_next().await {
        if let Ok(pair) = done {
            found.push(pair);
        }
    }
    updates_from(rows, |id| {
        found
            .iter()
            .find(|(seen, _)| *seen == id)
            .and_then(|(_, version)| version.clone())
    })
}

/// The PowerShell script the Updates window runs: each update under a heading, then a pause
/// so the output can be read before the window closes.
pub fn update_script(steps: &[(String, String)]) -> String {
    let mut script = String::from("$ErrorActionPreference = 'Continue'\n");
    for (label, line) in steps {
        let heading = label.replace('\'', "''");
        script.push_str(&format!(
            "Write-Host ''\nWrite-Host '== Updating {heading} ==' -ForegroundColor Cyan\n{line}\n"
        ));
    }
    script.push_str(
        "Write-Host ''\nWrite-Host 'Done. Legate checks the versions again when this window closes.' -ForegroundColor Green\nRead-Host 'Press Enter to close'\n",
    );
    script
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{Install, RequirementNeed};

    fn row(id: RequirementId, state: RequirementState, version: Option<&str>) -> Requirement {
        Requirement {
            id,
            label: id.as_str().to_string(),
            need: RequirementNeed::Plan,
            state,
            version: version.map(str::to_string),
            install: Install {
                command: String::new(),
                url: String::new(),
            },
            sign_in: None,
            note: String::new(),
        }
    }

    #[test]
    fn version_numbers_take_the_first_dotted_number() {
        assert_eq!(
            version_numbers("2.0.37 (Claude Code)"),
            Some(vec![2, 0, 37])
        );
        assert_eq!(
            version_numbers("git version 2.47.1.windows.2"),
            Some(vec![2, 47, 1])
        );
        assert_eq!(version_numbers("codex-cli 0.58.0"), Some(vec![0, 58, 0]));
        assert_eq!(version_numbers("no digits"), None);
    }

    #[test]
    fn newer_compares_numbers_not_text() {
        assert!(is_newer("2.0.9", "2.0.10"));
        assert!(is_newer("gh version 2.82.1 (2025-10-01)", "2.83.0"));
        assert!(!is_newer("2.0.10", "2.0.10"));
        assert!(!is_newer("2.1", "2.0.99"));
        assert!(is_newer("2.1", "2.1.1"));
        assert!(!is_newer("unknown", "2.0.0"));
    }

    #[test]
    fn feeds_read_npm_versions_and_release_tags() {
        let npm = Feed::Npm("@openai/codex");
        assert_eq!(
            npm.parse(r#"{"name":"x","version":"0.60.1"}"#).as_deref(),
            Some("0.60.1")
        );
        let gh = Feed::GithubRelease("cli/cli");
        assert_eq!(
            gh.parse(r#"{"tag_name":"v2.83.0"}"#).as_deref(),
            Some("2.83.0")
        );
        assert_eq!(gh.parse("not json"), None);
        assert_eq!(npm.url(), "https://registry.npmjs.org/@openai/codex/latest");
    }

    #[test]
    fn cursor_has_no_feed_and_relies_on_its_own_updater() {
        assert_eq!(feed_of(RequirementId::Cursor), None);
        let list = updates_from(
            &[row(
                RequirementId::Cursor,
                RequirementState::Ready,
                Some("2025.11.06"),
            )],
            |_| None,
        );
        assert!(list[0].can_update && !list[0].newer && list[0].latest.is_none());
    }

    #[test]
    fn missing_tools_are_listed_but_cannot_update() {
        let list = updates_from(
            &[
                row(
                    RequirementId::Claude,
                    RequirementState::Ready,
                    Some("2.0.37 (Claude Code)"),
                ),
                row(RequirementId::Codex, RequirementState::Missing, None),
                row(
                    RequirementId::Gh,
                    RequirementState::SignedOut,
                    Some("gh version 2.83.0"),
                ),
            ],
            |id| match id {
                RequirementId::Claude => Some("2.0.40".into()),
                RequirementId::Codex => Some("0.60.1".into()),
                RequirementId::Gh => Some("2.83.0".into()),
                _ => None,
            },
        );
        assert!(list[0].newer && list[0].can_update);
        assert!(!list[1].can_update && !list[1].newer && list[1].installed.is_none());
        assert!(!list[2].newer && list[2].can_update);
    }

    #[test]
    fn the_script_runs_each_step_then_waits() {
        let script = update_script(&[
            ("Claude Code CLI".into(), "& 'claude' update".into()),
            (
                "GitHub's CLI".into(),
                "winget upgrade --id GitHub.cli -e".into(),
            ),
        ]);
        let claude = script.find("& 'claude' update").expect("claude step");
        let gh = script.find("winget upgrade").expect("gh step");
        assert!(claude < gh);
        assert!(script.contains("'== Updating GitHub''s CLI =='"));
        assert!(
            script
                .trim_end()
                .ends_with("Read-Host 'Press Enter to close'")
        );
    }
}
