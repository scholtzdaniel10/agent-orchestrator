use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use legate_providers::{
    AgentEvent, AgentPlugins, Bin, BridgeInfo, CursorAdapter, FileEdit, GROK_DEFAULT_MODEL,
    InteractiveOptions, JobAccess, LimitError, ModelSource, ProviderAdapter, ProviderId,
    RunOptions, StepPhase, StepTool, cursor_env, cursor_version_dirs, node_platform,
    parse_cursor_models, resolve_cursor_bin, with_legate_note,
};
use serde_json::{Value, json};

use super::support::{self, Running};

const PROMPT: &str = "say \"hello\"\n100% |& done >";
const EDIT_JSON: &str = r#"{"permissions":{"allow":["Read(**)","Write(**)"],"deny":["Shell(*)"]}}"#;

fn adapter(args: &[&str]) -> CursorAdapter {
    CursorAdapter::new(Some(support::fake(args)))
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

fn bridge(tools: &[&str]) -> BridgeInfo {
    BridgeInfo {
        url: "http://127.0.0.1:9/mcp".to_string(),
        token: "secret-token".to_string(),
        tools: tools.iter().map(|tool| (*tool).to_string()).collect(),
    }
}

fn step_label(event: &AgentEvent) -> String {
    match event {
        AgentEvent::Step {
            phase, tool, title, ..
        } => {
            format!(
                "{}:{}:{title}",
                match phase {
                    StepPhase::Start => "start",
                    StepPhase::End => "end",
                },
                match tool {
                    StepTool::Say => "say",
                    StepTool::Think => "think",
                    StepTool::Read => "read",
                    StepTool::Edit => "edit",
                    StepTool::Shell => "shell",
                    StepTool::Search => "search",
                    StepTool::Web => "web",
                    StepTool::Mcp => "mcp",
                    StepTool::Other => "other",
                }
            )
        }
        _ => String::new(),
    }
}

fn kind_name(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::Init { .. } => "init",
        AgentEvent::Text { .. } => "text",
        AgentEvent::Step { .. } => "step",
        AgentEvent::Usage { .. } => "usage",
        AgentEvent::Limit { .. } => "limit",
        AgentEvent::Result { .. } => "result",
    }
}

fn make_version(versions: &Path, name: &str, files: &[&str]) -> PathBuf {
    let dir = versions.join(name);
    support::mkdir(&dir);
    for file in files {
        std::fs::write(dir.join(file), "").expect("version file");
    }
    dir
}

#[cfg(windows)]
struct EnvVar {
    key: &'static str,
    prev: Option<String>,
}

#[cfg(windows)]
impl EnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        Self { key, prev }
    }
}

