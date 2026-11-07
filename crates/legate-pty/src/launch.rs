use std::collections::BTreeMap;

use legate_core::Platform;

/// A process to place in a PTY. `env` replaces the inherited environment when set;
/// `None` means the host filters the current process environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
    pub env: Option<BTreeMap<String, String>>,
}

/// What the host asks the provider launcher to put on the command line.
///
/// See the crate docs for which fields are set. The launcher, not this crate, turns
/// them into `claude --resume <id>` and the other provider flags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchOpts {
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub resume: Option<String>,
    pub prompt: Option<String>,
}

/// Options for [`crate::PtyHost::open`]. Empty `prompt` and empty `resume` are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenOptions {
    /// Directory the CLI runs in. `None` uses the host's project directory.
    pub cwd: Option<String>,
    /// Worktree id. When set, the title is `claude 1 · <change>`.
    pub change: Option<String>,
    /// Initial task passed to the CLI. Empty is ignored.
    pub prompt: Option<String>,
    /// Take over this CLI session instead of starting a new one.
    pub resume: Option<String>,
}

/// Options for [`crate::PtyHost::open_shell`].
#[derive(Debug, Clone, PartialEq)]
pub struct ShellOptions {
    pub cwd: String,
    pub change: Option<String>,
    pub title: String,
    pub commands: Vec<String>,
    pub cols: f64,
    pub rows: f64,
}

/// Shell script used by setup terminals.
///
/// Windows runs `powershell.exe -NoLogo -NoProfile -Command` with a `$LASTEXITCODE`
/// check after every line, so the first failing command stops the script. Other
/// platforms run `sh -c` with the commands joined by ` && `.
pub fn setup_launch(commands: &[String], platform: Platform) -> Launch {
    if platform == Platform::Win32 {
        let mut lines = Vec::with_capacity(commands.len() * 2);
        for command in commands {
            lines.push(command.clone());
            lines.push("if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }".to_string());
        }
        Launch {
            command: "powershell.exe".to_string(),
            args: vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-Command".to_string(),
                lines.join("\n"),
            ],
            env: None,
        }
    } else {
        Launch {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), commands.join(" && ")],
            env: None,
        }
    }
}

/// First line of a `create-chat` result, when it looks like a session id.
pub(crate) fn parse_session_id(text: &str) -> Option<String> {
    let line = text.trim().split(['\r', '\n']).next().unwrap_or("").trim();
    let mut chars = line.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    let mut len = 1usize;
    for ch in chars {
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-')) {
            return None;
        }
        len += 1;
    }
    if !(8..=128).contains(&len) {
        return None;
    }
    Some(line.to_string())
}

/// Build a `create_chat` callback from an async function.
pub fn chat_fn<F, Fut>(f: F) -> ChatFn
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<String, crate::PtyError>> + Send + 'static,
{
    std::sync::Arc::new(move || Box::pin(f()))
}

pub type ChatFn = std::sync::Arc<
    dyn Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, crate::PtyError>> + Send>,
        > + Send
        + Sync,
>;
