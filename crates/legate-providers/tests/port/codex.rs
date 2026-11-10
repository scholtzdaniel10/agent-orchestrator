use super::support;
use legate_providers::{
    AgentEvent, BridgeInfo, CodexAdapter, JobAccess, ProviderAdapter, RunOptions,
};

#[tokio::test]
async fn codex_spawns_the_official_protocol_and_forwards_stdin() {
    let temp = support::Temp::new("codex-protocol");
    let path = temp.join("events.ndjson");
    std::fs::write(&path, concat!(
        "{\"type\":\"thread.started\",\"thread_id\":\"codex-test\"}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"m1\",\"type\":\"agent_message\",\"text\":\"ok\"}}\n",
        "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}\n"
    )).unwrap();
    let file = support::path_text(&path);
    let adapter = CodexAdapter::new(Some(support::fake(&["replay", &file])));
    let mut run = adapter.run(
        "job",
        "reply exactly ok",
        temp.path(),
        &RunOptions::default(),
    );
    let (events, exit) = support::collect(&mut run).await;
    assert_eq!(exit.code, Some(0));
    assert!(!exit.unreadable);
    assert!(exit.stderr.contains("reply exactly ok"));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::Result {
            ok: true,
            tokens: Some(5),
            ..
        }
    )));
}

#[tokio::test]
async fn codex_lead_stays_read_only_and_keeps_bridge_token_off_argv() {
    let temp = support::Temp::new("codex-lead");
    let adapter = CodexAdapter::new(Some(support::fake(&["argv"])));
    let opts = RunOptions {
        access: Some(JobAccess::Full),
        model: Some("test-model".into()),
        resume: Some("known-thread".into()),
        bridge: Some(BridgeInfo {
            url: "http://127.0.0.1:9/mcp".into(),
            token: "private-bridge-token".into(),
            tools: vec!["get_status".into()],
        }),
        ..RunOptions::default()
    };
    let mut run = adapter.run("job", "plan", temp.path(), &opts);
    let args = support::cli_args(&mut run).await;
    assert!(args.windows(2).any(|a| a == ["--sandbox", "read-only"]));
    assert!(args.windows(2).any(|a| a == ["--model", "test-model"]));
    assert!(args.windows(2).any(|a| a == ["resume", "known-thread"]));
    assert!(args.iter().any(|a| a.contains("LEGATE_MCP_TOKEN")));
    assert!(!args.iter().any(|a| a.contains("private-bridge-token")));
}

#[tokio::test]
async fn codex_missing_binary_returns_a_readable_failure() {
    let temp = support::Temp::new("codex-missing");
    let adapter = CodexAdapter::new(Some(legate_providers::Bin::new("legate-no-such-codex")));
    let mut run = adapter.run("job", "hello", temp.path(), &RunOptions::default());
    let (_, exit) = support::collect(&mut run).await;
    assert_eq!(exit.code, None);
    assert!(!exit.stderr.is_empty());
}
