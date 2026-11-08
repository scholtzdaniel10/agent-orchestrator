//! Usage text from `src/cli/help.ts`. The lines are the TypeScript strings.

use crate::args::{self, ParseFail, Parsed};

const FULL: &str = "\
legate [--project <path>] <command>
legate ui
legate status
legate job \"<prompt>\" [--type planning|debugging|review|refactor|boilerplate] [--worker auto|claude|cursor|grok] [--access read|edit|full] [--json] [--quiet] [--verbose]
legate jobs [--limit N]
legate show <job-id>
legate changes
legate diff <change-id>
legate merge <change-id>
legate discard <change-id> [--yes]
legate pr \"<title>\" [--body \"<text>\"]
legate worktree create [name]
legate worktree list
legate worktree path <change-id-or-name>
legate routine list
legate routine run \"<name>\" [--json] [--quiet] [--verbose]
legate goal \"<text>\" [--check \"<command>\"] [--rounds N] [--yes]
legate goals
legate --help
legate --version";

pub fn usage_for(key: Option<&str>) -> &'static str {
    match key {
        Some("ui") => "legate ui",
        Some("status") => "legate status",
        Some("plans") => "legate plans",
        Some("job") => {
            "legate job \"<prompt>\" [--type planning|debugging|review|refactor|boilerplate] [--worker auto|claude|cursor|grok] [--access read|edit|full] [--json] [--quiet] [--verbose]"
        }
        Some("jobs") => "legate jobs [--limit N]",
        Some("show") => "legate show <job-id>",
        Some("changes") => "legate changes",
        Some("diff") => "legate diff <change-id>",
        Some("merge") => "legate merge <change-id>",
        Some("discard") => "legate discard <change-id> [--yes]",
        Some("pr") => "legate pr \"<title>\" [--body \"<text>\"]",
        Some("worktree") => "legate worktree create [name] | list | path <change-id-or-name>",
        Some("worktree create") => "legate worktree create [name]",
        Some("worktree list") => "legate worktree list",
        Some("worktree path") => "legate worktree path <change-id-or-name>",
        Some("routine") => "legate routine list | run \"<name>\" [--json] [--quiet] [--verbose]",
        Some("routine list") => "legate routine list",
        Some("routine run") => "legate routine run \"<name>\" [--json] [--quiet] [--verbose]",
        Some("goal") => "legate goal \"<text>\" [--check \"<command>\"] [--rounds N] [--yes]",
        Some("goals") => "legate goals",
        _ => FULL,
    }
}

fn known(key: &str) -> bool {
    usage_for(Some(key)) != FULL
}

pub struct EarlyExit {
    pub code: i32,
    pub stderr: bool,
    pub text: String,
}

pub fn early_exit(argv: &[String], version: &str, tty: bool) -> Option<EarlyExit> {
    match args::parse_args(argv) {
        Err(ParseFail { message, command }) => Some(fail(
            2,
            &format!("{message}\n{}\n", usage_for(command.as_deref())),
        )),
        Ok(parsed) => early_parsed(&parsed, version, tty),
    }
}

fn early_parsed(parsed: &Parsed, version: &str, tty: bool) -> Option<EarlyExit> {
    if parsed.flags.help {
        return Some(ok(&format!("{}\n", usage_for(parsed.key.as_deref()))));
    }
    if parsed.flags.version {
        return Some(ok(&format!("legate {version}\n")));
    }
    let command = parsed.command.as_deref();
    if command.is_none() || command == Some("ui") {
        if tty {
            return None;
        }
        return Some(fail(2, &format!("Missing command.\n{}\n", usage_for(None))));
    }
    let command = command.unwrap_or("");
    let key = parsed.key.as_deref().unwrap_or("");
    if !known(command) && !known(key) {
        return Some(fail(
            2,
            &format!("Unknown command \"{command}\".\n{}\n", usage_for(None)),
        ));
    }
    if (command == "worktree" || command == "routine") && parsed.sub.is_none() {
        return Some(fail(
            2,
            &format!("Missing {command} command.\n{}\n", usage_for(Some(command))),
        ));
    }
    if let Some(sub) = parsed.sub.as_deref() {
        let combined = format!("{command} {sub}");
        if !known(&combined) {
            return Some(fail(
                2,
                &format!(
                    "Unknown command \"{combined}\".\n{}\n",
                    usage_for(Some(command))
                ),
            ));
        }
    }
    None
}

fn ok(text: &str) -> EarlyExit {
    EarlyExit {
        code: 0,
        stderr: false,
        text: text.to_string(),
    }
}

fn fail(code: i32, text: &str) -> EarlyExit {
    EarlyExit {
        code,
        stderr: true,
        text: text.to_string(),
    }
}