#[cfg(windows)]
impl Drop for EnvVar {
    fn drop(&mut self) {
        unsafe {
            match &self.prev {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[test]
fn cursor_plain_parses_init_text_and_result() {
    let adapter = CursorAdapter::new(None);
    let events = support::events_of(|line| adapter.parse_events(line), "cursor-plain.ndjson");
    assert_eq!(
        events,
        support::expect_events(
            r#"[
              {"kind":"init","sessionId":"00000000-0000-4000-8000-00000000011c","model":"Grok 4.7 256K Extra High"},
              {"kind":"step","id":"think-1790832734780","phase":"end","tool":"think","title":"The user requested a","detail":"The user requested a"},
              {"kind":"step","id":"think-1790832734905","phase":"end","tool":"think","title":"reply containing exactly","detail":" reply containing exactly"},
              {"kind":"text","text":"ok"},
              {"kind":"step","id":"say-00000000-0000-4000-8000-00000000011c-ok","phase":"end","tool":"say","title":"ok","detail":"ok"},
              {"kind":"result","ok":true,"text":"ok","sessionId":"00000000-0000-4000-8000-00000000011c","durationMs":14224,"tokens":10406}
            ]"#
        )
    );
}

#[test]
fn cursor_tool_yields_ordered_steps_then_the_fixture_text() {
    let adapter = CursorAdapter::new(None);
    let events = support::events_of(|line| adapter.parse_events(line), "cursor-tool.ndjson");
    let steps: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, AgentEvent::Step { .. }))
        .map(step_label)
        .collect();
    assert_eq!(
        steps,
        [
            "end:think:Reading hello.txt to",
            "end:think:summarize its contents",
            "end:say:I'll read `hello.txt` and answer in five words or fewer.",
            "start:search:Searched for **/hello.txt",
            "end:search:Searched for **/hello.txt",
            "start:read:Read hello.txt",
            "end:read:Read hello.txt",
            "end:say:hello fixture",
        ]
    );
    assert_eq!(
        events.iter().map(kind_name).collect::<Vec<_>>(),
        [
            "init", "step", "step", "text", "step", "step", "step", "step", "step", "text", "step",
            "result"
        ]
    );
    match events.last() {
        Some(AgentEvent::Result { ok: true, text, .. }) => assert!(text.contains("hello fixture")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn cursor_edit_steps_use_reported_counts_when_present() {
    let adapter = CursorAdapter::new(None);
    adapter.parse_events(
        &json!({
            "type": "system",
            "subtype": "init",
            "session_id": "sess-c",
            "cwd": "/work/example",
            "model": "Grok"
        })
        .to_string(),
    );
    let started = adapter.parse_events(
        &json!({
            "type": "tool_call",
            "subtype": "started",
            "call_id": "call-edit",
            "session_id": "sess-c",
            "tool_call": { "editToolCall": { "args": { "path": "/work/example/src/app.ts" } } }
        })
        .to_string(),
    );
    match started.first() {
        Some(AgentEvent::Step {
            id,
            phase: StepPhase::Start,
            tool: StepTool::Edit,
            title,
            edit:
                Some(FileEdit {
                    path,
                    added: 0,
                    removed: 0,
                }),
            ..
        }) => {
            assert_eq!(id, "call-edit");
            assert_eq!(title, "Edited src/app.ts");
            assert_eq!(path, "src/app.ts");
        }
        other => panic!("{other:?}"),
    }
    let done = adapter.parse_events(
        &json!({
            "type": "tool_call",
            "subtype": "completed",
            "call_id": "call-edit",
            "session_id": "sess-c",
            "tool_call": {
                "editToolCall": {
                    "args": { "path": "/work/example/src/app.ts" },
                    "result": { "success": { "addedLines": 4, "removedLines": 1 } }
                }
            }
        })
        .to_string(),
    );
    match done.first() {
        Some(AgentEvent::Step {
            phase: StepPhase::End,
            tool: StepTool::Edit,
            ok: Some(true),
            edit:
                Some(FileEdit {
                    path,
                    added: 4,
                    removed: 1,
                }),
            ..
        }) => assert_eq!(path, "src/app.ts"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn cursor_limit_synthetic_yields_a_limit_event() {
    let adapter = CursorAdapter::new(None);
    let events = support::events_of(
        |line| adapter.parse_events(line),
        "cursor-limit.synthetic.ndjson",
    );
    assert_eq!(
        events,
        support::expect_events(
            r#"[
              {"kind":"init","sessionId":"00000000-0000-4000-8000-000000000002","model":"Grok 4.7 256K Extra High"},
              {"kind":"limit","message":"You've hit your usage limit for this plan."}
            ]"#
        )
    );
    assert!(adapter.is_limit_error(LimitError::Event(&events[1])));
}

#[test]
fn parse_event_returns_none_for_garbage_without_throwing() {
    let adapter = CursorAdapter::new(None);
    for line in [
        "",
        "   ",
        "not-json",
        "{",
        "null",
        "[]",
        r#"{"type":"thinking"}"#,
        r#"{"type":"tool_call"}"#,
        r#"{"type":"user"}"#,
    ] {
        assert!(adapter.parse_event(line).is_none(), "{line}");
    }
}

#[test]
fn is_limit_error_matches_limit_text_and_ignores_unrelated_errors() {
    let adapter = CursorAdapter::new(None);
    assert!(adapter.is_limit_error(LimitError::Text("Claude AI usage limit reached")));
    assert!(!adapter.is_limit_error(LimitError::Text("network timeout")));
}

#[tokio::test]
async fn run_replays_a_fixture_and_passes_the_prompt_on_stdin() {
    let dir = support::Temp::new("ao-cursor");
    let fixture = support::path_text(&support::fixture("cursor-plain.ndjson"));
    let adapter = adapter(&["replay", &fixture]);
    let mut running = Running {
        handle: adapter.run("job-1", PROMPT, dir.path(), &RunOptions::default()),
    };
    let (events, exit) = support::collect(&mut running.handle).await;
    assert_eq!(
        events,
        support::events_of(|line| adapter.parse_events(line), "cursor-plain.ndjson")
    );
    assert_eq!(exit.code, Some(0));
    assert!(!exit.unreadable);
    assert_eq!(exit.stderr, with_legate_note(PROMPT));
}

#[tokio::test]
async fn kill_stops_a_hung_cli() {
    let dir = support::Temp::new("ao-cursor-kill");
    let adapter = adapter(&["hang"]);
    let mut running = Running {
        handle: adapter.run("k", "x", dir.path(), &RunOptions::default()),
    };
    let first = support::timeout(running.handle.events.recv(), 3000).await;
    assert!(matches!(first, Some(AgentEvent::Init { .. })), "{first:?}");
    running.handle.kill();
    let exit = support::timeout(running.handle.exit(), 5000).await;
    assert_ne!(exit.code, Some(0));
    running.handle.kill();
}

#[tokio::test]
async fn run_args_cover_no_opts_resume_bridge_and_both() {
    let root = support::Temp::new("ao-cursor-opts");
    let adapter = adapter(&["argv"]);
    let url = "http://127.0.0.1:9/mcp";
    let token = "secret-token";
    let bridge = bridge(&["send_job", "get_result"]);
    let base = strings(&["-p", "--trust", "--output-format", "stream-json"]);
    let mcp_json = format!(
        r#"{{"mcpServers":{{"orchestrator":{{"url":"{url}","headers":{{"Authorization":"Bearer {token}"}}}}}}}}"#
    );
    let cli_json =
        r#"{"permissions":{"allow":["Mcp(orchestrator:*)"],"deny":["Shell(*)","Write(**)"]}}"#;
    let plain = root.join("plain");
    let resume_dir = root.join("resume");
    support::mkdir(&plain);
    support::mkdir(&resume_dir);

    let mut plain_run = Running {
        handle: adapter.run("job-1", "hello", &plain, &RunOptions::default()),
    };
    assert_eq!(support::cli_args(&mut plain_run.handle).await, base);
    assert!(!plain.join(".cursor").exists());

    let mut resume_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &resume_dir,
            &RunOptions {
                resume: Some("sess-1".to_string()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut resume_run.handle).await,
        [base.clone(), strings(&["--resume", "sess-1"])].concat()
    );
    assert!(!resume_dir.join(".cursor").exists());

    let bridged = root.join("bridged");
    let mut bridged_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &bridged,
            &RunOptions {
                bridge: Some(bridge.clone()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut bridged_run.handle).await,
        [base.clone(), strings(&["--approve-mcps"])].concat()
    );
    assert_eq!(
        support::read(&bridged.join(".cursor").join("mcp.json")),
        mcp_json
    );
    assert_eq!(
        support::read(&bridged.join(".cursor").join("cli.json")),
        cli_json
    );

    let both = root.join("both");
    let mut both_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &both,
            &RunOptions {
                resume: Some("sess-2".to_string()),
                bridge: Some(bridge.clone()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut both_run.handle).await,
        [
            base.clone(),
            strings(&["--resume", "sess-2", "--approve-mcps"])
        ]
        .concat()
    );
    assert_eq!(
        support::read(&both.join(".cursor").join("mcp.json")),
        mcp_json
    );
    assert_eq!(
        support::read(&both.join(".cursor").join("cli.json")),
        cli_json
    );

    let model_dir = root.join("model");
    support::mkdir(&model_dir);
    let mut model_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &model_dir,
            &RunOptions {
                model: Some("composer-2.5".to_string()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut model_run.handle).await,
        [base.clone(), strings(&["--model", "composer-2.5"])].concat()
    );
    let mut empty_model = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &plain,
            &RunOptions {
                model: Some(String::new()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(support::cli_args(&mut empty_model.handle).await, base);

    let all = root.join("all");
    let mut all_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &all,
            &RunOptions {
                resume: Some("sess-3".to_string()),
                bridge: Some(bridge),
                model: Some("composer-2.5".to_string()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut all_run.handle).await,
        [
            base,
            strings(&[
                "--resume",
                "sess-3",
                "--approve-mcps",
                "--model",
                "composer-2.5"
            ]),
        ]
        .concat()
    );
}

#[tokio::test]
async fn edit_writes_a_permission_file_before_spawn_and_removes_it_after_exit() {
    let root = support::Temp::new("ao-cursor-edit");
    let adapter = adapter(&["edit-report"]);
    let base = strings(&["-p", "--trust", "--output-format", "stream-json"]);

    let fresh = root.join("fresh");
    support::mkdir(&fresh);
    let created = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &fresh,
            &RunOptions {
                edit: Some(true),
                ..RunOptions::default()
            },
        ),
    };
    let cli = fresh.join(".cursor").join("cli.json");
    assert!(cli.exists());
    assert_eq!(support::read(&cli), EDIT_JSON);
    let created_exit = created.handle.exit().await;
    assert_eq!(created_exit.code, Some(0), "{}", created_exit.stderr);
    let created_report: Value =
        serde_json::from_str(&created_exit.stderr).expect(&created_exit.stderr);
    assert_eq!(created_report["argv"], json!(base));
    assert!(
        created_report["argv"]
            .as_array()
            .unwrap()
            .iter()
            .all(|arg| arg != "--force")
    );
    assert_eq!(created_report["body"], EDIT_JSON);
    assert!(!cli.exists());
    assert!(!fresh.join(".cursor").exists());

    let kept = root.join("kept");
    support::mkdir(&kept.join(".cursor"));
    let kept_cli = kept.join(".cursor").join("cli.json");
    let own = r#"{"permissions":{"allow":["Read(**)"]}}"#;
    std::fs::write(&kept_cli, own).unwrap();
    let kept_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &kept,
            &RunOptions {
                edit: Some(true),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(support::read(&kept_cli), own);
    let kept_exit = kept_run.handle.exit().await;
    let kept_report: Value = serde_json::from_str(&kept_exit.stderr).expect(&kept_exit.stderr);
    assert_eq!(kept_report["argv"], json!(base));
    assert_eq!(kept_report["body"], own);
    assert_eq!(support::read(&kept_cli), own);
    assert!(kept.join(".cursor").exists());

    let shared = root.join("shared");
    support::mkdir(&shared.join(".cursor"));
    std::fs::write(shared.join(".cursor").join("other.txt"), "keep").unwrap();
    let shared_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &shared,
            &RunOptions {
                edit: Some(true),
                ..RunOptions::default()
            },
        ),
    };
    assert!(shared.join(".cursor").join("cli.json").exists());
    let _ = shared_run.handle.exit().await;
    assert!(!shared.join(".cursor").join("cli.json").exists());
    assert_eq!(
        support::read(&shared.join(".cursor").join("other.txt")),
        "keep"
    );

    let bridged = root.join("bridged");
    let bridge = bridge(&["send_job"]);
    let bridge_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &bridged,
            &RunOptions {
                edit: Some(true),
                bridge: Some(bridge),
                ..RunOptions::default()
            },
        ),
    };
    let bridge_cli = bridged.join(".cursor").join("cli.json");
    let bridge_body = support::read(&bridge_cli);
    assert_ne!(bridge_body, EDIT_JSON);
    let bridge_exit = bridge_run.handle.exit().await;
    let bridge_report: Value =
        serde_json::from_str(&bridge_exit.stderr).expect(&bridge_exit.stderr);
    let argv = bridge_report["argv"].as_array().unwrap();
    assert!(argv.iter().any(|arg| arg == "--approve-mcps"));
    assert!(argv.iter().all(|arg| arg != "--force"));
    assert_eq!(support::read(&bridge_cli), bridge_body);
}

#[tokio::test]
async fn full_adds_force_and_does_not_write_a_permission_file() {
    let root = support::Temp::new("ao-cursor-full");
    let adapter = adapter(&["full-report"]);
    let base = strings(&["-p", "--trust", "--output-format", "stream-json"]);

    let fresh = root.join("fresh");
    support::mkdir(&fresh);
    let created = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &fresh,
            &RunOptions {
                access: Some(JobAccess::Full),
                ..RunOptions::default()
            },
        ),
    };
    assert!(!fresh.join(".cursor").join("cli.json").exists());
    let created_exit = created.handle.exit().await;
    assert_eq!(created_exit.code, Some(0), "{}", created_exit.stderr);
    let created_report: Value =
        serde_json::from_str(&created_exit.stderr).expect(&created_exit.stderr);
    assert_eq!(
        created_report["argv"],
        json!([base.clone(), strings(&["--force"])].concat())
    );
    assert_eq!(created_report["hasCli"], false);
    assert!(!fresh.join(".cursor").exists());

    let read_dir = root.join("read");
    support::mkdir(&read_dir);
    let read_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &read_dir,
            &RunOptions {
                access: Some(JobAccess::Read),
                ..RunOptions::default()
            },
        ),
    };
    let read_exit = read_run.handle.exit().await;
    let read_report: Value = serde_json::from_str(&read_exit.stderr).expect(&read_exit.stderr);
    assert_eq!(read_report["argv"], json!(base));
    assert_eq!(read_report["hasCli"], false);

    let edit_dir = root.join("edit");
    support::mkdir(&edit_dir);
    let edit_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &edit_dir,
            &RunOptions {
                access: Some(JobAccess::Edit),
                ..RunOptions::default()
            },
        ),
    };
    assert!(edit_dir.join(".cursor").join("cli.json").exists());
    let edit_exit = edit_run.handle.exit().await;
    let edit_report: Value = serde_json::from_str(&edit_exit.stderr).expect(&edit_exit.stderr);
    assert_eq!(edit_report["argv"], json!(base));
    assert_eq!(edit_report["hasCli"], true);
    assert!(
        edit_report["argv"]
            .as_array()
            .unwrap()
            .iter()
            .all(|arg| arg != "--force")
    );
    assert!(!edit_dir.join(".cursor").exists());

    let bridged = root.join("bridged");
    let bridge_run = Running {
        handle: adapter.run(
            "job-1",
            "hello",
            &bridged,
            &RunOptions {
                access: Some(JobAccess::Full),
                bridge: Some(bridge(&["send_job"])),
                ..RunOptions::default()
            },
        ),
    };
    let bridge_exit = bridge_run.handle.exit().await;
    let bridge_report: Value =
        serde_json::from_str(&bridge_exit.stderr).expect(&bridge_exit.stderr);
    let argv = bridge_report["argv"].as_array().unwrap();
    assert!(argv.iter().any(|arg| arg == "--approve-mcps"));
    assert!(argv.iter().all(|arg| arg != "--force"));
}

