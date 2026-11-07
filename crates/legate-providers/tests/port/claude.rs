use std::path::Path;

use legate_providers::{
    AgentEvent, BridgeInfo, ClaudeAdapter, InteractiveCommand, InteractiveOptions, JobAccess,
    LimitError, ProviderAdapter, RunOptions, StepPhase, StepTool,
};

use super::support::{self, Running};

const PROMPT: &str = "say \"hello\"\n100% |& done >";

fn adapter(args: &[&str]) -> ClaudeAdapter {
    ClaudeAdapter::new(Some(support::fake(args)))
}

fn no_note(spec: InteractiveCommand) -> InteractiveCommand {
    InteractiveCommand {
        command: spec.command,
        args: support::without_note(&spec.args),
        env: spec.env,
    }
}

async fn args_no_note(adapter: &ClaudeAdapter, cwd: &Path, opts: &RunOptions) -> Vec<String> {
    let mut running = Running {
        handle: adapter.run("job-1", "hello", cwd, opts),
    };
    support::without_note(&support::cli_args(&mut running.handle).await)
}

fn bridge(tools: &[&str]) -> BridgeInfo {
    BridgeInfo {
        url: "http://127.0.0.1:9/mcp".to_string(),
        token: "secret-token".to_string(),
        tools: tools.iter().map(|tool| (*tool).to_string()).collect(),
    }
}

#[test]
fn claude_plain_parses_init_text_usage_and_result() {
    let adapter = ClaudeAdapter::new(None);
    let events = support::events_of(|line| adapter.parse_events(line), "claude-plain.ndjson");
    assert_eq!(
        events,
        support::expect_events(
            r#"[
              {"kind":"init","sessionId":"00000000-0000-4000-8000-000000000103","model":"claude-opus-5-5"},
              {"kind":"text","text":"ok"},
              {"kind":"step","id":"msg_fixture0109","phase":"end","tool":"say","title":"ok","detail":"ok"},
              {"kind":"usage","utilization":0.41,"resetsAt":1791118800,"windows":[
                {"name":"five_hour","utilization":0.24,"resetsAt":1790848800},
                {"name":"seven_day","utilization":0.41,"resetsAt":1791118800}
              ]},
              {"kind":"result","ok":true,"text":"ok","sessionId":"00000000-0000-4000-8000-000000000103","durationMs":6515,"costUsd":0.1355346,"tokens":6}
            ]"#
        )
    );
}

#[test]
fn claude_tool_yields_ordered_read_steps_then_the_answer() {
    let adapter = ClaudeAdapter::new(None);
    let raw = support::fixture_text("claude-tool.ndjson");
    let tool_line = raw
        .split('\n')
        .find(|line| line.contains("\"tool_use\""))
        .expect("tool line");
    match adapter.parse_event(tool_line) {
        Some(AgentEvent::Step {
            phase: StepPhase::Start,
            tool: StepTool::Read,
            title,
            ..
        }) => assert_eq!(title, "Read /work/example/hello.txt"),
        other => panic!("tool line parsed as {other:?}"),
    }
    let events = support::events_of(|line| adapter.parse_events(line), "claude-tool.ndjson");
    assert_eq!(
        events,
        support::expect_events(
            r#"[
              {"kind":"init","sessionId":"00000000-0000-4000-8000-00000000010d","model":"claude-opus-5-5"},
              {"kind":"step","id":"toolu_fixture0118","phase":"start","tool":"read","title":"Read hello.txt"},
              {"kind":"usage","utilization":0.41,"resetsAt":1791118800,"windows":[
                {"name":"five_hour","utilization":0.25,"resetsAt":1790848800},
                {"name":"seven_day","utilization":0.41,"resetsAt":1791118800}
              ]},
              {"kind":"step","id":"toolu_fixture0118","phase":"end","tool":"read","title":"Read hello.txt","ok":true,"detail":"1\thello fixture\n2\t"},
              {"kind":"text","text":"It says \"hello fixture\"."},
              {"kind":"step","id":"msg_fixture011a","phase":"end","tool":"say","title":"It says \"hello fixture\".","detail":"It says \"hello fixture\"."},
              {"kind":"result","ok":true,"text":"It says \"hello fixture\".","sessionId":"00000000-0000-4000-8000-00000000010d","durationMs":5780,"costUsd":0.16763240000000001,"tokens":137}
            ]"#
        )
    );
}

