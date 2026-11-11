//! Ports of `src/cli/run.test.ts`.

use std::sync::{Arc, Mutex};

use legate_core::{
    ChangedFile, FileEdit, JobAccess, JobStatus, JobStepStatus, JobType, PlanProblem, ProjectInfo,
    ProviderId, StepTool, VersionStatus,
};
use legate_git::{ListedName, MergeOutcome};
use legate_lead::{GoalCheck, GoalRound, GoalStatus};

use crate::format::change_footer;
use crate::harness::{self, Fake, change, goal, job, plan, planned, proposal, row, step, words};
use crate::run;

async fn go(args: &[&str], fake: &Fake) -> i32 {
    run(&words(args), fake).await
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[tokio::test]
async fn help_version_and_an_unknown_command() {
    let help = Fake::new();
    assert_eq!(go(&["--help"], &help).await, 0);
    assert!(help.out().contains("legate job"));
    assert!(help.out().contains("legate ui"));
    assert!(help.out().contains("legate goal \"<text>\""));
    assert!(help.out().contains("legate goals"));
    assert_eq!(help.err(), "");

    let missing = Fake::new();
    assert_eq!(go(&[], &missing).await, 2);
    assert!(missing.err().contains("Missing command."));
    assert!(missing.err().contains("legate ui"));

    let version = Fake::new();
    assert_eq!(go(&["-v"], &version).await, 0);
    assert_eq!(version.out(), "legate 1.0.0\n");

    let unknown = Fake::new();
    assert_eq!(go(&["nope"], &unknown).await, 2);
    assert!(unknown.err().contains("Unknown command \"nope\"."));
    assert!(unknown.err().contains("legate status"));

    let flag = Fake::new();
    assert_eq!(go(&["status", "--json"], &flag).await, 2);
    assert!(flag.err().contains("Unknown flag --json."));
    assert!(flag.err().contains("legate status"));
}

#[tokio::test]
async fn status_prints_plans_then_the_project_and_live_jobs() {
    let fake = Fake::new();
    let mut cursor = plan(ProviderId::Cursor);
    cursor.available = false;
    cursor.problem = Some(PlanProblem::NotInstalled);
    cursor.version = None;
    cursor.version_status = VersionStatus::Unknown;
    fake.set_plans(vec![plan(ProviderId::Claude), cursor]);
    fake.set_jobs(vec![
        row(10_000 - 60_000, job("abcd9999-0000", JobStatus::Running)),
        row(1, job("ffff0000-0000", JobStatus::Done)),
    ]);
    let running = {
        let mut job = job("abcd9999-0000", JobStatus::Running);
        job.prompt = "ship the login".to_string();
        job
    };
    fake.set_jobs(vec![
        row(10_000 - 60_000, running),
        row(1, job("ffff0000-0000", JobStatus::Done)),
    ]);
    assert_eq!(go(&["status"], &fake).await, 0);
    assert!(fake.inited());
    let out = fake.out();
    assert!(out.contains("claude · ready · CLI 2.1.292 · 80% left"));
    assert!(out.contains("cursor · not installed · The Cursor CLI is not installed."));
    assert!(out.contains("/proj"));
    assert!(out.contains("abcd9999"));
    assert!(out.contains("running"));
    assert!(!out.contains("ffff0000"));
}

#[tokio::test]
async fn a_job_prints_steps_as_they_finish_then_the_answer() {
    let fake = Fake::new();
    let mut tick = 0i64;
    fake.set_now(move || {
        tick += 1;
        if tick == 1 { 0 } else { 42_000 }
    });
    let mut running = job("job-1234", JobStatus::Running);
    running.job_type = JobType::Review;
    running.reason = Some("first choice".to_string());
    running.model = Some("sonnet".to_string());
    let returned = running.clone();
    let live = fake.clone();
    fake.set_submit(move |_, _| {
        let live = live.clone();
        let running = running.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            let mut stepped = running.clone();
            stepped.steps = Some(vec![
                step("s1", StepTool::Read, "Read src/app.ts", JobStepStatus::Done),
                step("s2", StepTool::Say, "hello", JobStepStatus::Done),
            ]);
            live.emit_job(stepped);
            let mut done = running;
            done.status = JobStatus::Done;
            done.output = "the answer".to_string();
            live.emit_job(done);
        });
        Ok(returned.clone())
    });
    assert_eq!(go(&["job", "fix it", "--type", "review"], &fake).await, 0);
    let out = fake.out();
    assert!(out.contains("claude · sonnet · review · read · first choice"));
    assert!(out.contains("READ   Read src/app.ts"));
    assert!(!out.contains("SAY"));
    assert!(out.contains("the answer"));
    assert!(out.contains("done in 42s"));
}

