//! Argument parsing from `src/cli/args.ts`.
//!
//! Clap describes the same commands. The parser itself is the TypeScript one:
//! the tests require its error sentences and the way `--help` wins over a bad flag.

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flags {
    pub type_name: Option<String>,
    pub worker: Option<String>,
    pub access: Option<String>,
    pub limit: Option<String>,
    pub body: Option<String>,
    pub project: Option<String>,
    pub check: Option<String>,
    pub rounds: Option<String>,
    pub json: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub yes: bool,
    pub help: bool,
    pub version: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub command: Option<String>,
    pub sub: Option<String>,
    pub args: Vec<String>,
    pub key: Option<String>,
    pub flags: Flags,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFail {
    pub message: String,
    pub command: Option<String>,
}

const VALUE_FLAGS: &[&str] = &[
    "type", "worker", "access", "limit", "body", "project", "check", "rounds",
];
const BOOL_FLAGS: &[&str] = &["json", "quiet", "verbose", "yes", "help", "version"];
const GLOBAL_FLAGS: &[&str] = &["project", "help", "version"];

fn allowed(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "ui" | "status" | "goals" | "show" | "changes" | "diff" | "merge" | "worktree"
        | "worktree create" | "worktree list" | "worktree path" | "routine" | "routine list"
        | "plans" => &[],
        "job" => &["type", "worker", "access", "json", "quiet", "verbose"],
        "jobs" => &["limit"],
        "goal" => &["check", "rounds", "yes"],
        "discard" => &["yes"],
        "pr" => &["body"],
        "routine run" => &["json", "quiet", "verbose"],
        _ => return None,
    })
}

pub fn parse_args(argv: &[String]) -> Result<Parsed, ParseFail> {
    let mut positionals = Vec::new();
    let mut values: Vec<(String, String)> = Vec::new();
    let mut bools: Vec<String> = Vec::new();
    let mut error: Option<String> = None;
    let mut ended = false;
    let mut index = 0;
    while index < argv.len() {
        let token = argv[index].as_str();
        if ended {
            positionals.push(token.to_string());
            index += 1;
            continue;
        }
        if token == "--" {
            ended = true;
            index += 1;
            continue;
        }
        if token == "-h" {
            push_unique(&mut bools, "help");
            index += 1;
            continue;
        }
        if token == "-v" {
            push_unique(&mut bools, "version");
            index += 1;
            continue;
        }
        if token == "-" {
            positionals.push(token.to_string());
            index += 1;
            continue;
        }
        if !token.starts_with('-') {
            positionals.push(token.to_string());
            index += 1;
            continue;
        }
        if !token.starts_with("--") {
            if error.is_none() {
                error = Some(format!("Unknown flag {token}."));
            }
            index += 1;
            continue;
        }
        let mut name = token[2..].to_string();
        let mut attached: Option<String> = None;
        if let Some(eq) = name.find('=') {
            attached = Some(name[eq + 1..].to_string());
            name.truncate(eq);
        }
        if name.is_empty() || !is_flag_name(&name) {
            if error.is_none() {
                error = Some(format!("Unknown flag --{name}."));
            }
            index += 1;
            continue;
        }
        if BOOL_FLAGS.contains(&name.as_str()) {
            if attached.is_some() {
                if error.is_none() {
                    error = Some(format!("Flag --{name} takes no value."));
                }
                index += 1;
                continue;
            }
            push_unique(&mut bools, &name);
            index += 1;
            continue;
        }
        if !VALUE_FLAGS.contains(&name.as_str()) {
            if error.is_none() {
                error = Some(format!("Unknown flag --{name}."));
            }
            index += 1;
            continue;
        }
        let value = if let Some(value) = attached {
            value
        } else {
            let next = argv.get(index + 1).map(String::as_str);
            if next.is_none()
                || next == Some("--")
                || next.is_some_and(|item| item.starts_with('-'))
            {
                if error.is_none() {
                    error = Some(format!("Missing value for --{name}."));
                }
                index += 1;
                continue;
            }
            index += 1;
            argv[index].clone()
        };
        if value.is_empty() {
            if error.is_none() {
                error = Some(format!("Missing value for --{name}."));
            }
            index += 1;
            continue;
        }
        values.retain(|(key, _)| key != &name);
        values.push((name, value));
        index += 1;
    }

    let split = split_command(&positionals);
    if bools.iter().any(|name| name == "help" || name == "version") {
        error = None;
    }
    if let Some(message) = error {
        return Err(ParseFail {
            message,
            command: split.key.clone(),
        });
    }
    if !bools.iter().any(|name| name == "help" || name == "version")
        && let Some(key) = split.key.as_deref()
        && let Some(allowed) = allowed(key)
    {
        let mut used = bools.clone();
        used.extend(values.iter().map(|(name, _)| name.clone()));
        for name in used {
            if GLOBAL_FLAGS.contains(&name.as_str()) || allowed.contains(&name.as_str()) {
                continue;
            }
            return Err(ParseFail {
                message: format!("Unknown flag --{name}."),
                command: split.key.clone(),
            });
        }
    }

    let mut flags = Flags {
        type_name: None,
        worker: None,
        access: None,
        limit: None,
        body: None,
        project: None,
        check: None,
        rounds: None,
        json: bools.iter().any(|name| name == "json"),
        quiet: bools.iter().any(|name| name == "quiet"),
        verbose: bools.iter().any(|name| name == "verbose"),
        yes: bools.iter().any(|name| name == "yes"),
        help: bools.iter().any(|name| name == "help"),
        version: bools.iter().any(|name| name == "version"),
    };
    for (name, value) in values {
        match name.as_str() {
            "type" => flags.type_name = Some(value),
            "worker" => flags.worker = Some(value),
            "access" => flags.access = Some(value),
            "limit" => flags.limit = Some(value),
            "body" => flags.body = Some(value),
            "project" => flags.project = Some(value),
            "check" => flags.check = Some(value),
            "rounds" => flags.rounds = Some(value),
            _ => {}
        }
    }
    Ok(Parsed {
        command: split.command,
        sub: split.sub,
        args: split.args,
        key: split.key,
        flags,
    })
}

