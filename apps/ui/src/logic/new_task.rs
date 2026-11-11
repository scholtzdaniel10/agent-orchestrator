//! Decisions for the New task dialog. Port of `new-task.ts`.

use legate_core::{JobAccess, JobType, ProviderId, plan_of};

pub const DEFAULT_WHO: ProviderId = ProviderId::Cursor;
pub const DEFAULT_HOW: How = How::Terminal;
pub const DEFAULT_KIND: JobType = JobType::Refactor;
pub const DEFAULT_COUNT: i64 = 1;
pub const DEFAULT_ROUNDS: i64 = 5;
pub const COUNT_MIN: i64 = 1;
pub const COUNT_MAX: i64 = 6;
pub const ROUNDS_MIN: i64 = 1;
pub const ROUNDS_MAX: i64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Terminal,
    Background,
    Goal,
}

impl How {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Background => "background",
            Self::Goal => "goal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhoOption {
    pub id: ProviderId,
    pub label: &'static str,
    pub note: Option<&'static str>,
}

pub const WHO_OPTIONS: [WhoOption; 4] = [
    WhoOption {
        id: ProviderId::Codex,
        label: "Codex",
        note: Some("ChatGPT plan"),
    },
    WhoOption {
        id: ProviderId::Cursor,
        label: "Cursor",
        note: None,
    },
    WhoOption {
        id: ProviderId::Claude,
        label: "Claude",
        note: None,
    },
    WhoOption {
        id: ProviderId::Grok,
        label: "Grok",
        note: Some("Cursor plan"),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HowOption {
    pub id: How,
    pub label: &'static str,
    pub detail: &'static str,
}

pub const HOW_OPTIONS: [HowOption; 3] = [
    HowOption {
        id: How::Terminal,
        label: "In a terminal",
        detail: "Starts the agent in its own worktree. You watch and talk to it.",
    },
    HowOption {
        id: How::Background,
        label: "In the background",
        detail: "Runs as a job in its own worktree. You review the change.",
    },
    HowOption {
        id: How::Goal,
        label: "As a goal",
        detail: "The lead plans rounds and asks you before each one.",
    },
];

/// Kind order shown in the dialog, which is not the enum declaration order.
pub const JOB_KINDS: [JobType; 5] = [
    JobType::Debugging,
    JobType::Refactor,
    JobType::Boilerplate,
    JobType::Planning,
    JobType::Review,
];

#[derive(Debug, Clone, PartialEq)]
pub struct NewTaskDraft {
    pub text: String,
    pub who: ProviderId,
    pub how: How,
    pub count: f64,
    pub kind: JobType,
    pub check: String,
    pub max_rounds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewTaskContext {
    pub is_repo: bool,
    pub claude_available: bool,
    pub codex_available: bool,
    pub cursor_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTaskGoalInput {
    pub text: String,
    pub check: String,
    pub max_rounds: Option<i64>,
    pub quick_type: Option<JobType>,
    pub quick_access: Option<JobAccess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewTaskPlan {
    Invalid {
        error: String,
    },
    Terminal {
        text: String,
        who: ProviderId,
        count: i64,
    },
    Job {
        job_type: JobType,
        text: String,
        who: ProviderId,
        access: JobAccess,
    },
    Goal {
        input: NewTaskGoalInput,
        created: &'static str,
    },
}

pub fn provider_ready(who: ProviderId, ctx: NewTaskContext) -> bool {
    if who == ProviderId::Codex {
        return ctx.codex_available;
    }
    if plan_of(who) == ProviderId::Claude {
        ctx.claude_available
    } else {
        ctx.cursor_available
    }
}

pub fn unavailable_hint(ready: bool) -> Option<&'static str> {
    if ready { None } else { Some("Not ready") }
}

pub fn how_enabled(how: How, is_repo: bool) -> bool {
    is_repo || how == How::Background
}

pub fn active_how(how: How, is_repo: bool) -> How {
    if how_enabled(how, is_repo) {
        how
    } else {
        How::Background
    }
}

pub fn repo_hint(is_repo: bool) -> Option<&'static str> {
    if is_repo {
        None
    } else {
        Some("Worktrees need a git repository.")
    }
}

pub fn submit_label(how: How) -> &'static str {
    match how {
        How::Terminal => "Start agent",
        How::Background => "Run job",
        How::Goal => "Start goal",
    }
}

pub fn landing_line(is_repo: bool) -> &'static str {
    if is_repo {
        "Pick it in the sidebar, then Review to merge."
    } else {
        "This job only reads the folder. It does not change files."
    }
}

pub fn clamp_count(value: f64) -> i64 {
    if !value.is_finite() {
        return DEFAULT_COUNT;
    }
    (value.round() as i64).clamp(COUNT_MIN, COUNT_MAX)
}

pub fn clamp_rounds(value: f64) -> i64 {
    if !value.is_finite() {
        return DEFAULT_ROUNDS;
    }
    (value.round() as i64).clamp(ROUNDS_MIN, ROUNDS_MAX)
}

pub fn as_kind(value: &str) -> JobType {
    JOB_KINDS
        .into_iter()
        .find(|kind| kind.as_str() == value)
        .unwrap_or(DEFAULT_KIND)
}

fn provider_name(who: ProviderId) -> &'static str {
    match who {
        ProviderId::Cursor => "Cursor",
        ProviderId::Grok => "Grok",
        ProviderId::Claude | ProviderId::Local => "Claude",
        ProviderId::Codex => "Codex",
    }
}

