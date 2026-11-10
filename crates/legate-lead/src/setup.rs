//! Whether git, Claude Code, Cursor CLI, and `gh` are installed and signed in.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use legate_providers::ProviderAdapter;
use serde::{Deserialize, Serialize};

use crate::ports::CommandRun;

const CHECK_MS: u64 = 15_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequirementId {
    Git,
    Claude,
    Codex,
    Cursor,
    Gh,
}

impl RequirementId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::Gh => "gh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequirementNeed {
    Required,
    Plan,
    Optional,
}

impl RequirementNeed {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Plan => "plan",
            Self::Optional => "optional",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequirementState {
    Ready,
    Missing,
    SignedOut,
}

impl RequirementState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::SignedOut => "signed-out",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Win32,
    Darwin,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "windows" => Self::Win32,
            "macos" => Self::Darwin,
            _ => Self::Linux,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Install {
    pub command: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub id: RequirementId,
    pub label: String,
    pub need: RequirementNeed,
    pub state: RequirementState,
    pub version: Option<String>,
    pub install: Install,
    pub sign_in: Option<String>,
    pub note: String,
}

/// The slice of a provider adapter setup checks. `Err` is a thrown check.
#[async_trait]
pub trait SetupAdapter: Send + Sync {
    async fn is_installed(&self) -> Result<bool, String>;
    async fn is_signed_in(&self) -> Result<bool, String>;
    async fn version(&self) -> Result<Option<String>, String>;
}

/// A `ProviderAdapter` as a [`SetupAdapter`]. Throws never; the CLI methods return values.
pub struct ProviderSetup(pub Arc<dyn ProviderAdapter>);

#[async_trait]
impl SetupAdapter for ProviderSetup {
    async fn is_installed(&self) -> Result<bool, String> {
        Ok(self.0.is_installed().await)
    }

    async fn is_signed_in(&self) -> Result<bool, String> {
        Ok(self.0.is_signed_in().await)
    }

    async fn version(&self) -> Result<Option<String>, String> {
        Ok(self.0.version().await)
    }
}

pub struct Setup {
    run: Arc<dyn CommandRun>,
    codex: Option<Arc<dyn SetupAdapter>>,
    claude: Arc<dyn SetupAdapter>,
    cursor: Arc<dyn SetupAdapter>,
    platform: Platform,
    check: Duration,
}

impl Setup {
    pub fn new(
        run: Arc<dyn CommandRun>,
        claude: Arc<dyn SetupAdapter>,
        cursor: Arc<dyn SetupAdapter>,
        platform: Platform,
    ) -> Self {
        Self {
            run,
            codex: None,
            claude,
            cursor,
            platform,
            check: Duration::from_millis(CHECK_MS),
        }
    }

    pub fn with_codex(mut self, adapter: Arc<dyn SetupAdapter>) -> Self {
        self.codex = Some(adapter);
        self
    }

    pub fn with_check_timeout(mut self, check: Duration) -> Self {
        self.check = check;
        self
    }

    pub async fn status(&self) -> Vec<Requirement> {
        let (git, claude, cursor, gh) = tokio::join!(
            self.limit(RequirementId::Git, self.check_git()),
            self.limit(
                RequirementId::Claude,
                self.check_plan(RequirementId::Claude)
            ),
            self.limit(
                RequirementId::Cursor,
                self.check_plan(RequirementId::Cursor)
            ),
            self.limit(RequirementId::Gh, self.check_gh()),
        );
        let mut rows = vec![git, claude, cursor, gh];
        if self.codex.is_some() {
            rows.insert(
                2,
                self.limit(RequirementId::Codex, self.check_plan(RequirementId::Codex))
                    .await,
            );
        }
        rows
    }

    pub fn can_work(&self, list: &[Requirement]) -> bool {
        let state = |id: RequirementId| list.iter().find(|row| row.id == id).map(|row| row.state);
        state(RequirementId::Git) == Some(RequirementState::Ready)
            && (state(RequirementId::Claude) == Some(RequirementState::Ready)
                || state(RequirementId::Cursor) == Some(RequirementState::Ready)
                || state(RequirementId::Codex) == Some(RequirementState::Ready))
    }