#[test]
fn claude_edit_and_write_steps_count_lines_from_the_tool_input() {
    let adapter = ClaudeAdapter::new(None);
    adapter.parse_events(
        &serde_json::json!({
            "type": "system",
            "subtype": "init",
            "session_id": "sess-edit",
            "cwd": "/work/example",
            "model": "claude-opus-5-5"
        })
        .to_string(),
    );
    let edit_start = adapter.parse_events(
        &serde_json::json!({
            "type": "assistant",
            "session_id": "sess-edit",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_edit",
                    "name": "Edit",
                    "input": {
                        "file_path": "/work/example/src/app.ts",
                        "old_string": "a\n",
                        "new_string": "a\nb\nc\n"
                    }
                }]
            }
        })
        .to_string(),
    );
    assert_eq!(
        edit_start,
        support::expect_events(
            r#"[{"kind":"step","id":"toolu_edit","phase":"start","tool":"edit","title":"Edited src/app.ts","edit":{"path":"src/app.ts","added":3,"removed":1}}]"#
        )
    );
    let write_start = adapter.parse_events(
        &serde_json::json!({
            "type": "assistant",
            "session_id": "sess-edit",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "toolu_write",
                    "name": "Write",
                    "input": { "file_path": "/work/example/notes.txt", "content": "one\ntwo\n" }
                }]
            }
        })
        .to_string(),
    );
    match write_start.first() {
        Some(AgentEvent::Step {
            tool: StepTool::Edit,
            title,
            edit: Some(edit),
            ..
        }) => {
            assert_eq!(title, "Edited notes.txt");
            assert_eq!(edit.path, "notes.txt");
            assert_eq!(edit.added, 2);
            assert_eq!(edit.removed, 0);
        }
        other => panic!("{other:?}"),
    }
    let unknown = adapter.parse_events(
        &serde_json::json!({
            "type": "assistant",
            "session_id": "sess-edit",
            "message": {
                "content": [{ "type": "tool_use", "id": "toolu_x", "name": "MysteryTool", "input": {} }]
            }
        })
        .to_string(),
    );
    assert_eq!(
        unknown,
        support::expect_events(
            r#"[{"kind":"step","id":"toolu_x","phase":"start","tool":"other","title":"MysteryTool"}]"#
        )
    );
}

#[test]
fn claude_limit_synthetic_yields_a_limit_event() {
    let adapter = ClaudeAdapter::new(None);
    let events = support::events_of(
        |line| adapter.parse_events(line),
        "claude-limit.synthetic.ndjson",
    );
    assert_eq!(
        events,
        support::expect_events(
            r#"[
              {"kind":"init","sessionId":"00000000-0000-4000-8000-000000000001","model":"claude-opus-5-5"},
              {"kind":"limit","resetsAt":1790848800,"message":"rate limit rejected"},
              {"kind":"limit","message":"rate_limit"},
              {"kind":"limit","message":"5-hour limit reached - resets 2pm"}
            ]"#
        )
    );
    assert!(adapter.is_limit_error(LimitError::Event(&events[1])));
}

#[test]
fn parse_event_returns_none_for_garbage_without_throwing() {
    let adapter = ClaudeAdapter::new(None);
    for line in [
        "",
        "   ",
        "not-json",
        "{",
        "null",
        "[]",
        r#"{"type":"commands_changed"}"#,
    ] {
        assert!(adapter.parse_event(line).is_none(), "{line}");
    }
    let allowed = serde_json::json!({
        "type": "rate_limit_event",
        "rate_limit_info": { "status": "allowed" }
    })
    .to_string();
    assert!(adapter.parse_event(&allowed).is_none());
}