#[tokio::test]
async fn a_failed_job_exits_1() {
    let fake = Fake::new();
    fake.set_submit(|_, _| {
        let mut failed = job("job-9", JobStatus::Failed);
        failed.error = Some("no provider available".to_string());
        Ok(failed)
    });
    assert_eq!(go(&["job", "fix it"], &fake).await, 1);
    assert!(fake.out().contains("failed: no provider available"));
    assert_eq!(fake.err(), "");
}

#[tokio::test]
async fn json_is_one_object_and_quiet_is_only_the_answer() {
    let fake = Fake::new();
    fake.set_color(true);
    fake.set_submit(|_, _| {
        let mut done = job("job-9", JobStatus::Done);
        done.output = "just the answer".to_string();
        done.model = Some("sonnet".to_string());
        done.steps = Some(vec![step(
            "s1",
            StepTool::Read,
            "Read a.ts",
            JobStepStatus::Done,
        )]);
        Ok(done)
    });
    assert_eq!(go(&["job", "fix it", "--json"], &fake).await, 0);
    assert!(!fake.out().contains('\u{1b}'));
    let parsed: serde_json::Value = serde_json::from_str(fake.out().trim()).expect("json");
    assert_eq!(parsed["id"], "job-9");
    assert_eq!(parsed["status"], "done");
    assert_eq!(parsed["provider"], "claude");
    assert_eq!(parsed["model"], "sonnet");
    assert_eq!(parsed["access"], "read");
    assert_eq!(parsed["output"], "just the answer");
    assert!(parsed["error"].is_null());
    assert!(parsed["change"].is_null());

    let quiet = Fake::new();
    quiet.set_submit(|_, _| {
        let mut failed = job("job-8", JobStatus::Failed);
        failed.error = Some("boom".to_string());
        failed.output = "partial".to_string();
        Ok(failed)
    });
    assert_eq!(go(&["job", "fix it", "--quiet"], &quiet).await, 1);
    assert_eq!(quiet.out(), "partial\n");
    assert_eq!(quiet.err(), "failed: boom\n");
}

#[tokio::test]
async fn ctrl_c_cancels_the_job_and_a_second_one_exits_at_once() {
    let mut running = job("job-1234", JobStatus::Running);
    running.model = Some("sonnet".to_string());
    running.reason = Some("first choice".to_string());

    let once = Fake::new();
    let live = once.clone();
    let returned = running.clone();
    once.set_submit(move |_, _| {
        let live = live.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            live.fire_sigint();
        });
        Ok(returned.clone())
    });
    let failed_from = running.clone();
    once.set_cancel(move |fake, _| {
        let mut failed = failed_from.clone();
        failed.status = JobStatus::Failed;
        failed.error = Some("stopped".to_string());
        fake.emit_job(failed.clone());
        Ok(failed)
    });
    assert_eq!(go(&["job", "fix it"], &once).await, 130);
    assert!(once.out().contains("failed: stopped"));

    let twice = Fake::new();
    let live = twice.clone();
    let returned = running.clone();
    twice.set_submit(move |_, _| {
        let live = live.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            live.fire_sigint();
            live.fire_sigint();
        });
        Ok(returned.clone())
    });
    let still = running.clone();
    twice.set_cancel(move |_, _| Ok(still.clone()));
    assert_eq!(go(&["job", "fix it"], &twice).await, 130);
    assert_eq!(twice.exited(), vec![130]);
}