    async fn limit<F>(&self, id: RequirementId, check: F) -> Requirement
    where
        F: Future<Output = Result<Requirement, String>>,
    {
        match tokio::time::timeout(self.check, check).await {
            Ok(Ok(row)) => row,
            Ok(Err(_)) | Err(_) => self.row(id, RequirementState::Missing, None),
        }
    }

    async fn check_git(&self) -> Result<Requirement, String> {
        let result = self
            .run
            .run("git", &["--version".to_string()], None)
            .await?;
        if result.code != Some(0) {
            return Ok(self.row(RequirementId::Git, RequirementState::Missing, None));
        }
        Ok(self.row(
            RequirementId::Git,
            RequirementState::Ready,
            version_of(&result.stdout),
        ))
    }

    async fn check_plan(&self, id: RequirementId) -> Result<Requirement, String> {
        let adapter: &dyn SetupAdapter = match id {
            RequirementId::Claude => self.claude.as_ref(),
            RequirementId::Codex => self.codex.as_deref().ok_or("Codex adapter unavailable")?,
            _ => self.cursor.as_ref(),
        };
        if !adapter.is_installed().await? {
            return Ok(self.row(id, RequirementState::Missing, None));
        }
        let (signed, version) = tokio::join!(adapter.is_signed_in(), adapter.version());
        let signed = signed?;
        let version = version?;
        if !signed {
            return Ok(self.row(id, RequirementState::SignedOut, version));
        }
        Ok(self.row(id, RequirementState::Ready, version))
    }

    async fn check_gh(&self) -> Result<Requirement, String> {
        let ver = self.run.run("gh", &["--version".to_string()], None).await?;
        if ver.code != Some(0) {
            return Ok(self.row(RequirementId::Gh, RequirementState::Missing, None));
        }
        let version = version_of(&ver.stdout);
        let auth = self
            .run
            .run("gh", &["auth".to_string(), "status".to_string()], None)
            .await?;
        if auth.code != Some(0) {
            return Ok(self.row(RequirementId::Gh, RequirementState::SignedOut, version));
        }
        Ok(self.row(RequirementId::Gh, RequirementState::Ready, version))
    }

    fn row(
        &self,
        id: RequirementId,
        state: RequirementState,
        version: Option<String>,
    ) -> Requirement {
        Requirement {
            id,
            label: label(id).to_string(),
            need: need(id),
            state,
            version,
            install: Install {
                command: install_command(id, self.platform).to_string(),
                url: url(id).to_string(),
            },
            sign_in: sign_in(id).map(str::to_string),
            note: note_for(id, state).to_string(),
        }
    }
}

fn label(id: RequirementId) -> &'static str {
    match id {
        RequirementId::Git => "Git",
        RequirementId::Claude => "Claude Code CLI",
        RequirementId::Codex => "Codex CLI",
        RequirementId::Cursor => "Cursor CLI",
        RequirementId::Gh => "GitHub CLI",
    }
}

fn need(id: RequirementId) -> RequirementNeed {
    match id {
        RequirementId::Git => RequirementNeed::Required,
        RequirementId::Gh => RequirementNeed::Optional,
        RequirementId::Claude | RequirementId::Codex | RequirementId::Cursor => {
            RequirementNeed::Plan
        }
    }
}

fn url(id: RequirementId) -> &'static str {
    match id {
        RequirementId::Git => "https://git-scm.com/downloads",
        RequirementId::Codex => "https://developers.openai.com/codex/cli",
        RequirementId::Claude => "https://claude.com/claude-code",
        RequirementId::Cursor => "https://cursor.com/cli",
        RequirementId::Gh => "https://cli.github.com",
    }
}