#[tokio::test]
async fn unknown_format_fixture_is_unreadable_and_has_no_result_event() {
    let dir = support::Temp::new("ao-cursor-unknown");
    let fixture = support::path_text(&support::fixture("cursor-unknown-format.synthetic.ndjson"));
    let adapter = adapter(&["replay", &fixture]);
    for line in support::fixture_text("cursor-unknown-format.synthetic.ndjson").split('\n') {
        if !line.trim().is_empty() {
            assert!(adapter.parse_event(line.trim()).is_none(), "{line}");
        }
    }
    let mut running = Running {
        handle: adapter.run("u", "hi", dir.path(), &RunOptions::default()),
    };
    let (events, exit) = support::collect(&mut running.handle).await;
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, AgentEvent::Result { .. }))
    );
    assert_eq!(exit.code, Some(0));
    assert!(exit.unreadable);
}

#[tokio::test]
async fn is_installed_and_version_share_one_version_call() {
    let dir = support::Temp::new("ao-cursor-ver");
    let counter = support::path_text(&dir.join("count.txt"));
    let adapter = adapter(&["version", &counter, "2026.10.01-e373342"]);
    assert!(adapter.is_installed().await);
    assert_eq!(
        adapter.version().await.as_deref(),
        Some("2026.10.01-e373342")
    );
    assert_eq!(support::read(&dir.join("count.txt")), "1");
    assert!(adapter.is_installed().await);
    assert_eq!(
        adapter.version().await.as_deref(),
        Some("2026.10.01-e373342")
    );
    assert_eq!(support::read(&dir.join("count.txt")), "2");
}