#[tokio::test]
async fn the_prompt_can_come_from_stdin() {
    let piped = Fake::new();
    piped.set_stdin_tty(false);
    piped.set_read("from stdin\n");
    let seen = Arc::new(Mutex::new(String::new()));
    let slot = Arc::clone(&seen);
    piped.set_submit(move |_, request| {
        *lock(&slot) = request.prompt;
        let mut done = job("job-1", JobStatus::Done);
        done.output = "ok".to_string();
        Ok(done)
    });
    assert_eq!(go(&["job"], &piped).await, 0);
    assert_eq!(lock(&seen).clone(), "from stdin");

    let tty = Fake::new();
    assert_eq!(go(&["job"], &tty).await, 2);
    assert!(tty.err().contains("Missing prompt."));
}

#[tokio::test]
async fn an_editing_job_reports_its_change() {
    let fake = Fake::new();
    let mut listed = change("abcd1234");
    listed.files = vec![
        ChangedFile {
            path: "a.ts".to_string(),
            insertions: Some(3),
            deletions: Some(1),
        },
        ChangedFile {
            path: "b.ts".to_string(),
            insertions: Some(1),
            deletions: Some(0),
        },
    ];
    listed.insertions = 4;
    listed.deletions = 1;
    fake.set_changes(vec![listed]);
    fake.set_ids(vec!["abcd1234".to_string()]);
    fake.set_submit(|_, _| {
        let mut done = job("job-1", JobStatus::Done);
        done.access = Some(JobAccess::Edit);
        done.edit = Some(true);
        done.change = Some("abcd1234".to_string());
        done.output = "edited".to_string();
        Ok(done)
    });
    assert_eq!(go(&["job", "edit it", "--access", "edit"], &fake).await, 0);
    assert!(fake.out().contains(&change_footer("abcd1234", 2, 4, 1)));
}

#[tokio::test]
async fn jobs_lists_newest_first_and_show_prints_every_step() {
    let older = {
        let mut job = job("aaaa1111-0000", JobStatus::Failed);
        job.prompt = "older job".to_string();
        job.error = Some("nope".to_string());
        job.output = "half".to_string();
        job.steps = Some(vec![step("s", StepTool::Say, "noted", JobStepStatus::Done)]);
        job
    };
    let newer = {
        let mut job = job("bbbb2222-0000", JobStatus::Done);
        job.prompt = "newer job".to_string();
        job
    };
    let rows = vec![
        row(10_000 - 3_600_000, newer),
        row(10_000 - 86_400_000, older),
    ];
    let listed = Fake::new();
    listed.set_jobs(rows.clone());
    assert_eq!(go(&["jobs", "--limit", "5"], &listed).await, 0);
    assert_eq!(listed.limit(), 5);
    let out = listed.out();
    assert!(out.find("bbbb2222").unwrap() < out.find("aaaa1111").unwrap());
    assert!(out.contains("1h"));

    let shown = Fake::new();
    shown.set_jobs(rows.clone());
    assert_eq!(go(&["show", "aaaa"], &shown).await, 0);
    assert!(shown.out().contains("SAY    noted"));
    assert!(shown.out().contains("half"));
    assert!(shown.out().contains("failed: nope"));

    let messy = Fake::new();
    messy.set_jobs(rows);
    assert_eq!(go(&["show", "xxxx"], &messy).await, 2);
    assert!(messy.err().contains("No job matches \"xxxx\"."));
}