fn sign_in(id: RequirementId) -> Option<&'static str> {
    match id {
        RequirementId::Git => None,
        RequirementId::Claude => Some("claude auth login"),
        RequirementId::Codex => Some("codex login"),
        RequirementId::Cursor => Some("agent login"),
        RequirementId::Gh => Some("gh auth login"),
    }
}

fn install_command(id: RequirementId, platform: Platform) -> &'static str {
    match (id, platform) {
        (RequirementId::Codex, _) => "npm install -g @openai/codex",
        (RequirementId::Git, Platform::Win32) => "winget install --id Git.Git -e",
        (RequirementId::Git, Platform::Darwin) => "xcode-select --install",
        (RequirementId::Git, Platform::Linux) => "sudo apt install git",
        (RequirementId::Claude, Platform::Win32) => "winget install --id Anthropic.ClaudeCode -e",
        (RequirementId::Claude, Platform::Darwin | Platform::Linux) => {
            "npm install -g @anthropic-ai/claude-code"
        }
        (RequirementId::Cursor, Platform::Win32) => {
            "irm 'https://cursor.com/install?win32=true' | iex"
        }
        (RequirementId::Cursor, Platform::Darwin | Platform::Linux) => {
            "curl https://cursor.com/install -fsS | bash"
        }
        (RequirementId::Gh, Platform::Win32) => "winget install --id GitHub.cli -e",
        (RequirementId::Gh, Platform::Darwin) => "brew install gh",
        (RequirementId::Gh, Platform::Linux) => "sudo apt install gh",
    }
}

fn note_for(id: RequirementId, state: RequirementState) -> &'static str {
    if state == RequirementState::Ready {
        return "Ready.";
    }
    match id {
        RequirementId::Git => "Needed for worktrees.",
        RequirementId::Codex => {
            "Install Codex CLI, then run codex login with your ChatGPT account. API-key-only sign-ins are not used."
        }
        RequirementId::Claude => {
            if state == RequirementState::SignedOut {
                "Installed. Sign in to use your Claude plan."
            } else {
                "Install the Claude Code CLI, then sign in. Claude Code inside the Claude app does not count."
            }
        }
        RequirementId::Cursor => {
            if state == RequirementState::SignedOut {
                "Installed. Sign in to use your Cursor plan."
            } else {
                "Install the Cursor CLI, then sign in."
            }
        }
        RequirementId::Gh => {
            if state == RequirementState::SignedOut {
                "Installed. Sign in to clone repositories and open pull requests."
            } else {
                "Optional: used to clone your repositories and open pull requests."
            }
        }
    }
}