#[test]
fn is_limit_error_matches_limit_text_and_ignores_unrelated_errors() {
    let adapter = ClaudeAdapter::new(None);
    assert!(adapter.is_limit_error(LimitError::Text("Claude AI usage limit reached")));
    assert!(!adapter.is_limit_error(LimitError::Text("network timeout")));
}

#[tokio::test]
async fn run_replays_a_fixture_and_passes_the_prompt_on_stdin() {
    let dir = support::Temp::new("ao-claude");
    let fixture = support::path_text(&support::fixture("claude-plain.ndjson"));
    let adapter = adapter(&["replay", &fixture]);
    let mut running = Running {
        handle: adapter.run("job-1", PROMPT, dir.path(), &RunOptions::default()),
    };
    let (events, exit) = support::collect(&mut running.handle).await;
    assert_eq!(
        events,
        support::events_of(|line| adapter.parse_events(line), "claude-plain.ndjson")
    );
    assert_eq!(exit.code, Some(0));
    assert!(!exit.unreadable);
    assert_eq!(exit.stderr, PROMPT);
}

#[tokio::test]
async fn kill_stops_a_hung_cli() {
    let dir = support::Temp::new("ao-claude-kill");
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
    let root = support::Temp::new("ao-claude-opts");
    let adapter = adapter(&["argv"]);
    let url = "http://127.0.0.1:9/mcp";
    let token = "secret-token";
    let bridge = bridge(&["send_job", "get_result"]);
    let start = ["-p", "--output-format", "stream-json", "--verbose"];
    let strict = "--strict-mcp-config";
    let base = [
        start.as_slice(),
        &[
            "--disallowedTools",
            "Bash,PowerShell,Edit,Write,NotebookEdit",
            strict,
        ],
    ]
    .concat();
    let editing = [
        start.as_slice(),
        &["--disallowedTools", "Bash,PowerShell", strict],
    ]
    .concat();
    let expected_config = format!(
        r#"{{"mcpServers":{{"orchestrator":{{"type":"http","url":"{url}","headers":{{"Authorization":"Bearer {token}"}}}}}}}}"#
    );
    let mcp = |cwd: &Path| -> Vec<String> {
        vec![
            "--mcp-config".to_string(),
            support::path_text(&cwd.join("orchestrator-mcp.json")),
            "--strict-mcp-config".to_string(),
            "--allowedTools".to_string(),
            "mcp__orchestrator__send_job,mcp__orchestrator__get_result".to_string(),
            "--setting-sources".to_string(),
            "project".to_string(),
        ]
    };
    let owned = |args: &[&str]| {
        args.iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>()
    };

    let plain = root.join("plain");
    let resume_dir = root.join("resume");
    support::mkdir(&plain);
    support::mkdir(&resume_dir);
    assert_eq!(
        args_no_note(&adapter, &plain, &RunOptions::default()).await,
        owned(&base)
    );
    assert!(!plain.join("orchestrator-mcp.json").exists());

    assert_eq!(
        args_no_note(
            &adapter,
            &resume_dir,
            &RunOptions {
                resume: Some("sess-1".to_string()),
                ..RunOptions::default()
            }
        )
        .await,
        [owned(&base), owned(&["--resume", "sess-1"])].concat()
    );
    assert!(!resume_dir.join("orchestrator-mcp.json").exists());

    let bridged = root.join("bridged");
    assert_eq!(
        args_no_note(
            &adapter,
            &bridged,
            &RunOptions {
                bridge: Some(bridge.clone()),
                ..RunOptions::default()
            }
        )
        .await,
        [owned(&start), mcp(&bridged)].concat()
    );
    assert_eq!(
        support::read(&bridged.join("orchestrator-mcp.json")),
        expected_config
    );

    let both = root.join("both");
    assert_eq!(
        args_no_note(
            &adapter,
            &both,
            &RunOptions {
                resume: Some("sess-2".to_string()),
                bridge: Some(bridge.clone()),
                ..RunOptions::default()
            }
        )
        .await,
        [owned(&start), owned(&["--resume", "sess-2"]), mcp(&both)].concat()
    );
    assert_eq!(
        support::read(&both.join("orchestrator-mcp.json")),
        expected_config
    );

    let model_dir = root.join("model");
    support::mkdir(&model_dir);
    assert_eq!(
        args_no_note(
            &adapter,
            &model_dir,
            &RunOptions {
                model: Some("opus".to_string()),
                ..RunOptions::default()
            }
        )
        .await,
        [owned(&base), owned(&["--model", "opus"])].concat()
    );
    assert_eq!(
        args_no_note(
            &adapter,
            &plain,
            &RunOptions {
                model: Some(String::new()),
                ..RunOptions::default()
            }
        )
        .await,
        owned(&base)
    );

    let all = root.join("all");
    assert_eq!(
        args_no_note(
            &adapter,
            &all,
            &RunOptions {
                resume: Some("sess-3".to_string()),
                bridge: Some(bridge.clone()),
                model: Some("opus".to_string()),
                ..RunOptions::default()
            }
        )
        .await,
        [
            owned(&start),
            owned(&["--resume", "sess-3"]),
            mcp(&all),
            owned(&["--model", "opus"]),
        ]
        .concat()
    );

    let reading = root.join("reading");
    assert_eq!(
        args_no_note(
            &adapter,
            &reading,
            &RunOptions {
                bridge: Some(bridge.clone()),
                read_dirs: Some(vec!["/work/project".to_string()]),
                ..RunOptions::default()
            }
        )
        .await,
        [
            owned(&start),
            owned(&["--add-dir", "/work/project"]),
            vec![
                "--mcp-config".to_string(),
                support::path_text(&reading.join("orchestrator-mcp.json")),
                "--strict-mcp-config".to_string(),
                "--allowedTools".to_string(),
                "mcp__orchestrator__send_job,mcp__orchestrator__get_result,Read,Grep,Glob"
                    .to_string(),
                "--setting-sources".to_string(),
                "project".to_string(),
            ],
        ]
        .concat()
    );

    let edit_dir = root.join("edit");
    support::mkdir(&edit_dir);
    assert_eq!(
        args_no_note(
            &adapter,
            &edit_dir,
            &RunOptions {
                access: Some(JobAccess::Read),
                ..RunOptions::default()
            }
        )
        .await,
        owned(&base)
    );
    let editing_args = [
        owned(&editing),
        owned(&["--permission-mode", "acceptEdits"]),
    ]
    .concat();
    assert_eq!(
        args_no_note(
            &adapter,
            &edit_dir,
            &RunOptions {
                edit: Some(true),
                ..RunOptions::default()
            }
        )
        .await,
        editing_args
    );
    assert_eq!(
        args_no_note(
            &adapter,
            &edit_dir,
            &RunOptions {
                access: Some(JobAccess::Edit),
                ..RunOptions::default()
            }
        )
        .await,
        [
            owned(&editing),
            owned(&["--permission-mode", "acceptEdits"])
        ]
        .concat()
    );
    assert_eq!(
        args_no_note(
            &adapter,
            &edit_dir,
            &RunOptions {
                edit: Some(true),
                resume: Some("sess-e".to_string()),
                model: Some("opus".to_string()),
                ..RunOptions::default()
            }
        )
        .await,
        [
            owned(&editing),
            owned(&[
                "--permission-mode",
                "acceptEdits",
                "--resume",
                "sess-e",
                "--model",
                "opus"
            ]),
        ]
        .concat()
    );
    let full = [
        owned(&start),
        owned(&[strict, "--permission-mode", "bypassPermissions"]),
    ]
    .concat();
    assert_eq!(
        args_no_note(
            &adapter,
            &edit_dir,
            &RunOptions {
                access: Some(JobAccess::Full),
                ..RunOptions::default()
            }
        )
        .await,
        full.clone()
    );
    assert!(!full.iter().any(|arg| arg == "--disallowedTools"));
    assert_eq!(
        args_no_note(
            &adapter,
            &edit_dir,
            &RunOptions {
                access: Some(JobAccess::Full),
                resume: Some("sess-f".to_string()),
                model: Some("opus".to_string()),
                ..RunOptions::default()
            }
        )
        .await,
        [full, owned(&["--resume", "sess-f", "--model", "opus"])].concat()
    );
    assert_eq!(
        args_no_note(
            &adapter,
            &bridged,
            &RunOptions {
                access: Some(JobAccess::Full),
                bridge: Some(bridge),
                ..RunOptions::default()
            }
        )
        .await,
        [owned(&start), mcp(&bridged)].concat()
    );
}