#[tokio::test]
async fn changes_diff_merge_and_discard() {
    let mut listed_change = change("abcd1234");
    listed_change.name = Some("login".to_string());
    listed_change.files = vec![ChangedFile {
        path: "a.ts".to_string(),
        insertions: Some(2),
        deletions: Some(1),
    }];
    listed_change.insertions = 2;
    listed_change.deletions = 1;

    let listed = Fake::new();
    listed.set_changes(vec![listed_change.clone()]);
    listed.set_ids(vec!["abcd1234".to_string(), "eeee9999".to_string()]);
    listed.set_names(vec![ListedName {
        id: "eeee9999".to_string(),
        name: "empty".to_string(),
        created_at: 50,
    }]);
    assert_eq!(go(&["changes"], &listed).await, 0);
    assert!(listed.out().contains("login"));
    assert!(listed.out().contains("empty"));
    assert!(listed.out().contains("+2 −1"));

    let diff = Fake::new();
    diff.set_changes(vec![listed_change.clone()]);
    diff.set_ids(vec!["abcd1234".to_string()]);
    diff.set_diff(|_| Ok("diff --git a/a.ts\n".to_string()));
    assert_eq!(go(&["diff", "login"], &diff).await, 0);
    assert_eq!(diff.out(), "diff --git a/a.ts\n");

    let merged = Fake::new();
    merged.set_changes(vec![listed_change.clone()]);
    merged.set_ids(vec!["abcd1234".to_string()]);
    assert_eq!(go(&["merge", "abcd"], &merged).await, 0);
    assert!(merged.out().contains("applied to main"));
    assert_eq!(merged.deleted(), vec!["abcd1234".to_string()]);

    let blocked = Fake::new();
    blocked.set_stdin_tty(false);
    blocked.set_changes(vec![listed_change.clone()]);
    blocked.set_ids(vec!["abcd1234".to_string()]);
    let discarded = Arc::new(Mutex::new(false));
    let flag = Arc::clone(&discarded);
    blocked.set_discard(move |_| {
        *lock(&flag) = true;
        Ok(())
    });
    assert_eq!(go(&["discard", "abcd"], &blocked).await, 2);
    assert!(blocked.err().contains("Refusing to discard without --yes"));
    assert!(!*lock(&discarded));

    let kept = Fake::new();
    kept.set_changes(vec![listed_change.clone()]);
    kept.set_ids(vec!["abcd1234".to_string()]);
    kept.set_ask(|_| "n".to_string());
    assert_eq!(go(&["discard", "abcd"], &kept).await, 0);
    assert_eq!(kept.out(), "Kept abcd1234.\n");

    let gone = Fake::new();
    gone.set_changes(vec![listed_change]);
    gone.set_ids(vec!["abcd1234".to_string()]);
    gone.set_ask(|_| "should not be asked".to_string());
    let flag = Arc::clone(&discarded);
    *lock(&flag) = false;
    gone.set_discard(move |_| {
        *lock(&flag) = true;
        Ok(())
    });
    assert_eq!(go(&["discard", "abcd", "--yes"], &gone).await, 0);
    assert_eq!(gone.asked(), "");
    assert_eq!(gone.out(), "Discarded abcd1234.\n");
    assert!(*lock(&discarded));
}