pub fn plan_new_task(draft: &NewTaskDraft, ctx: NewTaskContext) -> NewTaskPlan {
    let text = draft.text.trim();
    if text.is_empty() {
        return NewTaskPlan::Invalid {
            error: "Write what should be done.".into(),
        };
    }
    if !provider_ready(draft.who, ctx) {
        return NewTaskPlan::Invalid {
            error: format!(
                "{} is not ready. Pick a plan that is, or set it up and try again.",
                provider_name(draft.who)
            ),
        };
    }
    let how = active_how(draft.how, ctx.is_repo);
    let check = draft.check.trim();
    if !ctx.is_repo {
        return NewTaskPlan::Job {
            job_type: draft.kind,
            text: text.to_string(),
            who: draft.who,
            access: JobAccess::Read,
        };
    }
    if how == How::Terminal {
        return NewTaskPlan::Terminal {
            text: text.to_string(),
            who: draft.who,
            count: clamp_count(draft.count),
        };
    }
    if how == How::Background {
        if check.is_empty() {
            return NewTaskPlan::Job {
                job_type: draft.kind,
                text: text.to_string(),
                who: draft.who,
                access: JobAccess::Edit,
            };
        }
        return NewTaskPlan::Goal {
            created: "goal",
            input: NewTaskGoalInput {
                text: text.to_string(),
                check: check.to_string(),
                max_rounds: None,
                quick_type: Some(draft.kind),
                quick_access: Some(JobAccess::Edit),
            },
        };
    }
    NewTaskPlan::Goal {
        created: "goal",
        input: NewTaskGoalInput {
            text: text.to_string(),
            check: check.to_string(),
            max_rounds: Some(clamp_rounds(draft.max_rounds)),
            quick_type: None,
            quick_access: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_COUNT, DEFAULT_HOW, DEFAULT_KIND, DEFAULT_ROUNDS, DEFAULT_WHO, HOW_OPTIONS, How,
        JOB_KINDS, NewTaskContext, NewTaskDraft, NewTaskGoalInput, NewTaskPlan, WHO_OPTIONS,
        active_how, as_kind, clamp_count, clamp_rounds, how_enabled, landing_line, plan_new_task,
        provider_ready, repo_hint, submit_label, unavailable_hint,
    };
    use legate_core::{JobAccess, JobType, ProviderId};

    fn draft(text: &str) -> NewTaskDraft {
        NewTaskDraft {
            text: text.into(),
            who: ProviderId::Cursor,
            how: How::Terminal,
            count: 1.0,
            kind: JobType::Refactor,
            check: String::new(),
            max_rounds: 5.0,
        }
    }

    fn ready() -> NewTaskContext {
        NewTaskContext {
            is_repo: true,
            claude_available: true,
            codex_available: true,
            cursor_available: true,
        }
    }

    fn not_repo() -> NewTaskContext {
        NewTaskContext {
            is_repo: false,
            ..ready()
        }
    }

    #[test]
    fn grok_is_offered_and_ready_exactly_when_cursor_is() {
        let listed: Vec<_> = WHO_OPTIONS
            .iter()
            .map(|option| (option.id.as_str(), option.label, option.note))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("codex", "Codex", Some("ChatGPT plan")),
                ("cursor", "Cursor", None),
                ("claude", "Claude", None),
                ("grok", "Grok", Some("Cursor plan")),
            ]
        );
        assert_eq!(
            provider_ready(ProviderId::Grok, ready()),
            provider_ready(ProviderId::Cursor, ready())
        );
        let cursor_out = NewTaskContext {
            cursor_available: false,
            ..ready()
        };
        assert!(!provider_ready(ProviderId::Grok, cursor_out));
        assert!(!provider_ready(ProviderId::Cursor, cursor_out));
        assert!(provider_ready(ProviderId::Claude, cursor_out));
        let claude_out = NewTaskContext {
            claude_available: false,
            ..ready()
        };
        assert!(provider_ready(ProviderId::Grok, claude_out));
        assert!(provider_ready(ProviderId::Cursor, claude_out));
        let mut grok = draft("Fix the pager");
        grok.who = ProviderId::Grok;
        assert_eq!(
            plan_new_task(&grok, cursor_out),
            NewTaskPlan::Invalid {
                error: "Grok is not ready. Pick a plan that is, or set it up and try again.".into(),
            }
        );
        assert_eq!(
            plan_new_task(&grok, ready()),
            NewTaskPlan::Terminal {
                text: "Fix the pager".into(),
                who: ProviderId::Grok,
                count: 1,
            }
        );
    }

    #[test]
    fn defaults_are_cursor_a_terminal_and_a_refactor() {
        assert_eq!(DEFAULT_WHO, ProviderId::Cursor);
        assert_eq!(DEFAULT_HOW, How::Terminal);
        assert_eq!(DEFAULT_KIND, JobType::Refactor);
        assert_eq!(DEFAULT_COUNT, 1);
        assert_eq!(DEFAULT_ROUNDS, 5);
        assert_eq!(
            JOB_KINDS.map(|kind| kind.as_str()),
            ["debugging", "refactor", "boilerplate", "planning", "review"]
        );
    }

    #[test]
    fn how_choices_explain_the_three_ways_to_start() {
        let listed: Vec<_> = HOW_OPTIONS
            .iter()
            .map(|option| (option.id.as_str(), option.label, option.detail))
            .collect();
        assert_eq!(
            listed,
            vec![
                (
                    "terminal",
                    "In a terminal",
                    "Starts the agent in its own worktree. You watch and talk to it."
                ),
                (
                    "background",
                    "In the background",
                    "Runs as a job in its own worktree. You review the change."
                ),
                (
                    "goal",
                    "As a goal",
                    "The lead plans rounds and asks you before each one."
                ),
            ]
        );
    }

    #[test]
    fn labels_follow_the_way_the_work_starts() {
        assert_eq!(submit_label(How::Terminal), "Start agent");
        assert_eq!(submit_label(How::Background), "Run job");
        assert_eq!(submit_label(How::Goal), "Start goal");
        assert_eq!(
            landing_line(true),
            "Pick it in the sidebar, then Review to merge."
        );
        assert_eq!(
            landing_line(false),
            "This job only reads the folder. It does not change files."
        );
        assert_eq!(repo_hint(true), None);
        assert_eq!(repo_hint(false), Some("Worktrees need a git repository."));
        assert_eq!(unavailable_hint(true), None);
        assert_eq!(unavailable_hint(false), Some("Not ready"));
    }

    #[test]
    fn outside_a_repository_only_a_background_job_is_allowed() {
        assert!(!how_enabled(How::Terminal, false));
        assert!(!how_enabled(How::Goal, false));
        assert!(how_enabled(How::Background, false));
        assert!(how_enabled(How::Terminal, true));
        assert_eq!(active_how(How::Terminal, false), How::Background);
        assert_eq!(active_how(How::Goal, false), How::Background);
        assert_eq!(active_how(How::Goal, true), How::Goal);
    }

    #[test]
    fn count_stays_between_1_and_6_rounds_between_1_and_20() {
        assert_eq!(clamp_count(0.0), 1);
        assert_eq!(clamp_count(1.0), 1);
        assert_eq!(clamp_count(6.0), 6);
        assert_eq!(clamp_count(9.0), 6);
        assert_eq!(clamp_count(1.4), 1);
        assert_eq!(clamp_count(f64::NAN), 1);
        assert_eq!(clamp_rounds(0.0), 1);
        assert_eq!(clamp_rounds(20.0), 20);
        assert_eq!(clamp_rounds(50.0), 20);
        assert_eq!(clamp_rounds(f64::NAN), 5);
        assert_eq!(as_kind("planning"), JobType::Planning);
        assert_eq!(as_kind("nope"), JobType::Refactor);
    }

    #[test]
    fn empty_text_is_an_error_and_makes_no_call() {
        assert_eq!(
            plan_new_task(&draft("   "), ready()),
            NewTaskPlan::Invalid {
                error: "Write what should be done.".into(),
            }
        );
        assert_eq!(
            plan_new_task(&draft(""), not_repo()),
            NewTaskPlan::Invalid {
                error: "Write what should be done.".into(),
            }
        );
    }

    #[test]
    fn a_plan_that_is_not_ready_is_an_error() {
        let cursor_out = NewTaskContext {
            cursor_available: false,
            ..ready()
        };
        assert_eq!(
            plan_new_task(&draft("Fix the pager"), cursor_out),
            NewTaskPlan::Invalid {
                error: "Cursor is not ready. Pick a plan that is, or set it up and try again."
                    .into(),
            }
        );
        let claude_out = NewTaskContext {
            claude_available: false,
            ..ready()
        };
        let mut claude = draft("  Look around  ");
        claude.who = ProviderId::Claude;
        assert_eq!(
            plan_new_task(&claude, claude_out),
            NewTaskPlan::Invalid {
                error: "Claude is not ready. Pick a plan that is, or set it up and try again."
                    .into(),
            }
        );
        assert!(matches!(
            plan_new_task(&draft("  "), cursor_out),
            NewTaskPlan::Invalid { error } if error == "Write what should be done."
        ));
    }

    #[test]
    fn a_terminal_starts_that_agent_and_a_check_is_ignored() {
        assert_eq!(
            plan_new_task(&draft("Fix the pager"), ready()),
            NewTaskPlan::Terminal {
                text: "Fix the pager".into(),
                who: ProviderId::Cursor,
                count: 1,
            }
        );
        let mut shipped = draft("  Ship it  ");
        shipped.who = ProviderId::Claude;
        shipped.count = 9.0;
        shipped.check = "pnpm test".into();
        assert_eq!(
            plan_new_task(&shipped, ready()),
            NewTaskPlan::Terminal {
                text: "Ship it".into(),
                who: ProviderId::Claude,
                count: 6,
            }
        );
    }

    #[test]
    fn a_background_job_with_no_check_edits_in_a_worktree() {
        for check in ["", "   "] {
            let mut item = draft("Fix the pager");
            item.how = How::Background;
            item.kind = JobType::Debugging;
            item.who = ProviderId::Claude;
            item.check = check.into();
            assert_eq!(
                plan_new_task(&item, ready()),
                NewTaskPlan::Job {
                    job_type: JobType::Debugging,
                    text: "Fix the pager".into(),
                    who: ProviderId::Claude,
                    access: JobAccess::Edit,
                }
            );
        }
    }

    #[test]
    fn a_background_job_with_a_check_becomes_a_quick_goal() {
        for kind in JOB_KINDS {
            let mut item = draft("Fix the pager");
            item.how = How::Background;
            item.kind = kind;
            item.check = "  pnpm test  ".into();
            assert_eq!(
                plan_new_task(&item, ready()),
                NewTaskPlan::Goal {
                    created: "goal",
                    input: NewTaskGoalInput {
                        text: "Fix the pager".into(),
                        check: "pnpm test".into(),
                        max_rounds: None,
                        quick_type: Some(kind),
                        quick_access: Some(JobAccess::Edit),
                    },
                }
            );
        }
    }

    #[test]
    fn a_goal_keeps_the_check_and_the_round_cap() {
        let mut item = draft("Fix the pager");
        item.how = How::Goal;
        assert_eq!(
            plan_new_task(&item, ready()),
            NewTaskPlan::Goal {
                created: "goal",
                input: NewTaskGoalInput {
                    text: "Fix the pager".into(),
                    check: String::new(),
                    max_rounds: Some(5),
                    quick_type: None,
                    quick_access: None,
                },
            }
        );
        item.check = " pnpm test ".into();
        item.max_rounds = 50.0;
        item.who = ProviderId::Claude;
        assert_eq!(
            plan_new_task(&item, ready()),
            NewTaskPlan::Goal {
                created: "goal",
                input: NewTaskGoalInput {
                    text: "Fix the pager".into(),
                    check: "pnpm test".into(),
                    max_rounds: Some(20),
                    quick_type: None,
                    quick_access: None,
                },
            }
        );
    }

    #[test]
    fn outside_a_repository_every_choice_is_a_read_only_job() {
        for how in [How::Terminal, How::Background, How::Goal] {
            for who in [ProviderId::Cursor, ProviderId::Claude, ProviderId::Grok] {
                let item = NewTaskDraft {
                    text: "  Read the pager  ".into(),
                    who,
                    how,
                    count: 4.0,
                    kind: JobType::Planning,
                    check: "pnpm test".into(),
                    max_rounds: 9.0,
                };
                assert_eq!(
                    plan_new_task(&item, not_repo()),
                    NewTaskPlan::Job {
                        job_type: JobType::Planning,
                        text: "Read the pager".into(),
                        who,
                        access: JobAccess::Read,
                    }
                );
            }
        }
    }

    #[test]
    fn outside_a_repository_a_missing_plan_still_makes_no_call() {
        let ctx = NewTaskContext {
            cursor_available: false,
            ..not_repo()
        };
        let mut item = draft("Read it");
        item.how = How::Background;
        assert!(matches!(
            plan_new_task(&item, ctx),
            NewTaskPlan::Invalid { .. }
        ));
    }
    #[test]
    fn codex_can_run_without_a_cursor_installation() {
        let ctx = NewTaskContext {
            is_repo: true,
            claude_available: false,
            cursor_available: false,
            codex_available: true,
        };
        assert!(provider_ready(ProviderId::Codex, ctx));
        assert!(!provider_ready(ProviderId::Cursor, ctx));
        assert!(!provider_ready(ProviderId::Claude, ctx));
    }
}