#[tokio::test]
async fn missing_cli_resolves_exit_and_ends_events() {
    let dir = support::Temp::new("ao-cursor-missing");
    let missing = dir.join("missing-cli.exe");
    let adapter = CursorAdapter::new(Some(Bin::new(support::path_text(&missing))));
    let mut running = Running {
        handle: adapter.run("m", "hi", dir.path(), &RunOptions::default()),
    };
    let (events, exit) = support::collect(&mut running.handle).await;
    assert!(events.is_empty());
    assert_eq!(exit.code, None);
    assert!(!exit.stderr.is_empty());
}

#[test]
fn parse_cursor_models_keeps_id_label_lines_and_drops_the_rest() {
    let zw = "\u{200B}\u{200C}\u{200D}\u{FEFF}";
    let stdout = [
        "Available models",
        "",
        &format!("{zw}auto{zw} - Auto{zw}"),
        "composer-2.5 - Composer 2.5 - fast",
        "not a model line",
    ]
    .join("\r\n");
    assert_eq!(
        parse_cursor_models(&stdout),
        vec![
            legate_providers::ModelOption {
                id: "auto".to_string(),
                label: "Auto".to_string(),
            },
            legate_providers::ModelOption {
                id: "composer-2.5".to_string(),
                label: "Composer 2.5 - fast".to_string(),
            },
        ]
    );
    assert!(parse_cursor_models("").is_empty());
    assert!(parse_cursor_models("\n\n").is_empty());
}