#[tokio::test]
async fn merge_and_discard_refuse_a_busy_worktree() {
    let mut listed = change("abcd1234");
    listed.files = vec![ChangedFile {
        path: "a.ts".to_string(),
        insertions: Some(1),
        deletions: Some(0),
    }];
    listed.insertions = 1;
    let mut running = job("j1", JobStatus::Running);
    running.change = Some("abcd1234".to_string());
    let rows = vec![row(1, running)];

    let blocked_merge = Fake::new();
    blocked_merge.set_jobs(rows.clone());
    blocked_merge.set_changes(vec![listed.clone()]);
    blocked_merge.set_ids(vec!["abcd1234".to_string()]);
    let merged = Arc::new(Mutex::new(false));
    let flag = Arc::clone(&merged);
    blocked_merge.set_merge(move |_| {
        *lock(&flag) = true;
        Ok(MergeOutcome {
            ok: true,
            message: "applied".to_string(),
        })
    });
    assert_eq!(go(&["merge", "abcd"], &blocked_merge).await, 1);
    assert!(blocked_merge.err().contains(
        "A job is still running in this worktree. Stop it first, or wait for it to finish."
    ));
    assert!(!*lock(&merged));

    let blocked_discard = Fake::new();
    blocked_discard.set_jobs(rows);
    blocked_discard.set_changes(vec![listed]);
    blocked_discard.set_ids(vec!["abcd1234".to_string()]);
    let discarded = Arc::new(Mutex::new(false));
    let flag = Arc::clone(&discarded);
    blocked_discard.set_discard(move |_| {
        *lock(&flag) = true;
        Ok(())
    });
    assert_eq!(go(&["discard", "abcd", "--yes"], &blocked_discard).await, 1);
    assert!(blocked_discard.err().contains(
        "A job is still running in this worktree. Stop it first, or wait for it to finish."
    ));
    assert!(!*lock(&discarded));
}

#[tokio::test]
async fn pull_requests_and_worktrees() {
    let pr = Fake::new();
    assert_eq!(go(&["pr", "Add login", "--body", "please"], &pr).await, 0);
    assert_eq!(pr.out(), "https://example.test/pull/1\n");

    let created = Fake::new();
    assert_eq!(go(&["worktree", "create", "login"], &created).await, 0);
    assert_eq!(created.out(), "abcd1234  /wt/abcd1234\n");
    assert_eq!(created.names()[0].name, "login");

    let bare = Fake::new();
    bare.set_info(ProjectInfo {
        path: "/proj".to_string(),
        is_repo: false,
        branch: None,
    });
    let made = Arc::new(Mutex::new(false));
    let flag = Arc::clone(&made);
    bare.set_create(move |_| {
        *lock(&flag) = true;
        Ok(crate::drive::CreatedTree {
            id: "abcd1234".to_string(),
            path: "/wt/abcd1234".to_string(),
            branch: "orch/abcd1234".to_string(),
        })
    });
    assert_eq!(go(&["worktree", "create"], &bare).await, 1);
    assert!(!*lock(&made));
    assert!(bare.err().contains("not a git repository"));

    let path = Fake::new();
    path.set_ids(vec!["abcd1234".to_string()]);
    path.set_names(vec![ListedName {
        id: "abcd1234".to_string(),
        name: "login".to_string(),
        created_at: 1,
    }]);
    assert_eq!(go(&["worktree", "path", "login"], &path).await, 0);
    assert_eq!(path.out(), "/wt/abcd1234\n");

    let list = Fake::new();
    assert_eq!(go(&["worktree", "list"], &list).await, 0);
    assert_eq!(list.out(), "");
}

#[tokio::test]
async fn routine_list_and_run() {
    let routine = harness::routine("r1", "tests");
    let listed = Fake::new();
    listed.set_routines(vec![routine.clone()]);
    assert_eq!(go(&["routine", "list"], &listed).await, 0);
    assert!(listed.out().contains("tests"));
    assert!(listed.out().contains("review"));

    let ran = Fake::new();
    ran.set_routines(vec![routine]);
    let ran_id = Arc::new(Mutex::new(String::new()));
    let slot = Arc::clone(&ran_id);
    ran.set_run_routine(move |id| {
        *lock(&slot) = id.to_string();
        let mut done = job("job-1", JobStatus::Done);
        done.output = "routine answer".to_string();
        Ok(done)
    });
    assert_eq!(go(&["routine", "run", "tests", "--quiet"], &ran).await, 0);
    assert_eq!(lock(&ran_id).clone(), "r1");
    assert_eq!(ran.out(), "routine answer\n");

    let missing = Fake::new();
    assert_eq!(go(&["routine", "run", "missing"], &missing).await, 2);
    assert!(missing.err().contains("No routine named \"missing\"."));
    assert!(!missing.inited());
}