pub(crate) fn version_of(stdout: &str) -> Option<String> {
    for token in stdout.split_whitespace() {
        if token.starts_with(|ch: char| ch.is_ascii_digit()) {
            return Some(token.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;

    use super::{
        Platform, Requirement, RequirementId, RequirementNeed, RequirementState, Setup,
        SetupAdapter,
    };
    use crate::ports::{CommandOutput, CommandRun};

    struct Call {
        command: String,
        args: Vec<String>,
    }

    type CommandHandler =
        Box<dyn Fn(&str, &[String]) -> Result<CommandOutput, String> + Send + Sync>;

    struct FakeRun {
        calls: Mutex<Vec<Call>>,
        handler: CommandHandler,
    }

    impl FakeRun {
        fn new(
            handler: impl Fn(&str, &[String]) -> Result<CommandOutput, String> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                handler: Box::new(handler),
            })
        }
    }

    #[async_trait]
    impl CommandRun for FakeRun {
        async fn run(
            &self,
            command: &str,
            args: &[String],
            _cwd: Option<&str>,
        ) -> Result<CommandOutput, String> {
            self.calls.lock().unwrap().push(Call {
                command: command.to_string(),
                args: args.to_vec(),
            });
            (self.handler)(command, args)
        }
    }

    struct FakeAdapter {
        installed: bool,
        signed_in: bool,
        version: Option<String>,
        hang: Option<&'static str>,
        throw_on: Option<&'static str>,
    }

    #[async_trait]
    impl SetupAdapter for FakeAdapter {
        async fn is_installed(&self) -> Result<bool, String> {
            if self.hang == Some("installed") {
                std::future::pending::<()>().await;
            }
            if self.throw_on == Some("installed") {
                return Err("boom".to_string());
            }
            Ok(self.installed)
        }

        async fn is_signed_in(&self) -> Result<bool, String> {
            if self.hang == Some("signedIn") {
                std::future::pending::<()>().await;
            }
            if self.throw_on == Some("signedIn") {
                return Err("boom".to_string());
            }
            Ok(self.signed_in)
        }

        async fn version(&self) -> Result<Option<String>, String> {
            if self.hang == Some("version") {
                std::future::pending::<()>().await;
            }
            if self.throw_on == Some("version") {
                return Err("boom".to_string());
            }
            Ok(self.version.clone())
        }
    }

    fn ok_output(stdout: &str) -> CommandOutput {
        CommandOutput {
            code: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn fail_output(stderr: &str) -> CommandOutput {
        CommandOutput {
            code: Some(1),
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    fn ok_run() -> Arc<FakeRun> {
        FakeRun::new(|command, _| {
            if command == "git" {
                return Ok(ok_output("git version 2.45.1\n"));
            }
            if command == "gh" {
                return Ok(ok_output("gh version 2.62.0 (2024-01-01)\n"));
            }
            Ok(fail_output(""))
        })
    }

    fn adapter(installed: bool, signed_in: bool, version: Option<&str>) -> Arc<FakeAdapter> {
        Arc::new(FakeAdapter {
            installed,
            signed_in,
            version: version.map(str::to_string),
            hang: None,
            throw_on: None,
        })
    }

    fn setup_of(
        run: Arc<dyn CommandRun>,
        claude: Arc<dyn SetupAdapter>,
        cursor: Arc<dyn SetupAdapter>,
        platform: Platform,
        check: Option<Duration>,
    ) -> Setup {
        let setup = Setup::new(run, claude, cursor, platform);
        if let Some(check) = check {
            setup.with_check_timeout(check)
        } else {
            setup
        }
    }

    fn ready() -> (Arc<dyn SetupAdapter>, Arc<dyn SetupAdapter>) {
        (
            adapter(true, true, Some("1.0.0")),
            adapter(true, true, Some("2026.10.01")),
        )
    }

    struct Rows {
        git: Requirement,
        claude: Requirement,
        cursor: Requirement,
        gh: Requirement,
    }

    fn by_id(list: Vec<Requirement>) -> Rows {
        let mut git = None;
        let mut claude = None;
        let mut cursor = None;
        let mut gh = None;
        for row in list {
            match row.id {
                RequirementId::Git => git = Some(row),
                RequirementId::Claude => claude = Some(row),
                RequirementId::Codex => {}
                RequirementId::Cursor => cursor = Some(row),
                RequirementId::Gh => gh = Some(row),
            }
        }
        Rows {
            git: git.unwrap(),
            claude: claude.unwrap(),
            cursor: cursor.unwrap(),
            gh: gh.unwrap(),
        }
    }

    #[tokio::test]
    async fn status_returns_four_rows_in_order() {
        let (claude, cursor) = ready();
        let list = setup_of(ok_run(), claude, cursor, Platform::Win32, None)
            .status()
            .await;
        assert_eq!(
            list.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            vec!["git", "claude", "cursor", "gh"]
        );
        assert_eq!(
            list.iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Git", "Claude Code CLI", "Cursor CLI", "GitHub CLI"]
        );
        assert_eq!(
            list.iter().map(|row| row.need.as_str()).collect::<Vec<_>>(),
            vec!["required", "plan", "plan", "optional"]
        );
        assert_eq!(list[0].sign_in, None);
        assert_eq!(list[1].sign_in.as_deref(), Some("claude auth login"));
        assert_eq!(list[2].sign_in.as_deref(), Some("agent login"));
        assert_eq!(list[3].sign_in.as_deref(), Some("gh auth login"));
        let _ = RequirementNeed::Required;
    }

    #[tokio::test]
    async fn git_is_ready_when_version_exits_0() {
        let run = FakeRun::new(|command, _| {
            if command == "git" {
                return Ok(ok_output("git version 2.45.1\n"));
            }
            Ok(ok_output("gh version 2.0.0\n"))
        });
        let (claude, cursor) = ready();
        let rows = by_id(
            setup_of(run.clone(), claude, cursor, Platform::Win32, None)
                .status()
                .await,
        );
        assert_eq!(rows.git.state, RequirementState::Ready);
        assert_eq!(rows.git.version.as_deref(), Some("2.45.1"));
        assert_eq!(rows.git.note, "Ready.");
        assert_eq!(rows.git.sign_in, None);
        assert!(run.calls.lock().unwrap().iter().any(|call| {
            call.command == "git" && call.args.first().map(String::as_str) == Some("--version")
        }));
    }

    #[tokio::test]
    async fn git_is_missing_when_version_fails() {
        let run = FakeRun::new(|command, _| {
            if command == "git" {
                return Ok(fail_output("not found"));
            }
            Ok(ok_output("gh version 2.0.0\n"))
        });
        let (claude, cursor) = ready();
        let git = setup_of(run, claude, cursor, Platform::Win32, None)
            .status()
            .await
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(git.state, RequirementState::Missing);
        assert!(git.version.is_none());
        assert_eq!(git.note, "Needed for worktrees.");
    }

    #[tokio::test]
    async fn claude_and_cursor_cover_missing_signed_out_and_ready() {
        let cases = [
            (
                RequirementId::Claude,
                false,
                false,
                RequirementState::Missing,
                "Install the Claude Code CLI, then sign in. Claude Code inside the Claude app does not count.",
            ),
            (
                RequirementId::Claude,
                true,
                false,
                RequirementState::SignedOut,
                "Installed. Sign in to use your Claude plan.",
            ),
            (
                RequirementId::Claude,
                true,
                true,
                RequirementState::Ready,
                "Ready.",
            ),
            (
                RequirementId::Cursor,
                false,
                false,
                RequirementState::Missing,
                "Install the Cursor CLI, then sign in.",
            ),
            (
                RequirementId::Cursor,
                true,
                false,
                RequirementState::SignedOut,
                "Installed. Sign in to use your Cursor plan.",
            ),
            (
                RequirementId::Cursor,
                true,
                true,
                RequirementState::Ready,
                "Ready.",
            ),
        ];
        for (id, installed, signed, state, note) in cases {
            let version = if installed { Some("9.9.9") } else { None };
            let fake = adapter(installed, signed, version);
            let (claude, cursor) = if id == RequirementId::Claude {
                (
                    fake.clone() as Arc<dyn SetupAdapter>,
                    adapter(true, true, Some("1.0.0")) as Arc<dyn SetupAdapter>,
                )
            } else {
                (
                    adapter(true, true, Some("1.0.0")) as Arc<dyn SetupAdapter>,
                    fake as Arc<dyn SetupAdapter>,
                )
            };
            let list = setup_of(ok_run(), claude, cursor, Platform::Win32, None)
                .status()
                .await;
            let found = list.iter().find(|row| row.id == id).unwrap();
            assert_eq!(found.state, state, "{} {}", id.as_str(), state.as_str());
            assert_eq!(found.note, note);
            assert_eq!(found.version.as_deref(), version);
        }
    }

    #[tokio::test]
    async fn gh_is_ready_only_when_version_and_auth_exit_0() {
        let run = FakeRun::new(|command, args| {
            if command == "git" {
                return Ok(ok_output("git version 2.45.1\n"));
            }
            if command == "gh" && args.first().map(String::as_str) == Some("--version") {
                return Ok(ok_output("gh version 2.62.0 (2024-01-01)\n"));
            }
            if command == "gh" && args.first().map(String::as_str) == Some("auth") {
                return Ok(ok_output(""));
            }
            Ok(fail_output(""))
        });
        let (claude, cursor) = ready();
        let rows = by_id(
            setup_of(run.clone(), claude, cursor, Platform::Win32, None)
                .status()
                .await,
        );
        assert_eq!(rows.gh.state, RequirementState::Ready);
        assert_eq!(rows.gh.version.as_deref(), Some("2.62.0"));
        assert_eq!(rows.gh.note, "Ready.");
        let gh_args: Vec<Vec<String>> = run
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.command == "gh")
            .map(|call| call.args.clone())
            .collect();
        assert_eq!(
            gh_args,
            vec![
                vec!["--version".to_string()],
                vec!["auth".to_string(), "status".to_string()]
            ]
        );
    }

    #[tokio::test]
    async fn gh_is_signed_out_when_auth_fails() {
        let run = FakeRun::new(|command, args| {
            if command == "git" {
                return Ok(ok_output("git version 2.0.0\n"));
            }
            if command == "gh" && args.first().map(String::as_str) == Some("--version") {
                return Ok(ok_output("gh version 2.62.0\n"));
            }
            Ok(fail_output("not logged in"))
        });
        let (claude, cursor) = ready();
        let rows = by_id(
            setup_of(run, claude, cursor, Platform::Win32, None)
                .status()
                .await,
        );
        assert_eq!(rows.gh.state, RequirementState::SignedOut);
        assert_eq!(rows.gh.version.as_deref(), Some("2.62.0"));
        assert_eq!(
            rows.gh.note,
            "Installed. Sign in to clone repositories and open pull requests."
        );
    }

    #[tokio::test]
    async fn gh_is_missing_when_version_fails() {
        let run = FakeRun::new(|command, _| {
            if command == "git" {
                return Ok(ok_output("git version 2.0.0\n"));
            }
            Ok(fail_output(""))
        });
        let (claude, cursor) = ready();
        let rows = by_id(
            setup_of(run.clone(), claude, cursor, Platform::Win32, None)
                .status()
                .await,
        );
        assert_eq!(rows.gh.state, RequirementState::Missing);
        assert!(rows.gh.version.is_none());
        assert_eq!(
            rows.gh.note,
            "Optional: used to clone your repositories and open pull requests."
        );
        assert!(!run.calls.lock().unwrap().iter().any(|call| {
            call.command == "gh" && call.args.first().map(String::as_str) == Some("auth")
        }));
    }

    #[tokio::test]
    async fn install_commands_and_urls_match_each_platform() {
        let expected = [
            (
                Platform::Win32,
                [
                    "winget install --id Git.Git -e",
                    "winget install --id Anthropic.ClaudeCode -e",
                    "irm 'https://cursor.com/install?win32=true' | iex",
                    "winget install --id GitHub.cli -e",
                ],
            ),
            (
                Platform::Darwin,
                [
                    "xcode-select --install",
                    "npm install -g @anthropic-ai/claude-code",
                    "curl https://cursor.com/install -fsS | bash",
                    "brew install gh",
                ],
            ),
            (
                Platform::Linux,
                [
                    "sudo apt install git",
                    "npm install -g @anthropic-ai/claude-code",
                    "curl https://cursor.com/install -fsS | bash",
                    "sudo apt install gh",
                ],
            ),
        ];
        let urls = [
            "https://git-scm.com/downloads",
            "https://claude.com/claude-code",
            "https://cursor.com/cli",
            "https://cli.github.com",
        ];
        for (platform, commands) in expected {
            let (claude, cursor) = ready();
            let rows = setup_of(ok_run(), claude, cursor, platform, None)
                .status()
                .await;
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(row.install.command, commands[index]);
                assert_eq!(row.install.url, urls[index]);
            }
        }
    }

    #[tokio::test]
    async fn can_work_needs_git_and_one_plan() {
        let (claude, cursor) = ready();
        let setup = setup_of(ok_run(), claude, cursor, Platform::Win32, None);
        let ready_rows = setup.status().await;
        assert!(setup.can_work(&ready_rows));

        let mut git_missing = ready_rows.clone();
        git_missing[0].state = RequirementState::Missing;
        assert!(!setup.can_work(&git_missing));

        let mut no_plans = ready_rows.clone();
        no_plans[1].state = RequirementState::SignedOut;
        no_plans[2].state = RequirementState::SignedOut;
        assert!(!setup.can_work(&no_plans));

        let mut only_claude = ready_rows.clone();
        only_claude[2].state = RequirementState::Missing;
        assert!(setup.can_work(&only_claude));

        let mut only_cursor = ready_rows.clone();
        only_cursor[1].state = RequirementState::SignedOut;
        assert!(setup.can_work(&only_cursor));

        let mut gh_missing = ready_rows;
        gh_missing[3].state = RequirementState::Missing;
        assert!(setup.can_work(&gh_missing));
    }

    #[tokio::test]
    async fn a_hanging_check_counts_as_missing() {
        let run = FakeRun::new(|command, _| {
            if command == "git" {
                return Err("hang".to_string());
            }
            Ok(ok_output("gh version 2.0.0\n"))
        });
        // The git handler returns immediately; a hang is a pending future. Rebuild git as pending.
        let run = HangGit { inner: run };
        let claude = Arc::new(FakeAdapter {
            installed: true,
            signed_in: true,
            version: Some("1.0.0".to_string()),
            hang: Some("installed"),
            throw_on: None,
        });
        let cursor = adapter(true, true, Some("2026.10.01"));
        let rows = by_id(
            setup_of(
                Arc::new(run),
                claude,
                cursor,
                Platform::Win32,
                Some(Duration::from_millis(20)),
            )
            .status()
            .await,
        );
        assert_eq!(rows.git.state, RequirementState::Missing);
        assert_eq!(rows.git.note, "Needed for worktrees.");
        assert_eq!(rows.claude.state, RequirementState::Missing);
        assert_eq!(
            rows.claude.note,
            "Install the Claude Code CLI, then sign in. Claude Code inside the Claude app does not count."
        );
        assert_eq!(rows.cursor.state, RequirementState::Ready);
        assert_eq!(rows.gh.state, RequirementState::Ready);
    }

    struct HangGit {
        inner: Arc<FakeRun>,
    }

    #[async_trait]
    impl CommandRun for HangGit {
        async fn run(
            &self,
            command: &str,
            args: &[String],
            cwd: Option<&str>,
        ) -> Result<CommandOutput, String> {
            if command == "git" {
                std::future::pending::<()>().await;
            }
            self.inner.run(command, args, cwd).await
        }
    }

    #[tokio::test]
    async fn a_check_that_throws_counts_as_missing() {
        let run = FakeRun::new(|command, _| {
            if command == "git" {
                return Err("no git".to_string());
            }
            Ok(ok_output("gh version 2.0.0\n"))
        });
        let cursor = Arc::new(FakeAdapter {
            installed: true,
            signed_in: true,
            version: Some("1.0.0".to_string()),
            hang: None,
            throw_on: Some("signedIn"),
        });
        let rows = by_id(
            setup_of(
                run,
                adapter(true, true, Some("1.0.0")),
                cursor,
                Platform::Win32,
                None,
            )
            .status()
            .await,
        );
        assert_eq!(rows.git.state, RequirementState::Missing);
        assert_eq!(rows.cursor.state, RequirementState::Missing);
        assert_eq!(rows.claude.state, RequirementState::Ready);
    }
    #[tokio::test]
    async fn codex_alone_satisfies_the_cloud_plan_requirement() {
        let setup = Setup::new(
            ok_run(),
            adapter(false, false, None),
            adapter(false, false, None),
            Platform::Win32,
        )
        .with_codex(adapter(true, true, Some("0.167.0")));
        let rows = setup.status().await;
        assert!(
            rows.iter()
                .any(|row| row.id == RequirementId::Codex && row.state == RequirementState::Ready)
        );
        assert!(setup.can_work(&rows));
    }
}