#[tokio::test]
async fn list_models_offers_each_family_by_short_name_and_exact_version() {
    let adapter = ClaudeAdapter::new(Some(support::fake(&[])));
    let ids: Vec<_> = adapter
        .list_models()
        .await
        .into_iter()
        .map(|option| option.id)
        .collect();
    assert_eq!(
        ids,
        [
            "opus",
            "claude-opus-5-5",
            "claude-opus-5",
            "sonnet",
            "claude-sonnet-5-5",
            "claude-sonnet-5",
            "haiku",
            "claude-haiku-5-5",
            "fable",
            "claude-fable-5-1",
        ]
    );
    assert_eq!(
        ids.len(),
        ids.iter().collect::<std::collections::BTreeSet<_>>().len()
    );
}

#[test]
fn interactive_is_the_resolved_binary_with_no_p() {
    let plain = ClaudeAdapter::new(Some(legate_providers::Bin::new("claude.exe")));
    assert_eq!(
        no_note(plain.interactive(&InteractiveOptions::default())),
        InteractiveCommand {
            command: "claude.exe".to_string(),
            args: Vec::new(),
            env: None,
        }
    );
    assert_eq!(
        no_note(plain.interactive(&InteractiveOptions {
            model: Some("opus".to_string()),
            ..InteractiveOptions::default()
        })),
        InteractiveCommand {
            command: "claude.exe".to_string(),
            args: vec!["--model".to_string(), "opus".to_string()],
            env: None,
        }
    );
    let wrapped = ClaudeAdapter::new(Some(legate_providers::Bin::with_args(
        "node.exe",
        ["cli.js"],
    )));
    assert_eq!(
        no_note(wrapped.interactive(&InteractiveOptions::default())),
        InteractiveCommand {
            command: "node.exe".to_string(),
            args: vec!["cli.js".to_string()],
            env: None,
        }
    );
    assert_eq!(
        no_note(wrapped.interactive(&InteractiveOptions {
            model: Some("sonnet".to_string()),
            ..InteractiveOptions::default()
        })),
        InteractiveCommand {
            command: "node.exe".to_string(),
            args: vec![
                "cli.js".to_string(),
                "--model".to_string(),
                "sonnet".to_string()
            ],
            env: None,
        }
    );
    assert_eq!(
        no_note(plain.interactive(&InteractiveOptions {
            model: Some("opus".to_string()),
            prompt: Some("fix the login".to_string()),
            ..InteractiveOptions::default()
        })),
        InteractiveCommand {
            command: "claude.exe".to_string(),
            args: vec![
                "--model".to_string(),
                "opus".to_string(),
                "fix the login".to_string()
            ],
            env: None,
        }
    );
    assert_eq!(
        no_note(wrapped.interactive(&InteractiveOptions {
            session_id: Some("sess-1".to_string()),
            model: Some("sonnet".to_string()),
            prompt: Some("fix the login".to_string()),
            ..InteractiveOptions::default()
        })),
        InteractiveCommand {
            command: "node.exe".to_string(),
            args: vec![
                "cli.js".to_string(),
                "--session-id".to_string(),
                "sess-1".to_string(),
                "--model".to_string(),
                "sonnet".to_string(),
                "fix the login".to_string(),
            ],
            env: None,
        }
    );
    assert_eq!(
        no_note(plain.interactive(&InteractiveOptions {
            resume: Some("sess-2".to_string()),
            prompt: Some("fix the login".to_string()),
            ..InteractiveOptions::default()
        })),
        InteractiveCommand {
            command: "claude.exe".to_string(),
            args: vec!["--resume".to_string(), "sess-2".to_string()],
            env: None,
        }
    );
}

