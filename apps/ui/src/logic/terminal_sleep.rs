//! Which terminals may sleep. Port of `terminal-sleep.ts`.

use std::collections::{HashMap, HashSet};

use legate_core::{JobRecord, JobStatus, TerminalInfo, TerminalProvider, TerminalStatus};

pub const SLEEP_AFTER_MS: i64 = 90_000;

pub fn terminals_to_sleep(
    terminals: &[TerminalInfo],
    visible: &HashSet<String>,
    activity: &HashMap<String, i64>,
    jobs: &[JobRecord],
    now: i64,
) -> Vec<String> {
    let busy: HashSet<Option<String>> = jobs
        .iter()
        .filter(|job| job.status == JobStatus::Queued || job.status == JobStatus::Running)
        .map(|job| job.change.clone())
        .collect();
    terminals
        .iter()
        .filter(|info| {
            if info.provider == TerminalProvider::Shell || info.status != TerminalStatus::Running {
                return false;
            }
            if visible.contains(&info.id) {
                return false;
            }
            if busy.contains(&info.change) {
                return false;
            }
            let last = activity
                .get(&info.id)
                .copied()
                .unwrap_or(0)
                .max(info.started_at);
            now - last >= SLEEP_AFTER_MS
        })
        .map(|info| info.id.clone())
        .collect()
}

pub fn terminals_to_wake(terminals: &[TerminalInfo], visible: &HashSet<String>) -> Vec<String> {
    terminals
        .iter()
        .filter(|info| info.status == TerminalStatus::Sleeping && visible.contains(&info.id))
        .map(|info| info.id.clone())
        .collect()
}

/// True when the chunk puts text on screen. Titles and cursor moves do not.
pub fn prints_something(data: &str) -> bool {
    let chars: Vec<char> = data.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let code = chars[index] as u32;
        if code == 0x1b {
            let next = chars.get(index + 1).copied();
            if next == Some(']') {
                let mut end = index + 2;
                while end < chars.len() && chars[end] as u32 != 0x07 && chars[end] as u32 != 0x1b {
                    end += 1;
                }
                index = if chars.get(end).copied().map(|ch| ch as u32) == Some(0x07) {
                    end + 1
                } else {
                    end + 2
                };
                continue;
            }
            if next == Some('[') {
                let mut end = index + 2;
                while end < chars.len() {
                    let byte = chars[end] as u32;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                    end += 1;
                }
                index = end + 1;
                continue;
            }
            index += 2;
            continue;
        }
        if code > 0x20 && code != 0x7f {
            return true;
        }
        index += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{SLEEP_AFTER_MS, prints_something, terminals_to_sleep, terminals_to_wake};
    use legate_core::{
        JobRecord, JobStatus, JobType, ProviderId, TerminalInfo, TerminalProvider, TerminalStatus,
    };
    use std::collections::{BTreeMap, HashMap, HashSet};

    const NOW: i64 = 10_000_000;

    fn term(id: &str) -> TerminalInfo {
        TerminalInfo {
            id: id.into(),
            provider: TerminalProvider::Grok,
            title: id.into(),
            status: TerminalStatus::Running,
            exit_code: None,
            model: None,
            started_at: NOW - 10 * SLEEP_AFTER_MS,
            change: None,
        }
    }

    fn job(id: &str, status: JobStatus, change: Option<&str>) -> JobRecord {
        JobRecord {
            id: id.into(),
            job_type: JobType::Refactor,
            prompt: String::new(),
            provider: Some(ProviderId::Grok),
            status,
            output: String::new(),
            failed_over: Vec::new(),
            steps: None,
            steps_dropped: None,
            model: None,
            reason: None,
            edit: None,
            access: None,
            change: change.map(str::to_string),
            group: None,
            lead_message: None,
            routine: None,
            title: None,
            session_id: None,
            goal: None,
            base: None,
            error: None,
            owner_id: None,
            heartbeat_at: None,
            cancel_requested: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn only_quiet_hidden_agent_terminals_sleep() {
        let mut quiet = term("quiet");
        quiet.change = Some("aaaa1111".into());
        let mut shown = term("shown");
        shown.change = Some("aaaa1111".into());
        let mut chatty = term("chatty");
        chatty.change = Some("bbbb2222".into());
        let mut shell = term("shell");
        shell.provider = TerminalProvider::Shell;
        shell.change = Some("cccc3333".into());
        let mut busy = term("busy");
        busy.change = Some("dddd4444".into());
        let mut fresh = term("fresh");
        fresh.started_at = NOW - 1000;
        let mut exited = term("exited");
        exited.status = TerminalStatus::Exited;
        let mut asleep = term("asleep");
        asleep.status = TerminalStatus::Sleeping;
        let terminals = vec![quiet, shown, chatty, shell, busy, fresh, exited, asleep];
        let jobs = vec![
            job("j", JobStatus::Running, Some("dddd4444")),
            job("k", JobStatus::Done, Some("aaaa1111")),
        ];
        let ids = terminals_to_sleep(
            &terminals,
            &HashSet::from(["shown".into()]),
            &HashMap::from([("chatty".into(), NOW - 5000)]),
            &jobs,
            NOW,
        );
        assert_eq!(ids, vec!["quiet"]);
    }

    #[test]
    fn a_sleeping_terminal_wakes_when_shown() {
        let mut a = term("a");
        a.status = TerminalStatus::Sleeping;
        let mut b = term("b");
        b.status = TerminalStatus::Sleeping;
        assert_eq!(
            terminals_to_wake(&[a, b], &HashSet::from(["b".into()])),
            vec!["b"]
        );
    }

    #[test]
    fn titles_and_bare_controls_are_not_output() {
        assert!(!prints_something(
            "\u{001b}]0;npm exec mcp-remote https://example\u{0007}"
        ));
        assert!(!prints_something(
            "\u{001b}]0;Windows PowerShell\u{0007}\u{001b}[?25h\u{001b}[2J\r\n"
        ));
        assert!(prints_something(
            "\u{001b}[38;1H  Grok 4.7 256K High\u{001b}[K"
        ));
        assert!(prints_something("Reading src/app.ts\r\n"));
    }
}