#[tokio::test]
async fn a_thrown_error_is_one_sentence_on_stderr() {
    let fake = Fake::new();
    fake.set_ids(vec!["abcd1234".to_string()]);
    fake.set_diff(|_| Err(crate::drive::DriveError::new("not a change worktree\nmore")));
    assert_eq!(go(&["diff", "abcd1234"], &fake).await, 1);
    assert_eq!(fake.err(), "not a change worktree\n");
}

fn sample_round(n: i64) -> GoalRound {
    GoalRound {
        n,
        summary: String::new(),
        jobs: Vec::new(),
        job_ids: Vec::new(),
        problems: Vec::new(),
        check: None,
    }
}

#[tokio::test]
async fn goal_help_and_a_goal_needs_text_and_a_round_count() {
    let goal_help = Fake::new();
    assert_eq!(go(&["goal", "--help"], &goal_help).await, 0);
    assert_eq!(
        goal_help.out(),
        "legate goal \"<text>\" [--check \"<command>\"] [--rounds N] [--yes]\n"
    );

    let goals_help = Fake::new();
    assert_eq!(go(&["goals", "-h"], &goals_help).await, 0);
    assert_eq!(goals_help.out(), "legate goals\n");

    let missing = Fake::new();
    assert_eq!(go(&["goal"], &missing).await, 2);
    assert!(missing.err().contains("Missing goal."));

    let rounds = Fake::new();
    assert_eq!(
        go(&["goal", "Ship login", "--rounds", "21"], &rounds).await,
        2
    );
    assert!(
        rounds
            .err()
            .contains("Rounds must be a whole number from 1 to 20.")
    );
}

#[tokio::test]
async fn goals_lists_newest_first_and_an_empty_list_hints() {
    let listed = Fake::new();
    let mut older = goal("old");
    older.status = GoalStatus::Planning;
    older.text = "Older goal".to_string();
    older.max_rounds = 3;
    older.updated_at = 1;
    let mut newer = goal("new");
    newer.status = GoalStatus::Done;
    newer.text = "Newer goal\nmore detail".to_string();
    newer.max_rounds = 5;
    newer.rounds = vec![sample_round(1), sample_round(2)];
    newer.updated_at = 2;
    listed.set_goals(vec![older, newer]);
    assert_eq!(go(&["goals"], &listed).await, 0);
    assert_eq!(
        listed.out(),
        "done      2/5  Newer goal\nplanning  0/3  Older goal\n"
    );
    assert!(!listed.out().contains("more detail"));

    let empty = Fake::new();
    assert_eq!(go(&["goals"], &empty).await, 0);
    assert_eq!(
        empty.err(),
        "No goals yet. Start one with: legate goal \"<what you want>\"\n"
    );
    assert_eq!(empty.out(), "");
}

#[tokio::test]
async fn a_goal_without_a_terminal_prints_the_proposal_and_stops() {
    let fake = Fake::new();
    fake.set_stdin_tty(false);
    let approved = Arc::new(Mutex::new(false));
    let flag = Arc::clone(&approved);
    fake.set_approve(move |_, _| {
        *lock(&flag) = true;
        Err(crate::drive::DriveError::new("unexpected approve"))
    });
    let live = fake.clone();
    fake.set_create_goal(move |_, input| {
        assert_eq!(input.project, "/proj");
        assert_eq!(input.text, "Ship login");
        assert_eq!(input.check.as_deref(), Some("pnpm test"));
        assert_eq!(input.max_rounds, 5.0);
        let planning = goal("g1");
        live.emit_goal(planning.clone());
        let mut waiting = goal("g1");
        waiting.status = GoalStatus::Waiting;
        waiting.proposal = Some(proposal(
            "Add the form",
            vec![planned(
                JobType::Refactor,
                JobAccess::Edit,
                "Write the form\nUse the field.",
            )],
        ));
        live.emit_goal(waiting);
        Ok(planning)
    });
    assert_eq!(
        go(&["goal", "Ship login", "--check", "pnpm test"], &fake).await,
        2
    );
    assert!(!*lock(&approved));
    let out = fake.out();
    assert!(out.contains("Planning round 1…"));
    assert!(out.contains("Round 1: Add the form"));
    assert!(out.contains("refactor · edit"));
    assert!(out.contains("Write the form"));
    assert!(out.contains("Use the field."));
    assert!(fake.err().contains("--yes"));
    assert!(fake.err().contains("app"));
}