#[tokio::test]
async fn unknown_format_fixture_is_unreadable_and_has_no_result_event() {
    let dir = support::Temp::new("ao-claude-unknown");
    let fixture = support::path_text(&support::fixture("claude-unknown-format.synthetic.ndjson"));
    let adapter = adapter(&["replay", &fixture]);
    for line in support::fixture_text("claude-unknown-format.synthetic.ndjson").split('\n') {
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
    let dir = support::Temp::new("ao-claude-ver");
    let counter = support::path_text(&dir.join("count.txt"));
    let adapter = adapter(&["version", &counter, "2.1.292 (Claude Code)"]);
    assert!(adapter.is_installed().await);
    assert_eq!(adapter.version().await.as_deref(), Some("2.1.292"));
    assert_eq!(support::read(&dir.join("count.txt")), "1");
    assert!(adapter.is_installed().await);
    assert_eq!(adapter.version().await.as_deref(), Some("2.1.292"));
    assert_eq!(support::read(&dir.join("count.txt")), "2");
}

#[tokio::test]
async fn missing_cli_resolves_exit_and_ends_events() {
    let dir = support::Temp::new("ao-claude-missing");
    let missing = dir.join("missing-cli.exe");
    let adapter = ClaudeAdapter::new(Some(legate_providers::Bin::new(support::path_text(
        &missing,
    ))));
    let mut running = Running {
        handle: adapter.run("m", "hi", dir.path(), &RunOptions::default()),
    };
    let (events, exit) = support::collect(&mut running.handle).await;
    assert!(events.is_empty());
    assert_eq!(exit.code, None);
    assert!(!exit.stderr.is_empty());
}

#[tokio::test]
async fn skills_workers_get_worker_plugins_and_the_lead_gets_lead_plugins() {
    let root = support::Temp::new("ao-claude-plugins");
    let adapter = adapter(&["argv"]);
    adapter.set_plugins(legate_providers::AgentPlugins {
        lead: vec!["L1".to_string(), "L2".to_string()],
        worker: vec!["W1".to_string()],
    });
    let bridge = bridge(&["send_job"]);
    support::mkdir(&root.join("w"));
    let mut worker = Running {
        handle: adapter.run("job-p", "hello", &root.join("w"), &RunOptions::default()),
    };
    let worker_args = support::cli_args(&mut worker.handle).await;
    let tail = |args: &[String]| {
        let at = args
            .iter()
            .position(|arg| arg == "--plugin-dir")
            .expect("plugin");
        args[at..].to_vec()
    };
    assert_eq!(
        tail(&worker_args),
        vec!["--plugin-dir".to_string(), "W1".to_string()]
    );
    let mut lead = Running {
        handle: adapter.run(
            "job-p",
            "hello",
            &root.join("l"),
            &RunOptions {
                bridge: Some(bridge),
                ..RunOptions::default()
            },
        ),
    };
    let lead_args = support::cli_args(&mut lead.handle).await;
    assert_eq!(
        tail(&lead_args),
        vec![
            "--plugin-dir".to_string(),
            "L1".to_string(),
            "--plugin-dir".to_string(),
            "L2".to_string(),
        ]
    );
    let session = adapter
        .interactive(&InteractiveOptions {
            prompt: Some("fix it".to_string()),
            ..InteractiveOptions::default()
        })
        .args;
    assert_eq!(
        tail(&session),
        vec![
            "--plugin-dir".to_string(),
            "W1".to_string(),
            "fix it".to_string()
        ]
    );
}