struct Split {
    command: Option<String>,
    sub: Option<String>,
    args: Vec<String>,
    key: Option<String>,
}

fn split_command(positionals: &[String]) -> Split {
    let command = positionals.first().cloned();
    if command.as_deref() == Some("worktree") || command.as_deref() == Some("routine") {
        let sub = positionals.get(1).cloned();
        let key = match &sub {
            None => command.clone(),
            Some(sub) => Some(format!("{} {sub}", command.as_deref().unwrap_or(""))),
        };
        return Split {
            command,
            sub,
            args: positionals.iter().skip(2).cloned().collect(),
            key,
        };
    }
    Split {
        command: command.clone(),
        sub: None,
        args: positionals.iter().skip(1).cloned().collect(),
        key: command,
    }
}

fn push_unique(bools: &mut Vec<String>, name: &str) {
    if !bools.iter().any(|item| item == name) {
        bools.push(name.to_string());
    }
}

fn is_flag_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

/// Clap mirror of the commands. Help text and errors stay in [`crate::help`]
/// because those strings are checked byte for byte.
#[allow(dead_code)]
#[derive(Parser)]
#[command(
    name = "legate",
    version,
    disable_help_flag = true,
    disable_version_flag = true
)]
pub struct LegateCli {
    #[arg(long, global = true)]
    project: Option<String>,
    #[arg(long, short = 'h', global = true, action = clap::ArgAction::SetTrue)]
    help: bool,
    #[arg(long, short = 'v', global = true, action = clap::ArgAction::SetTrue)]
    version: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[allow(dead_code)]
#[derive(Subcommand)]
enum Command {
    Ui,
    Status,
    /// Same screen as `status`. Kept so `legate plans` still works.
    Plans,
    Job {
        prompt: Option<String>,
        #[arg(long)]
        r#type: Option<String>,
        #[arg(long)]
        worker: Option<String>,
        #[arg(long)]
        access: Option<String>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        quiet: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        verbose: bool,
    },
    Jobs {
        #[arg(long)]
        limit: Option<String>,
    },
    Show {
        id: Option<String>,
    },
    Changes,
    Diff {
        id: Option<String>,
    },
    Merge {
        id: Option<String>,
    },
    Discard {
        id: Option<String>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
    Pr {
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
    },
    Worktree {
        #[command(subcommand)]
        action: Option<WorktreeCmd>,
    },
    Routine {
        #[command(subcommand)]
        action: Option<RoutineCmd>,
    },
    Goal {
        text: Option<String>,
        #[arg(long)]
        check: Option<String>,
        #[arg(long)]
        rounds: Option<String>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
    Goals,
}

#[allow(dead_code)]
#[derive(Subcommand)]
enum WorktreeCmd {
    Create { name: Option<String> },
    List,
    Path { id: Option<String> },
}

#[allow(dead_code)]
#[derive(Subcommand)]
enum RoutineCmd {
    List,
    Run {
        name: Option<String>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        json: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        quiet: bool,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        verbose: bool,
    },
}

pub fn clap_name() -> String {
    LegateCli::command().get_name().to_string()
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    fn owned(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn parses_flag_value_equals_and_boolean_flags() {
        let parsed = parse_args(&owned(&[
            "job",
            "--type=review",
            "--worker",
            "cursor",
            "--access",
            "edit",
            "--json",
            "--quiet",
            "fix the tests",
        ]))
        .unwrap();
        assert_eq!(parsed.command.as_deref(), Some("job"));
        assert_eq!(parsed.args, vec!["fix the tests"]);
        assert_eq!(parsed.flags.type_name.as_deref(), Some("review"));
        assert_eq!(parsed.flags.worker.as_deref(), Some("cursor"));
        assert_eq!(parsed.flags.access.as_deref(), Some("edit"));
        assert!(parsed.flags.json);
        assert!(parsed.flags.quiet);
        assert!(!parsed.flags.verbose);
    }

    #[test]
    fn double_dash_ends_flags_so_a_later_type_is_the_prompt() {
        let parsed = parse_args(&owned(&["job", "--", "--type"])).unwrap();
        assert_eq!(parsed.args, vec!["--type"]);
    }

    #[test]
    fn a_bare_dash_is_the_stdin_prompt_marker() {
        let parsed = parse_args(&owned(&["job", "-"])).unwrap();
        assert_eq!(parsed.args, vec!["-"]);
    }

    #[test]
    fn an_unknown_flag_names_itself() {
        let err = parse_args(&owned(&["status", "--bogus"])).unwrap_err();
        assert_eq!(err.message, "Unknown flag --bogus.");
        assert_eq!(err.command.as_deref(), Some("status"));
    }

    #[test]
    fn a_missing_flag_value_is_an_error() {
        let err = parse_args(&owned(&["job", "--type"])).unwrap_err();
        assert_eq!(err.message, "Missing value for --type.");
        assert_eq!(err.command.as_deref(), Some("job"));
    }

    #[test]
    fn a_flag_the_command_does_not_take_is_unknown() {
        let err = parse_args(&owned(&["jobs", "--json"])).unwrap_err();
        assert_eq!(err.message, "Unknown flag --json.");
        assert_eq!(err.command.as_deref(), Some("jobs"));
    }

    #[test]
    fn worktree_and_routine_keep_their_subcommand() {
        let parsed = parse_args(&owned(&["worktree", "path", "login"])).unwrap();
        assert_eq!(parsed.key.as_deref(), Some("worktree path"));
        assert_eq!(parsed.args, vec!["login"]);
    }

    #[test]
    fn help_wins_over_a_bad_flag() {
        let parsed = parse_args(&owned(&["job", "--help", "--type"])).unwrap();
        assert!(parsed.flags.help);
    }

    #[test]
    fn goal_keeps_its_text_check_rounds_and_yes() {
        let parsed = parse_args(&owned(&[
            "goal",
            "--check",
            "pnpm test",
            "--rounds=3",
            "--yes",
            "ship the login",
        ]))
        .unwrap();
        assert_eq!(parsed.command.as_deref(), Some("goal"));
        assert_eq!(parsed.args, vec!["ship the login"]);
        assert_eq!(parsed.flags.check.as_deref(), Some("pnpm test"));
        assert_eq!(parsed.flags.rounds.as_deref(), Some("3"));
        assert!(parsed.flags.yes);
    }

    #[test]
    fn goals_takes_no_flags_of_its_own() {
        let err = parse_args(&owned(&["goals", "--yes"])).unwrap_err();
        assert_eq!(err.message, "Unknown flag --yes.");
        assert_eq!(err.command.as_deref(), Some("goals"));
    }

    #[test]
    fn a_goal_flag_on_another_command_is_unknown() {
        let err = parse_args(&owned(&["jobs", "--check", "pnpm test"])).unwrap_err();
        assert_eq!(err.message, "Unknown flag --check.");
        assert_eq!(err.command.as_deref(), Some("jobs"));
    }
}