#[tokio::test]
async fn list_models_parses_caches_success_and_does_not_cache_failure() {
    let root = support::Temp::new("ao-cursor-models");
    let counter = support::path_text(&root.join("count.txt"));
    let ok = CursorAdapter::new(Some(support::fake(&["models-ok", &counter])));
    let first = ok.list_models().await;
    let second = ok.list_models().await;
    assert_eq!(
        first,
        vec![
            legate_providers::ModelOption {
                id: "auto".to_string(),
                label: "Auto".to_string(),
            },
            legate_providers::ModelOption {
                id: "composer-2.5".to_string(),
                label: "Composer 2.5".to_string(),
            },
        ]
    );
    assert_eq!(second, first);
    assert_eq!(support::read(&root.join("count.txt")), "1");

    let fail_count = support::path_text(&root.join("fail.txt"));
    let failing = CursorAdapter::new(Some(support::fake(&["models-fail", &fail_count])));
    assert!(failing.list_models().await.is_empty());
    assert_eq!(
        failing.list_models().await,
        vec![legate_providers::ModelOption {
            id: "auto".to_string(),
            label: "Auto".to_string(),
        }]
    );
    assert_eq!(support::read(&root.join("fail.txt")), "2");

    let empty_count = support::path_text(&root.join("empty.txt"));
    let empty = CursorAdapter::new(Some(support::fake(&["models-empty", &empty_count])));
    assert!(empty.list_models().await.is_empty());
    assert_eq!(
        empty.list_models().await,
        vec![legate_providers::ModelOption {
            id: "haiku".to_string(),
            label: "Haiku".to_string(),
        }]
    );
    assert_eq!(support::read(&root.join("empty.txt")), "2");
}

