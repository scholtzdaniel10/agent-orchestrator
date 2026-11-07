//! Ports of liteagent's pytest suite, plus the Ollama, OpenAI, stream, and loop-guard checks.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;

use crate::fake_server::FakeServer;
use crate::{
    Agent, Backend, DEFAULT_SYSTEM, ESCALATE, FINAL, StepEvent, Tool, default_tools, parse_step,
    step_schema, tools_prompt,
};

fn j_tool(thought: &str, tool: &str, args_body: &str) -> String {
    format!(r#"{{"thought": "{thought}", "tool": "{tool}", "args": {args_body}}}"#)
}

fn j_calc(thought: &str, expr: &str) -> String {
    j_tool(
        thought,
        "calculator",
        &format!(r#"{{"expression": "{expr}"}}"#),
    )
}

fn j_final(thought: &str, answer: &str) -> String {
    j_tool(
        thought,
        "final_answer",
        &format!(r#"{{"answer": "{answer}"}}"#),
    )
}

fn schema_names(scripted: &serde_json::Value) -> Vec<String> {
    scripted["oneOf"]
        .as_array()
        .expect("oneOf")
        .iter()
        .map(|branch| {
            branch["properties"]["tool"]["const"]
                .as_str()
                .expect("const")
                .to_string()
        })
        .collect()
}

#[test]
fn schema_and_parse() {
    let dir = tempfile::tempdir().unwrap();
    let tools = default_tools(dir.path());
    let schema = step_schema(&tools, true);
    assert_eq!(
        schema_names(&schema),
        vec![
            "calculator",
            "list_files",
            "read_file",
            "write_note",
            "final_answer",
            "ask_bigger_model"
        ]
    );
    let step = parse_step(r#"{"thought":"x","tool":"calculator","args":{"expression":"1+1"}}"#)
        .expect("step");
    assert_eq!(step.tool, "calculator");
    assert!(parse_step("not json").is_none());
    assert!(parse_step(r#"{"tool":"x","args":"oops"}"#).is_none());
}

#[test]
fn tools_prompt_matches_python() {
    let dir = tempfile::tempdir().unwrap();
    let tools = default_tools(dir.path());
    let prompt = tools_prompt(&tools, false);
    let expected = concat!(
        "You can use these tools. Reply with exactly one JSON object per step:\n",
        "{\"thought\": \"<one short sentence>\", \"tool\": \"<name>\", \"args\": {...}}\n",
        "\n",
        "- calculator: evaluate arithmetic, e.g. 12*(3+4) args={\"expression\": {\"type\": \"string\"}}\n",
        "- list_files: list files in a folder of the work folder args={\"folder\": {\"type\": \"string\"}}\n",
        "- read_file: read a text file from the work folder args={\"path\": {\"type\": \"string\"}}\n",
        "- write_note: write a text file in the work folder args={\"path\": {\"type\": \"string\"}, \"text\": {\"type\": \"string\"}}\n",
        "- final_answer: give the final answer to the user. args={\"answer\": \"string\"}"
    );
    assert_eq!(prompt, expected);
    let with_escalate = tools_prompt(&tools, true);
    assert!(with_escalate.ends_with(
        "- ask_bigger_model: use if the step is too hard for you. args={\"reason\": \"string\"}"
    ));
}

#[test]
fn calculator_safe() {
    let dir = tempfile::tempdir().unwrap();
    let tools = default_tools(dir.path());
    let calc = &tools[0];
    assert_eq!(calc.call(&json!({"expression": "2340*0.175"})), "409.5");
    assert_eq!(calc.call(&json!({"expression": "1250*12-3999"})), "11001");
    assert_eq!(calc.call(&json!({"expression": "6*7"})), "42");
    assert_eq!(calc.call(&json!({"expression": "12*(3+4)"})), "84");
    assert_eq!(calc.call(&json!({"expression": "2**3**2"})), "512");
    assert_eq!(calc.call(&json!({"expression": "-2**2"})), "-4");
    assert_eq!(calc.call(&json!({"expression": "4/2"})), "2.0");
    assert!(
        calc.call(&json!({"expression": "__import__('os')"}))
            .starts_with("error")
    );
}

#[test]
fn files_stay_in_the_work_folder() {
    let dir = tempfile::tempdir().unwrap();
    let tools = default_tools(dir.path());
    let write = tools.iter().find(|t| t.name == "write_note").unwrap();
    let read = tools.iter().find(|t| t.name == "read_file").unwrap();
    let list = tools.iter().find(|t| t.name == "list_files").unwrap();
    assert_eq!(
        write.call(&json!({"path": "todo.txt", "text": "call Mokoena Bakery"})),
        "ok"
    );
    assert_eq!(
        read.call(&json!({"path": "todo.txt"})),
        "call Mokoena Bakery"
    );
    assert_eq!(list.call(&json!({"folder": "."})), "todo.txt");
    assert!(
        read.call(&json!({"path": "../outside.txt"}))
            .starts_with("error:")
    );
    assert!(
        write
            .call(&json!({"path": "..\\outside.txt", "text": "no"}))
            .starts_with("error:")
    );
    let long = "x".repeat(2001);
    assert_eq!(write.call(&json!({"path": "long.txt", "text": long})), "ok");
    let trimmed = read.call(&json!({"path": "long.txt"}));
    assert!(trimmed.ends_with("\n...[trimmed]"), "{trimmed}");
    assert_eq!(trimmed.chars().filter(|c| *c == 'x').count(), 2000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_loop_cache_and_prefix_store() {
    let script = [
        j_calc("compute", "2340*0.175"),
        j_final("done", "409.5"),
        j_calc("compute", "1250*12-3999"),
        j_final("done", "11001"),
    ];
    let refs: Vec<&str> = script.iter().map(String::as_str).collect();
    let fake = FakeServer::llama(&refs).await;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(
        Backend::llama(fake.url()).with_name("big"),
        None,
        default_tools(dir.path()),
        DEFAULT_SYSTEM,
    );
    let first = agent.run("What is 17.5% of 2340?").await.unwrap();
    assert_eq!(first.answer.as_deref(), Some("409.5"));
    assert_eq!(first.stats.calls.len(), 2);
    let schema_calls: Vec<_> = fake
        .calls()
        .into_iter()
        .filter(|call| call.schema)
        .collect();
    assert!(
        schema_calls[1].cached > schema_calls[1].new_chars * 3,
        "cached {} new {}",
        schema_calls[1].cached,
        schema_calls[1].new_chars
    );
    let second = agent
        .run("Multiply 1250 by 12, then subtract 3999.")
        .await
        .unwrap();
    assert_eq!(second.answer.as_deref(), Some("11001"));
    let schema_calls: Vec<_> = fake
        .calls()
        .into_iter()
        .filter(|call| call.schema)
        .collect();
    assert!(
        schema_calls[2].cached > 500,
        "cached {}",
        schema_calls[2].cached
    );
    let stats = agent.prefix_stats();
    assert_eq!(stats.builds, 1, "{stats:?}");
    assert!(stats.warm_hits >= 3, "{stats:?}");
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|call| call.n_predict == 0)
            .count(),
        1
    );
    assert!(schema_calls.iter().all(|call| call.cache_prompt));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prefix_restore_after_restart() {
    let line = j_final("d", "a");
    let fake = FakeServer::llama(&[&line, &line]).await;
    let dir = tempfile::tempdir().unwrap();
    let tools = default_tools(dir.path());
    let mut first = Agent::new(
        Backend::llama(fake.url()),
        None,
        tools.clone(),
        DEFAULT_SYSTEM,
    );
    first.run("hi").await.unwrap();
    fake.clear_slots();
    let mut second = Agent::new(Backend::llama(fake.url()), None, tools, DEFAULT_SYSTEM);
    second.run("hi again").await.unwrap();
    assert_eq!(second.prefix_stats().restores, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cascade_escalates() {
    let small_script = [
        j_tool("hard", "ask_bigger_model", r#"{"reason": "multi-step"}"#),
        j_final("done", "42"),
    ];
    let big_script = [j_calc("calc", "6*7")];
    let small_refs: Vec<&str> = small_script.iter().map(String::as_str).collect();
    let big_refs: Vec<&str> = big_script.iter().map(String::as_str).collect();
    let small_fake = FakeServer::llama(&small_refs).await;
    let big_fake = FakeServer::llama(&big_refs).await;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(
        Backend::llama(big_fake.url()).with_name("big"),
        Some(Backend::llama(small_fake.url()).with_name("small")),
        default_tools(dir.path()),
        DEFAULT_SYSTEM,
    );
    let outcome = agent.run("6 times 7?").await.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("42"));
    assert_eq!(outcome.stats.escalations, 1);
    let models: Vec<_> = outcome
        .stats
        .calls
        .iter()
        .map(|c| c.model.as_str())
        .collect();
    assert_eq!(models, ["small", "big", "small"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_json_from_small_falls_back_to_big() {
    let small_fake = FakeServer::llama(&["{broken"]).await;
    let big_line = j_final("d", "ok");
    let big_fake = FakeServer::llama(&[&big_line]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(
        Backend::llama(big_fake.url()).with_name("big"),
        Some(Backend::llama(small_fake.url()).with_name("small")),
        default_tools(dir.path()),
        DEFAULT_SYSTEM,
    );
    let outcome = agent.run("x").await.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("ok"));
    assert_eq!(outcome.stats.invalid, 1);
    assert_eq!(outcome.stats.escalations, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loop_guard_forces_final_answer() {
    let calc = j_calc("c", "1250*12-3999");
    let done = j_final("d", "11001");
    let fake = FakeServer::llama(&[&calc, &calc, &done]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(
        Backend::llama(fake.url()),
        None,
        default_tools(dir.path()),
        DEFAULT_SYSTEM,
    );
    let outcome = agent
        .run("Multiply 1250 by 12, then subtract 3999.")
        .await
        .unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("11001"));
    assert_eq!(outcome.stats.repeats, 1);
    assert_eq!(outcome.stats.calls.len(), 3);
    let schema_calls: Vec<_> = fake
        .calls()
        .into_iter()
        .filter(|call| call.schema)
        .collect();
    assert_eq!(schema_calls.len(), 3);
    assert_eq!(
        schema_names(schema_calls[2].schema_body.as_ref().unwrap()),
        vec!["final_answer"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn small_model_repeat_escalates_to_big() {
    let calc = j_calc("c", "6*7");
    let small_fake = FakeServer::llama(&[&calc, &calc]).await;
    let done = j_final("d", "42");
    let big_fake = FakeServer::llama(&[&done]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(
        Backend::llama(big_fake.url()).with_name("big"),
        Some(Backend::llama(small_fake.url()).with_name("small")),
        default_tools(dir.path()),
        DEFAULT_SYSTEM,
    );
    let outcome = agent.run("6 times 7?").await.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("42"));
    assert_eq!(outcome.stats.escalations, 1);
    assert_eq!(outcome.stats.repeats, 1);
    let models: Vec<_> = outcome
        .stats
        .calls
        .iter()
        .map(|c| c.model.as_str())
        .collect();
    assert_eq!(models, ["small", "small", "big"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_call_is_not_executed_twice() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    let tool = Tool::new(
        "ping",
        "count calls",
        crate::object_schema(&[("id", json!({"type": "string"}))]),
        move |_args| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok("pong".to_string())
        },
    );
    let ping = j_tool("c", "ping", r#"{"id": "1"}"#);
    let done = j_final("d", "pong");
    let fake = FakeServer::llama(&[&ping, &ping, &done]).await;
    let mut agent = Agent::new(Backend::llama(fake.url()), None, vec![tool], DEFAULT_SYSTEM);
    let outcome = agent.run("ping once").await.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("pong"));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.stats.repeats, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ollama_is_append_only_and_skips_prefix_store() {
    let calc = j_calc("compute", "2340*0.175");
    let done = j_final("done", "409.5");
    let fake = FakeServer::ollama(&[&calc, &done]).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::ollama(fake.url())
        .with_name("big")
        .with_model("coder-agent:latest")
        .with_num_ctx(65536);
    let mut agent = Agent::new(backend, None, default_tools(dir.path()), DEFAULT_SYSTEM);
    let outcome = agent.run("What is 17.5% of 2340?").await.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("409.5"));
    assert_eq!(agent.prefix_stats().builds, 0);
    assert_eq!(agent.prefix_stats().restores, 0);
    let chats: Vec<_> = fake
        .bodies()
        .into_iter()
        .filter(|rec| rec.path == "/api/chat")
        .collect();
    assert!(chats.len() >= 2, "{}", chats.len());
    let first = chats[0].body["messages"].as_array().unwrap();
    let second = chats[1].body["messages"].as_array().unwrap();
    assert!(second.len() > first.len());
    assert_eq!(&second[..first.len()], first.as_slice());
    assert_eq!(chats[0].body["options"]["num_ctx"], json!(65536));
    assert!(chats[0].body["format"]["oneOf"].is_array());
    assert!(chats[0].body["model"] == "coder-agent:latest");
    assert!(fake.bodies().iter().all(|rec| rec.path != "/completion"));
    assert!(outcome.stats.prompt_tokens > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openai_chat_uses_tools() {
    let calc = j_calc("compute", "6*7");
    let done = j_final("done", "42");
    let fake = FakeServer::openai(&[&calc, &done]).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::openai(fake.url())
        .with_name("big")
        .with_model("local");
    let mut agent = Agent::new(backend, None, default_tools(dir.path()), DEFAULT_SYSTEM);
    let outcome = agent.run("6 times 7?").await.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("42"));
    assert_eq!(outcome.stats.calls[0].prompt_tokens, 60);
    assert_eq!(outcome.stats.calls[0].cached_tokens, 40);
    let requests: Vec<_> = fake
        .bodies()
        .into_iter()
        .filter(|rec| rec.path == "/v1/chat/completions")
        .collect();
    assert!(requests.len() >= 2);
    assert_eq!(requests[0].body["tool_choice"], "required");
    let names: Vec<_> = requests[0].body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.iter().any(|name| name == "calculator"));
    assert!(names.iter().any(|name| name == FINAL));
    assert!(!names.iter().any(|name| name == ESCALATE));
    let first = requests[0].body["messages"].as_array().unwrap();
    let second = requests[1].body["messages"].as_array().unwrap();
    assert_eq!(&second[..first.len()], first.as_slice());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_detects_llama_ollama_and_openai() {
    let line = j_final("d", "a");
    let llama = FakeServer::llama(&[&line]).await;
    let ollama = FakeServer::ollama(&[&line]).await;
    let openai = FakeServer::openai(&[&line]).await;
    let llama_kind = Backend::connect(llama.url()).await.unwrap().kind();
    let ollama_kind = Backend::connect(ollama.url()).await.unwrap().kind();
    let openai_kind = Backend::connect(openai.url()).await.unwrap().kind();
    assert_eq!(llama_kind, crate::ServerKind::LlamaServer);
    assert_eq!(ollama_kind, crate::ServerKind::Ollama);
    assert_eq!(openai_kind, crate::ServerKind::OpenAi);
    let down = Backend::llama("http://127.0.0.1:1");
    assert!(!down.health().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_yields_tool_call_result_and_final() {
    let calc = j_calc("compute", "2340*0.175");
    let done = j_final("done", "409.5");
    let fake = FakeServer::llama(&[&calc, &done]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(
        Backend::llama(fake.url()),
        None,
        default_tools(dir.path()),
        DEFAULT_SYSTEM,
    );
    let events = tokio::time::timeout(
        Duration::from_secs(5),
        agent.run_stream("percent").into_events(),
    )
    .await
    .expect("stream timed out");
    let events: Vec<_> = events.into_iter().map(Result::unwrap).collect();
    assert!(matches!(
        &events[0],
        StepEvent::ToolCall { name, .. } if name == "calculator"
    ));
    assert!(matches!(&events[1], StepEvent::ToolResult { result, .. } if result == "409.5"));
    assert!(matches!(
        events.last(),
        Some(StepEvent::Final { answer: Some(answer), .. }) if answer == "409.5"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_cache_skips_the_prefix_store() {
    let done = j_final("d", "ok");
    let fake = FakeServer::llama(&[&done]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(
        Backend::llama(fake.url()).with_cache(false),
        None,
        default_tools(dir.path()),
        DEFAULT_SYSTEM,
    );
    let outcome = agent.run("hi").await.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("ok"));
    assert_eq!(agent.prefix_stats().builds, 0);
    assert_eq!(agent.prefix_stats().warm_hits, 0);
    assert!(
        fake.calls()
            .iter()
            .all(|call| call.cached == 0 && !call.cache_prompt)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_can_register_a_tool() {
    let echo = j_tool("e", "echo", r#"{"text": "hello"}"#);
    let done = j_final("d", "hello");
    let fake = FakeServer::llama(&[&echo, &done]).await;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(
        Backend::llama(fake.url()),
        None,
        default_tools(dir.path()),
        DEFAULT_SYSTEM,
    );
    agent.register_tool(Tool::new(
        "echo",
        "repeat text",
        crate::object_schema(&[("text", json!({"type": "string"}))]),
        |args| {
            Ok(args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string())
        },
    ));
    let outcome = agent.run("echo").await.unwrap();
    assert_eq!(outcome.answer.as_deref(), Some("hello"));
    assert_eq!(outcome.steps[0].tool, "echo");
}

#[test]
fn prefix_key_is_stable() {
    let a = crate::PrefixStore::key_for("hello", "big");
    let b = crate::PrefixStore::key_for("hello", "small");
    assert_eq!(a.len(), 24);
    assert!(a.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert_ne!(a, b);
    assert_eq!(a, crate::PrefixStore::key_for("hello", "big"));
}
