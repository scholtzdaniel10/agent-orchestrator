//! Ports of `src/core/bridge/tools.test.ts`.
//!
//! Calls go through the real HTTP server. A fake backend stands in for the
//! orchestrator, which is not `Send`.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use legate_bridge::{
    GET_RESULT_DESCRIPTION, GET_RESULT_SCHEMA, GET_STATUS_DESCRIPTION, GET_STATUS_SCHEMA,
    LIST_WORKERS_DESCRIPTION, LIST_WORKERS_SCHEMA, PROPOSE_JOBS_DESCRIPTION, PROPOSE_JOBS_SCHEMA,
    SEND_JOB_DESCRIPTION, SEND_JOB_SCHEMA, ToolConfig, WorkerSnapshot, proposed_jobs,
};
use legate_core::{JobAccess, JobType, LeadProposal, ProposalStatus, ProposedJob, ProviderId};
use serde_json::{Value, json};

use common::{Fake, call_err, call_ok, client, json_body, rpc, start_orch};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_job_passes_lead_message_when_lead_turn_returns_an_id() {
    let fake = Fake::new();
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();

    fake.set_lead_turn(Some("m-turn".to_string()));
    let linked = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "delegated"}),
    )
    .await;
    assert_eq!(
        fake.job(linked["id"].as_str().unwrap())
            .lead_message
            .as_deref(),
        Some("m-turn")
    );

    fake.set_lead_turn(None);
    let omitted = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "plain"}),
    )
    .await;
    assert!(
        fake.job(omitted["id"].as_str().unwrap())
            .lead_message
            .is_none()
    );

    let bare = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "bare"}),
    )
    .await;
    assert!(
        fake.job(bare["id"].as_str().unwrap())
            .lead_message
            .is_none()
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_job_returns_the_job_id_and_provider_and_rejects_bad_input() {
    let fake = Fake::new();
    fake.set_max_running(1);
    fake.push_ok_held("done");
    fake.push_ok("next");
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();

    assert!(
        call_err(
            &http,
            &bridge,
            "send_job",
            json!({"type": "ship", "prompt": "x"})
        )
        .await
        .contains("type")
    );
    assert!(
        call_err(
            &http,
            &bridge,
            "send_job",
            json!({"type": "planning", "prompt": "   "})
        )
        .await
        .contains("prompt")
    );
    assert!(
        call_err(
            &http,
            &bridge,
            "send_job",
            json!({"type": "planning", "prompt": ""})
        )
        .await
        .contains("prompt")
    );
    assert!(
        call_err(
            &http,
            &bridge,
            "send_job",
            json!({"type": "planning", "prompt": "x".repeat(20_001)})
        )
        .await
        .contains("20000")
    );

    let first = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "hello"}),
    )
    .await;
    assert_eq!(first["provider"], json!("claude"));
    assert_eq!(first["status"], json!("running"));
    assert!(first["id"].is_string());
    assert_eq!(sorted_keys(&first), ["id", "provider", "status"]);
    assert_eq!(fake.job(first["id"].as_str().unwrap()).prompt, "hello");

    let second = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "queued-job"}),
    )
    .await;
    assert_eq!(second["provider"], json!("claude"));
    assert_eq!(second["status"], json!("queued"));

    fake.release();
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_status_lists_jobs_oldest_first_without_output_and_throws_for_an_unknown_id() {
    let fake = Fake::new();
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();
    let long = format!("{}MORE", "p".repeat(120));
    let first = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": long}),
    )
    .await;
    call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "debugging", "prompt": "short"}),
    )
    .await;

    let all = call_ok(&http, &bridge, "get_status", json!({})).await;
    let jobs = all.as_array().unwrap();
    let prompts: Vec<String> = jobs
        .iter()
        .map(|job| job["prompt"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(prompts, vec!["p".repeat(120), "short".to_string()]);
    assert!(jobs[0].get("output").is_none());
    assert!(jobs[1].get("output").is_none());
    assert_eq!(
        sorted_keys(&jobs[0]),
        ["failed_over", "id", "prompt", "provider", "status", "type"]
    );
    assert_eq!(jobs[0]["id"], first["id"]);
    assert_eq!(jobs[0]["type"], json!("planning"));
    assert_eq!(jobs[0]["provider"], json!("claude"));
    assert_eq!(jobs[0]["failed_over"], json!([]));

    let one = call_ok(
        &http,
        &bridge,
        "get_status",
        json!({"id": first["id"].as_str().unwrap()}),
    )
    .await;
    assert_eq!(one["id"], first["id"]);
    assert_eq!(one["prompt"], json!("p".repeat(120)));
    assert!(one.get("output").is_none());
    assert_eq!(
        call_err(&http, &bridge, "get_status", json!({"id": "missing"})).await,
        "Unknown job id: missing"
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_result_waits_until_the_job_finishes_and_returns_its_output() {
    let fake = Fake::new();
    fake.push_ok_held("finished-text");
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();
    let sent = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "review", "prompt": "look"}),
    )
    .await;
    let id = sent["id"].as_str().unwrap().to_string();
    let returned = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&returned);
    let bridge_url_owner = Arc::new(bridge);
    let pending_bridge = Arc::clone(&bridge_url_owner);
    let pending_id = id.clone();
    let pending = tokio::spawn(async move {
        let value = call_ok(
            &client(),
            &pending_bridge,
            "get_result",
            json!({"id": pending_id, "wait_seconds": 2}),
        )
        .await;
        flag.store(true, Ordering::SeqCst);
        value
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!returned.load(Ordering::SeqCst));
    fake.release();
    let value = pending.await.unwrap();
    assert_eq!(
        value,
        json!({
            "id": id,
            "status": "done",
            "provider": "claude",
            "failed_over": [],
            "output": "finished-text",
            "truncated": false,
            "edit": false,
            "change": null
        })
    );
    bridge_url_owner.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_result_returns_running_when_the_wait_is_zero_or_ends_first() {
    let fake = Fake::new();
    fake.push_ok_held("later");
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();
    let sent = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "hold"}),
    )
    .await;
    let id = sent["id"].as_str().unwrap();
    let immediate = call_ok(
        &http,
        &bridge,
        "get_result",
        json!({"id": id, "wait_seconds": 0}),
    )
    .await;
    assert_eq!(immediate["id"], json!(id));
    assert_eq!(immediate["status"], json!("running"));
    assert_eq!(immediate["provider"], json!("claude"));
    assert_eq!(immediate["failed_over"], json!([]));
    assert_eq!(immediate["output"], json!(""));
    assert_eq!(immediate["truncated"], json!(false));
    assert!(immediate.get("error").is_none());

    let early = call_ok(
        &http,
        &bridge,
        "get_result",
        json!({"id": id, "wait_seconds": 0.05}),
    )
    .await;
    assert_eq!(early["status"], json!("running"));
    assert_eq!(early["output"], json!(""));
    fake.release();
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_result_keeps_the_last_6000_characters_and_reports_a_failed_job() {
    let text = format!("{}END", "a".repeat(7000));
    let fake = Fake::new();
    fake.push_ok(&text);
    fake.push_fail("nope");
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();
    let big = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "big"}),
    )
    .await;
    let cut = call_ok(
        &http,
        &bridge,
        "get_result",
        json!({"id": big["id"].as_str().unwrap(), "wait_seconds": 2}),
    )
    .await;
    assert_eq!(cut["status"], json!("done"));
    assert_eq!(cut["truncated"], json!(true));
    let output = cut["output"].as_str().unwrap();
    assert_eq!(output, &text[text.len() - 6000..]);
    assert!(output.ends_with("END"));
    assert!(!output.starts_with(&"a".repeat(7000)));

    let failed = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "bad"}),
    )
    .await;
    let result = call_ok(
        &http,
        &bridge,
        "get_result",
        json!({"id": failed["id"].as_str().unwrap(), "wait_seconds": 2}),
    )
    .await;
    assert_eq!(result["status"], json!("failed"));
    assert_eq!(result["error"], json!("nope"));
    assert_eq!(result["provider"], json!("claude"));
    assert_eq!(
        call_err(&http, &bridge, "get_result", json!({"id": "missing"})).await,
        "Unknown job id: missing"
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_workers_reports_availability_headroom_rest_busy_and_queued() {
    let now = 1_700_000_000_000_i64;
    let fake = Fake::new();
    fake.set_max_running(1);
    fake.push_ok_held("a");
    fake.push_ok("b");
    fake.set_workers(vec![
        WorkerSnapshot {
            id: ProviderId::Claude,
            available: true,
            headroom: 1.0,
            resting_until: None,
            busy: true,
            queued: 1,
        },
        WorkerSnapshot {
            id: ProviderId::Cursor,
            available: false,
            headroom: 0.0,
            resting_until: Some(now + 60_000),
            busy: false,
            queued: 0,
        },
    ]);
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();
    call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "one"}),
    )
    .await;
    call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "two"}),
    )
    .await;
    let listed = call_ok(&http, &bridge, "list_workers", json!({})).await;
    assert_eq!(
        listed,
        json!({
            "max_access": "read",
            "workers": [
                {
                    "provider": "claude",
                    "available": true,
                    "headroom": 1,
                    "resting_until": null,
                    "busy": true,
                    "queued": 1
                },
                {
                    "provider": "cursor",
                    "available": false,
                    "headroom": 0,
                    "resting_until": "2023-11-14T22:14:20.000Z",
                    "busy": false,
                    "queued": 0
                }
            ]
        })
    );
    fake.release();
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_job_and_get_result_work_over_the_bridge() {
    let fake = Fake::new();
    fake.push_ok("bridge-output");
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();
    let sent = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "over http"}),
    )
    .await;
    let result = call_ok(
        &http,
        &bridge,
        "get_result",
        json!({"id": sent["id"].as_str().unwrap(), "wait_seconds": 2}),
    )
    .await;
    assert_eq!(result["id"], sent["id"]);
    assert_eq!(result["status"], json!("done"));
    assert_eq!(result["provider"], json!("claude"));
    assert_eq!(result["output"], json!("bridge-output"));
    assert_eq!(result["truncated"], json!(false));
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_job_passes_edit_through_and_get_result_reports_the_change() {
    let fake = Fake::new();
    fake.set_max_access(JobAccess::Edit);
    fake.push_ok("edited");
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();
    let listed = rpc(&http, &bridge, json!({"id": 1, "method": "tools/list"})).await;
    let text = listed.text.clone();
    assert!(text.contains(SEND_JOB_DESCRIPTION));
    assert!(text.contains(SEND_JOB_SCHEMA));
    assert!(text.contains(LIST_WORKERS_DESCRIPTION));
    assert!(text.contains(LIST_WORKERS_SCHEMA));
    assert!(text.contains(GET_STATUS_DESCRIPTION));
    assert!(text.contains(GET_STATUS_SCHEMA));
    assert!(text.contains(GET_RESULT_DESCRIPTION));
    assert!(text.contains(GET_RESULT_SCHEMA));
    let tools = json_body(&listed)["result"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    let send = tools
        .iter()
        .find(|tool| tool["name"] == "send_job")
        .unwrap();
    assert!(send["description"]
        .as_str()
        .unwrap()
        .contains("Access is read (files only), edit (change files in a separate worktree the user reviews and merges), or full"));

    assert_eq!(
        call_err(
            &http,
            &bridge,
            "send_job",
            json!({"type": "planning", "prompt": "x", "edit": "yes"})
        )
        .await,
        "edit must be a boolean"
    );
    let sent = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "boilerplate", "prompt": "change it", "edit": true}),
    )
    .await;
    let id = sent["id"].as_str().unwrap();
    let stored = fake.job(id);
    assert_eq!(stored.access, Some(JobAccess::Edit));
    assert_eq!(stored.edit, Some(true));
    let result = call_ok(
        &http,
        &bridge,
        "get_result",
        json!({"id": id, "wait_seconds": 1}),
    )
    .await;
    assert_eq!(result["status"], json!("done"));
    assert_eq!(result["edit"], json!(true));
    assert_eq!(result["change"], json!(&id[..8]));
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_job_access_passes_through_when_allowed_is_lowered_and_rejects_a_bad_value() {
    let fake = Fake::new();
    let bridge = start_orch(Arc::clone(&fake), ToolConfig::default()).await;
    let http = client();

    assert_eq!(
        call_err(
            &http,
            &bridge,
            "send_job",
            json!({"type": "planning", "prompt": "x", "access": "write"})
        )
        .await,
        "access must be read, edit, or full"
    );

    fake.set_max_access(JobAccess::Edit);
    let listed = call_ok(&http, &bridge, "list_workers", json!({})).await;
    assert_eq!(listed["max_access"], json!("edit"));

    fake.set_max_access(JobAccess::Read);
    let read_ok = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "planning", "prompt": "look", "access": "read"}),
    )
    .await;
    assert!(read_ok.get("note").is_none());
    let stored = fake.job(read_ok["id"].as_str().unwrap());
    assert_eq!(stored.access, Some(JobAccess::Read));
    assert!(stored.edit.is_none());

    fake.set_max_access(JobAccess::Edit);
    let edit_ok = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "boilerplate", "prompt": "change", "access": "edit"}),
    )
    .await;
    assert!(edit_ok.get("note").is_none());
    let stored = fake.job(edit_ok["id"].as_str().unwrap());
    assert_eq!(stored.access, Some(JobAccess::Edit));
    assert_eq!(stored.edit, Some(true));

    fake.set_max_access(JobAccess::Full);
    let full_ok = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "boilerplate", "prompt": "go", "access": "full"}),
    )
    .await;
    assert!(full_ok.get("note").is_none());
    let stored = fake.job(full_ok["id"].as_str().unwrap());
    assert_eq!(stored.access, Some(JobAccess::Full));
    assert_eq!(stored.edit, Some(true));

    fake.set_max_access(JobAccess::Edit);
    let old_bool = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "boilerplate", "prompt": "legacy", "edit": true}),
    )
    .await;
    assert!(old_bool.get("note").is_none());
    let stored = fake.job(old_bool["id"].as_str().unwrap());
    assert_eq!(stored.access, Some(JobAccess::Edit));
    assert_eq!(stored.edit, Some(true));

    fake.set_max_access(JobAccess::Read);
    let lowered_read = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "boilerplate", "prompt": "too much", "access": "full"}),
    )
    .await;
    assert_eq!(
        lowered_read["note"],
        json!("Access lowered to read: the person allows read only.")
    );
    let stored = fake.job(lowered_read["id"].as_str().unwrap());
    assert_eq!(stored.access, Some(JobAccess::Read));
    assert!(stored.edit.is_none());

    fake.set_max_access(JobAccess::Edit);
    let lowered_edit = call_ok(
        &http,
        &bridge,
        "send_job",
        json!({"type": "boilerplate", "prompt": "almost", "access": "full"}),
    )
    .await;
    assert_eq!(
        lowered_edit["note"],
        json!("Access lowered to edit: the person allows edit files.")
    );
    let stored = fake.job(lowered_edit["id"].as_str().unwrap());
    assert_eq!(stored.access, Some(JobAccess::Edit));
    assert_eq!(stored.edit, Some(true));
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_propose_the_lead_gets_propose_jobs_instead_of_send_job_and_nothing_starts() {
    let fake = Fake::new();
    fake.set_lead_turn(Some("m-turn".to_string()));
    fake.set_max_access(JobAccess::Edit);
    let bridge = start_orch(Arc::clone(&fake), ToolConfig { propose: true }).await;
    assert_eq!(
        bridge.info().tools,
        ["list_workers", "propose_jobs", "get_status", "get_result"]
    );
    let http = client();
    let listed = rpc(&http, &bridge, json!({"id": 1, "method": "tools/list"})).await;
    assert!(listed.text.contains(PROPOSE_JOBS_DESCRIPTION));
    assert!(listed.text.contains(PROPOSE_JOBS_SCHEMA));
    assert!(!listed.text.contains("\"name\":\"send_job\""));

    let result = call_ok(
        &http,
        &bridge,
        "propose_jobs",
        json!({
            "summary": "Split the router",
            "jobs": [
                {"title": "Queue", "type": "refactor", "access": "full", "prompt": "Move the queue"},
                {"title": "Review", "type": "review", "worker": "claude", "prompt": "Check it"}
            ]
        }),
    )
    .await;
    assert_eq!(result["jobs"], json!(2));
    assert!(
        result["note"]
            .as_str()
            .unwrap()
            .contains("Access lowered to edit")
    );
    assert!(fake.jobs().is_empty());
    let proposals = fake.proposals();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].0, "m-turn");
    assert_eq!(
        proposals[0].1,
        LeadProposal {
            summary: "Split the router".to_string(),
            status: ProposalStatus::Waiting,
            job_ids: None,
            jobs: vec![
                ProposedJob {
                    title: "Queue".to_string(),
                    job_type: JobType::Refactor,
                    access: JobAccess::Edit,
                    worker: None,
                    prompt: "Move the queue".to_string(),
                },
                ProposedJob {
                    title: "Review".to_string(),
                    job_type: JobType::Review,
                    access: JobAccess::Read,
                    worker: Some(ProviderId::Claude),
                    prompt: "Check it".to_string(),
                },
            ],
        }
    );
    bridge.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn propose_jobs_rejects_bad_plans_and_works_only_during_a_turn() {
    let fake = Fake::new();
    fake.set_lead_turn(Some("m".to_string()));
    let bridge = start_orch(Arc::clone(&fake), ToolConfig { propose: true }).await;
    let http = client();
    let job = json!({"title": "T", "type": "planning", "prompt": "p"});
    assert!(
        call_err(
            &http,
            &bridge,
            "propose_jobs",
            json!({"summary": "s", "jobs": []})
        )
        .await
        .contains("non-empty list")
    );
    let five = Value::Array(vec![
        job.clone(),
        job.clone(),
        job.clone(),
        job.clone(),
        job.clone(),
    ]);
    assert!(
        call_err(
            &http,
            &bridge,
            "propose_jobs",
            json!({"summary": "s", "jobs": five})
        )
        .await
        .contains("at most 4")
    );
    assert!(call_err(
        &http,
        &bridge,
        "propose_jobs",
        json!({"summary": "s", "jobs": [{"title": "T", "type": "planning", "prompt": "p", "worker": "codex"}]})
    )
    .await
    .contains("jobs[0].worker"));
    assert!(
        call_err(
            &http,
            &bridge,
            "propose_jobs",
            json!({"summary": "s", "jobs": [{"title": "T", "type": "planning", "prompt": " "}]})
        )
        .await
        .contains("jobs[0].prompt")
    );
    assert!(
        call_err(
            &http,
            &bridge,
            "propose_jobs",
            json!({"summary": "", "jobs": [job]})
        )
        .await
        .contains("summary")
    );

    fake.set_lead_turn(None);
    assert!(
        call_err(
            &http,
            &bridge,
            "propose_jobs",
            json!({"summary": "s", "jobs": [{"title": "T", "type": "planning", "prompt": "p"}]})
        )
        .await
        .contains("only works during a lead turn")
    );
    bridge.close().await.unwrap();
}

#[test]
fn proposed_jobs_checks_a_plan_the_person_edited() {
    assert_eq!(
        proposed_jobs(&json!([{
            "title": "A",
            "type": "debugging",
            "prompt": "fix it",
            "access": "full"
        }]))
        .unwrap(),
        vec![ProposedJob {
            title: "A".to_string(),
            job_type: JobType::Debugging,
            access: JobAccess::Full,
            worker: None,
            prompt: "fix it".to_string(),
        }]
    );
    assert!(
        proposed_jobs(&json!("nope"))
            .unwrap_err()
            .to_string()
            .contains("non-empty list")
    );
    assert!(
        proposed_jobs(&json!([{"title": "A", "type": "nope", "prompt": "x"}]))
            .unwrap_err()
            .to_string()
            .contains("jobs[0].type")
    );
}

fn sorted_keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}
