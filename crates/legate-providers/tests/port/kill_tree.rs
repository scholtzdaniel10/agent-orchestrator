use std::time::{Duration, Instant};

use legate_providers::{AgentEvent, ClaudeAdapter, ProviderAdapter, RunOptions};

use super::support::{self, Running};

#[tokio::test]
async fn kill_ends_the_child_and_the_grandchild() {
    let dir = support::Temp::new("tree");
    let dir_text = support::path_text(dir.path());
    let adapter = ClaudeAdapter::new(Some(support::fake(&["tree", &dir_text])));
    let mut running = Running {
        handle: adapter.run("tree", "x", dir.path(), &RunOptions::default()),
    };
    let first = support::timeout(running.handle.events.recv(), 5000).await;
    assert!(matches!(first, Some(AgentEvent::Init { .. })), "{first:?}");
    let child = support::read_pid(&dir.join("child.pid"));
    let grand = support::read_pid(&dir.join("grand.pid"));
    assert!(support::pid_alive(child), "child {child} was not alive");
    assert!(
        support::pid_alive(grand),
        "grandchild {grand} was not alive"
    );
    running.handle.kill();
    let exit = support::timeout(running.handle.exit(), 5000).await;
    assert_ne!(exit.code, Some(0), "stderr={}", exit.stderr);
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && (support::pid_alive(child) || support::pid_alive(grand)) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!support::pid_alive(child), "child {child} still alive");
    assert!(!support::pid_alive(grand), "grandchild {grand} still alive");
}