#[tokio::test]
async fn yes_approves_a_round_then_prints_the_job_check_and_outcome() {
    let fake = Fake::new();
    fake.set_stdin_tty(false);
    fake.set_ask(|_| "asked".to_string());
    let mut running = job("job-1", JobStatus::Running);
    running.job_type = JobType::Refactor;
    running.access = Some(JobAccess::Edit);
    running.model = Some("sonnet".to_string());
    running.goal = Some("g1".to_string());
    running.prompt = "Write the form".to_string();
    let live = fake.clone();
    fake.set_create_goal(move |_, input| {
        assert_eq!(input.max_rounds, 4.0);
        let mut planning = goal("g1");
        planning.max_rounds = 4;
        live.emit_goal(planning.clone());
        let mut waiting = planning.clone();
        waiting.status = GoalStatus::Waiting;
        waiting.proposal = Some(proposal(
            "Add the form",
            vec![planned(
                JobType::Refactor,
                JobAccess::Edit,
                "Write the form",
            )],
        ));
        live.emit_goal(waiting);
        Ok(planning)
    });
    let jobs = fake.clone();
    let running_for_approve = running.clone();
    fake.set_approve(move |_, id| {
        assert_eq!(id, "g1");
        let mut stepped = running_for_approve.clone();
        let mut edited = step(
            "s1",
            StepTool::Edit,
            "Edited src/login.ts",
            JobStepStatus::Done,
        );
        edited.edit = Some(FileEdit {
            path: "src/login.ts".to_string(),
            added: 4,
            removed: 1,
        });
        stepped.steps = Some(vec![edited]);
        jobs.emit_job(stepped);
        let mut done_job = running_for_approve.clone();
        done_job.status = JobStatus::Done;
        done_job.output = "form added".to_string();
        jobs.emit_job(done_job);
        let mut done = goal("g1");
        done.status = GoalStatus::Done;
        done.max_rounds = 4;
        done.outcome =
            Some("The check passed after round 1. Review the goal in Changes.".to_string());
        done.rounds = vec![GoalRound {
            n: 1,
            summary: "Add the form".to_string(),
            jobs: vec![planned(
                JobType::Refactor,
                JobAccess::Edit,
                "Write the form",
            )],
            job_ids: vec!["job-1".to_string()],
            problems: Vec::new(),
            check: Some(GoalCheck {
                code: Some(0),
                output: "all green\n".to_string(),
                at: 1,
            }),
        }];
        jobs.emit_goal(done.clone());
        Ok(done)
    });
    assert_eq!(
        go(&["goal", "Ship login", "--yes", "--rounds", "4"], &fake).await,
        0
    );
    assert_eq!(fake.asked(), "");
    let out = fake.out();
    assert!(out.contains("Planning round 1…"));
    assert!(out.contains("Round 1: Add the form"));
    assert!(out.contains("claude · sonnet · refactor · edit"));
    assert!(out.contains("EDIT   Edited src/login.ts"));
    assert!(out.contains("form added"));
    assert!(out.contains("done in 0s"));
    assert!(out.contains("Check passed\nall green"));
    assert!(out.contains("The check passed after round 1. Review the goal in Changes."));
}