#[test]
fn interactive_is_the_resolved_node_entry_with_no_p() {
    let _lock = support::env_lock();
    let source: BTreeMap<_, _> = std::env::vars().collect();
    let env = cursor_env(node_platform(), &source);
    let plain = CursorAdapter::new(Some(Bin::new("agent")));
    assert_eq!(
        plain.interactive(&InteractiveOptions::default()),
        legate_providers::InteractiveCommand {
            command: "agent".to_string(),
            args: Vec::new(),
            env: env.clone(),
        }
    );
    assert_eq!(
        plain.interactive(&InteractiveOptions {
            model: Some("composer-2.5".to_string()),
            ..InteractiveOptions::default()
        }),
        legate_providers::InteractiveCommand {
            command: "agent".to_string(),
            args: strings(&["--model", "composer-2.5"]),
            env: env.clone(),
        }
    );
    let wrapped = CursorAdapter::new(Some(Bin::with_args("node.exe", ["index.js"])));
    assert_eq!(
        wrapped.interactive(&InteractiveOptions::default()),
        legate_providers::InteractiveCommand {
            command: "node.exe".to_string(),
            args: strings(&["index.js"]),
            env: env.clone(),
        }
    );
    assert_eq!(
        wrapped.interactive(&InteractiveOptions {
            model: Some("fast".to_string()),
            ..InteractiveOptions::default()
        }),
        legate_providers::InteractiveCommand {
            command: "node.exe".to_string(),
            args: strings(&["index.js", "--model", "fast"]),
            env: env.clone(),
        }
    );
    assert_eq!(
        plain.interactive(&InteractiveOptions {
            model: Some("composer-2.5".to_string()),
            prompt: Some("fix the login".to_string()),
            ..InteractiveOptions::default()
        }),
        legate_providers::InteractiveCommand {
            command: "agent".to_string(),
            args: vec![
                "--model".to_string(),
                "composer-2.5".to_string(),
                "--trust".to_string(),
                with_legate_note("fix the login"),
            ],
            env: env.clone(),
        }
    );
    assert_eq!(
        wrapped.interactive(&InteractiveOptions {
            session_id: Some("chat-1".to_string()),
            model: Some("fast".to_string()),
            prompt: Some("fix the login".to_string()),
            ..InteractiveOptions::default()
        }),
        legate_providers::InteractiveCommand {
            command: "node.exe".to_string(),
            args: vec![
                "index.js".to_string(),
                "--resume".to_string(),
                "chat-1".to_string(),
                "--model".to_string(),
                "fast".to_string(),
                "--trust".to_string(),
                with_legate_note("fix the login"),
            ],
            env: env.clone(),
        }
    );
    assert_eq!(
        plain.interactive(&InteractiveOptions {
            resume: Some("chat-2".to_string()),
            prompt: Some("fix the login".to_string()),
            ..InteractiveOptions::default()
        }),
        legate_providers::InteractiveCommand {
            command: "agent".to_string(),
            args: strings(&["--resume", "chat-2"]),
            env,
        }
    );
}

#[test]
fn cursor_env_drops_the_git_bash_markers_on_windows_only() {
    let mut source = BTreeMap::new();
    source.insert("PATH".to_string(), "p".to_string());
    source.insert("SHELL".to_string(), "/bin/bash.exe".to_string());
    source.insert("MSYSTEM".to_string(), "MINGW64".to_string());
    source.insert("Shell".to_string(), "x".to_string());
    source.insert("TERM".to_string(), "t".to_string());
    let mut expect = BTreeMap::new();
    expect.insert("PATH".to_string(), "p".to_string());
    expect.insert("TERM".to_string(), "t".to_string());
    assert_eq!(cursor_env("win32", &source), Some(expect));
    assert_eq!(cursor_env("linux", &source), None);
    assert_eq!(cursor_env("darwin", &source), None);
}