#[tokio::test]
async fn a_note_plans_again_n_stops_and_an_empty_answer_stops() {
    let noted = Fake::new();
    noted.set_ask(|_| "try the other file".to_string());
    let live = noted.clone();
    noted.set_create_goal(move |_, _| {
        let planning = goal("g1");
        live.emit_goal(planning.clone());
        let mut waiting = goal("g1");
        waiting.status = GoalStatus::Waiting;
        waiting.proposal = Some(proposal(
            "Add the form",
            vec![planned(
                JobType::Refactor,
                JobAccess::Edit,
                "Write the form",
            )],
        ));
        live.emit_goal(waiting);
        Ok(planning)
    });
    let answered = Arc::new(Mutex::new(String::new()));
    let slot = Arc::clone(&answered);
    let live = noted.clone();
    noted.set_answer(move |_, _id, text| {
        *lock(&slot) = text.to_string();
        let mut planning = goal("g1");
        planning.notes = vec![text.to_string()];
        live.emit_goal(planning.clone());
        let mut done = goal("g1");
        done.status = GoalStatus::Done;
        done.outcome = Some("The lead judges the goal met. Review it in Changes.".to_string());
        done.notes = vec![text.to_string()];
        live.emit_goal(done);
        Ok(planning)
    });
    assert_eq!(go(&["goal", "Ship login"], &noted).await, 0);
    assert_eq!(lock(&answered).clone(), "try the other file");
    assert!(
        noted
            .out()
            .contains("The lead judges the goal met. Review it in Changes.")
    );

    let stopped = Fake::new();
    stopped.set_ask(|_| "n".to_string());
    let live = stopped.clone();
    stopped.set_create_goal(move |_, _| {
        let planning = goal("g1");
        live.emit_goal(planning.clone());
        let mut waiting = goal("g1");
        waiting.status = GoalStatus::Waiting;
        waiting.proposal = Some(proposal(
            "Add the form",
            vec![planned(
                JobType::Refactor,
                JobAccess::Edit,
                "Write the form",
            )],
        ));
        live.emit_goal(waiting);
        Ok(planning)
    });
    let live = stopped.clone();
    stopped.set_stop(move |_, _| {
        let mut goal = goal("g1");
        goal.status = GoalStatus::Stopped;
        goal.outcome = Some("Stopped by you. The work so far is in Changes.".to_string());
        live.emit_goal(goal.clone());
        Ok(goal)
    });
    assert_eq!(go(&["goal", "Ship login"], &stopped).await, 1);
    assert_eq!(
        stopped.asked(),
        "Approve round 1? [y] yes  [n] stop  or type a note to plan again: "
    );
    assert!(
        stopped
            .out()
            .contains("Stopped by you. The work so far is in Changes.")
    );

    let question = Fake::new();
    question.set_ask(|_| String::new());
    let live = question.clone();
    question.set_create_goal(move |_, _| {
        let planning = goal("g1");
        live.emit_goal(planning.clone());
        let mut waiting = goal("g1");
        waiting.status = GoalStatus::Waiting;
        waiting.question = Some("Which file?".to_string());
        live.emit_goal(waiting);
        Ok(planning)
    });
    let live = question.clone();
    question.set_stop(move |_, _| {
        let mut goal = goal("g1");
        goal.status = GoalStatus::Stopped;
        goal.question = Some("Which file?".to_string());
        goal.outcome = Some("Stopped by you. The work so far is in Changes.".to_string());
        live.emit_goal(goal.clone());
        Ok(goal)
    });
    assert_eq!(go(&["goal", "Ship login"], &question).await, 1);
    assert!(question.out().contains("Which file?"));
    assert!(
        question
            .out()
            .contains("Stopped by you. The work so far is in Changes.")
    );
}