#[cfg(windows)]
#[tokio::test]
async fn run_does_not_pass_shell_or_msystem_on_windows() {
    let dir = support::Temp::new("ao-cursor-env");
    let adapter = adapter(&["env"]);
    // `run` copies the environment at spawn, so the guard can drop before we wait.
    let mut running = {
        let _lock = support::env_lock();
        let _shell = EnvVar::set("SHELL", "/bin/bash.exe");
        let _msystem = EnvVar::set("MSYSTEM", "MINGW64");
        Running {
            handle: adapter.run("e", "x", dir.path(), &RunOptions::default()),
        }
    };
    let (_events, exit) = support::collect(&mut running.handle).await;
    let parsed: Value = serde_json::from_str(&exit.stderr).unwrap_or_else(|err| {
        panic!("env json ({err}): {}", exit.stderr);
    });
    assert_eq!(
        parsed,
        json!({"shell": null, "msystem": null, "path": "string"})
    );
}

#[tokio::test]
async fn grok_passes_a_grok_model_and_ignores_a_non_grok_one() {
    let root = support::Temp::new("ao-grok-model");
    let script = "argv.mjs";
    let bin = Bin::with_args(env!("CARGO_BIN_EXE_fake-cli"), [script]);
    let base = strings(&["-p", "--trust", "--output-format", "stream-json"]);
    let dir = root.join("run");
    support::mkdir(&dir);
    let grok = CursorAdapter::grok(Some(bin.clone()), None);
    let mut plain = Running {
        handle: grok.run("job-1", "hello", &dir, &RunOptions::default()),
    };
    assert_eq!(
        support::cli_args(&mut plain.handle).await,
        [base.clone(), strings(&["--model", GROK_DEFAULT_MODEL])].concat()
    );
    let mut resumed = Running {
        handle: grok.run(
            "job-1",
            "hello",
            &dir,
            &RunOptions {
                resume: Some("sess-1".to_string()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut resumed.handle).await,
        [
            base.clone(),
            strings(&["--resume", "sess-1", "--model", GROK_DEFAULT_MODEL]),
        ]
        .concat()
    );
    assert_eq!(
        grok.interactive(&InteractiveOptions::default()).args,
        strings(&[script, "--model", GROK_DEFAULT_MODEL])
    );
    assert_eq!(
        grok.interactive(&InteractiveOptions {
            resume: Some("sess-9".to_string()),
            ..InteractiveOptions::default()
        })
        .args,
        strings(&[script, "--resume", "sess-9", "--model", GROK_DEFAULT_MODEL])
    );

    let ignored = CursorAdapter::grok(
        Some(bin.clone()),
        Some(Arc::new(|| Some("composer-2.5".to_string())) as ModelSource),
    );
    let mut ignored_run = Running {
        handle: ignored.run(
            "job-1",
            "hello",
            &dir,
            &RunOptions {
                model: Some("sonnet".to_string()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut ignored_run.handle).await,
        [base.clone(), strings(&["--model", GROK_DEFAULT_MODEL])].concat()
    );
    let from_settings = CursorAdapter::grok(
        Some(bin.clone()),
        Some(Arc::new(|| Some("grok-4".to_string())) as ModelSource),
    );
    let mut settings_run = Running {
        handle: from_settings.run(
            "job-1",
            "hello",
            &dir,
            &RunOptions {
                model: Some("sonnet".to_string()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut settings_run.handle).await,
        [base.clone(), strings(&["--model", "grok-4"])].concat()
    );
    let mut kept = Running {
        handle: from_settings.run(
            "job-1",
            "hello",
            &dir,
            &RunOptions {
                model: Some("grok-3".to_string()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut kept.handle).await,
        [base.clone(), strings(&["--model", "grok-3"])].concat()
    );

    let cursor = CursorAdapter::new(Some(bin));
    assert_eq!(cursor.id(), ProviderId::Cursor);
    let mut cursor_plain = Running {
        handle: cursor.run("job-1", "hello", &dir, &RunOptions::default()),
    };
    assert_eq!(support::cli_args(&mut cursor_plain.handle).await, base);
    assert_eq!(
        cursor.interactive(&InteractiveOptions::default()).args,
        strings(&[script])
    );
    let mut cursor_model = Running {
        handle: cursor.run(
            "job-1",
            "hello",
            &dir,
            &RunOptions {
                model: Some("composer-2.5".to_string()),
                ..RunOptions::default()
            },
        ),
    };
    assert_eq!(
        support::cli_args(&mut cursor_model.handle).await,
        [base, strings(&["--model", "composer-2.5"])].concat()
    );
}

#[tokio::test]
async fn grok_model_list_is_the_cursor_list_filtered_to_grok_ids() {
    let grok = CursorAdapter::grok(Some(support::fake(&["models-grok"])), None);
    assert_eq!(
        grok.list_models().await,
        vec![legate_providers::ModelOption {
            id: "grok-4.7-high".to_string(),
            label: "Grok 4.7".to_string(),
        }]
    );
    let cursor = adapter(&["models-grok"]);
    assert_eq!(
        cursor.list_models().await,
        vec![
            legate_providers::ModelOption {
                id: "grok-4.7-high".to_string(),
                label: "Grok 4.7".to_string(),
            },
            legate_providers::ModelOption {
                id: "composer-2.5".to_string(),
                label: "Composer".to_string(),
            },
        ]
    );
}

#[test]
fn skills_workers_get_worker_plugins_and_the_lead_gets_lead_plugins() {
    let adapter = CursorAdapter::new(Some(Bin::new("agent")));
    adapter.set_plugins(AgentPlugins {
        lead: vec!["L1".to_string()],
        worker: vec!["W1".to_string(), "W2".to_string()],
    });
    assert_eq!(
        adapter.interactive(&InteractiveOptions::default()).args,
        strings(&["--plugin-dir", "W1", "--plugin-dir", "W2"])
    );
    assert_eq!(
        adapter
            .interactive(&InteractiveOptions {
                prompt: Some("go".to_string()),
                ..InteractiveOptions::default()
            })
            .args,
        vec![
            "--plugin-dir".to_string(),
            "W1".to_string(),
            "--plugin-dir".to_string(),
            "W2".to_string(),
            "--trust".to_string(),
            with_legate_note("go"),
        ]
    );
}

#[test]
fn resolve_cursor_bin_finds_the_cli_the_editor_keeps() {
    let root = support::Temp::new("ao-cursor-bin");
    let local = root.join("local");
    let app = root.join("roaming");
    let dirs = cursor_version_dirs(Some(&local), Some(&app));
    assert_eq!(dirs[0], local.join("cursor-agent").join("versions"));
    let editor = &dirs[1];
    let dir = make_version(editor, "2026.10.01-e373342", &["node.exe", "index.js"]);
    make_version(editor, "2026.09.28-64d2043", &["node.exe", "index.js"]);
    let bin = resolve_cursor_bin(Some(&local), Some(&app), "win32");
    assert_eq!(bin.command, support::path_text(&dir.join("node.exe")));
    assert_eq!(bin.args, vec![support::path_text(&dir.join("index.js"))]);
}

#[test]
fn resolve_cursor_bin_prefers_the_standalone_install_and_skips_incomplete_versions() {
    let root = support::Temp::new("ao-cursor-bin");
    let local = root.join("local");
    let app = root.join("roaming");
    let dirs = cursor_version_dirs(Some(&local), Some(&app));
    make_version(&dirs[1], "2026.12.01-newer", &["node.exe", "index.js"]);
    make_version(&dirs[0], "2026.11.01-partial", &["node.exe"]);
    let dir = make_version(&dirs[0], "2026.10.01-whole", &["node.exe", "index.js"]);
    let bin = resolve_cursor_bin(Some(&local), Some(&app), "win32");
    assert_eq!(bin.command, support::path_text(&dir.join("node.exe")));
    assert_eq!(bin.args, vec![support::path_text(&dir.join("index.js"))]);
}

#[test]
fn resolve_cursor_bin_falls_back_to_agent_on_path() {
    let root = support::Temp::new("ao-cursor-bin");
    let local = root.join("local");
    let app = root.join("roaming");
    let missing = resolve_cursor_bin(Some(&local), Some(&app), "win32");
    assert_eq!(missing.command, "agent");
    assert!(missing.args.is_empty());
    let empty = resolve_cursor_bin(None, None, "win32");
    assert_eq!(empty.command, "agent");
    assert!(empty.args.is_empty());
    let dirs = cursor_version_dirs(Some(&local), Some(&app));
    make_version(&dirs[1], "2026.10.01-e373342", &["node.exe", "index.js"]);
    let linux = resolve_cursor_bin(Some(&local), Some(&app), "linux");
    assert_eq!(linux.command, "agent");
    assert!(linux.args.is_empty());
}
