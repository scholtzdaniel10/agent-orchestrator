//! Speed and reliability check. Same tasks as the Python `bench.py`.
//!
//! ```text
//! cargo run -p liteagent --bin liteagent-bench -- --big http://127.0.0.1:8080 --slot-dir C:\liteagent\slots
//! cargo run -p liteagent --bin liteagent-bench -- --big http://127.0.0.1:11434 --model coder-agent:latest --num-ctx 65536
//! ```

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use clap::Parser;
use liteagent::{Agent, Backend, DEFAULT_SYSTEM, ServerKind, default_tools};
use serde::Serialize;

const FILES: &[(&str, &str)] = &[
    (
        "invoices/jan.txt",
        "Invoice total: R 1250\nClient: Mokoena Bakery\n",
    ),
    (
        "invoices/feb.txt",
        "Invoice total: R 980\nClient: Mokoena Bakery\n",
    ),
    (
        "invoices/mar.txt",
        "Invoice total: R 1415\nClient: Dlamini Motors\n",
    ),
    (
        "notes/meeting.txt",
        "Next meeting: Thursday 14:00. Bring the price list.\n",
    ),
    (
        "prices.csv",
        "item,price\ncoffee,32\nmuffin,28\nsandwich,55\n",
    ),
];

const TASKS: &[(&str, Check)] = &[
    ("What is 17.5% of 2340?", Check::Parts(&["409.5"])),
    (
        "Multiply 1250 by 12, then subtract 3999.",
        Check::Parts(&["11001"]),
    ),
    (
        "What time is the next meeting? Check notes/meeting.txt.",
        Check::Parts(&["14:00"]),
    ),
    (
        "Add up the invoice totals for jan, feb and mar in the invoices folder.",
        Check::Parts(&["3645"]),
    ),
    (
        "Which client is on the March invoice?",
        Check::Parts(&["Dlamini"]),
    ),
    (
        "How much do 3 coffees and 2 muffins cost? Use prices.csv.",
        Check::Parts(&["152"]),
    ),
    (
        "List the files in the invoices folder and tell me how many there are.",
        Check::Parts(&["3"]),
    ),
    (
        "Write a note called todo.txt that says 'call Mokoena Bakery', then confirm.",
        Check::File("todo.txt"),
    ),
    (
        "What is the average of the jan and feb invoice totals?",
        Check::Parts(&["1115"]),
    ),
    (
        "Price of a sandwich plus 15% VAT?",
        Check::Parts(&["63.25"]),
    ),
];

#[derive(Clone, Copy)]
enum Check {
    Parts(&'static [&'static str]),
    File(&'static str),
}

#[derive(Parser, Debug)]
#[command(name = "liteagent-bench")]
struct Args {
    /// Big model server. llama-server or Ollama; the protocol is detected.
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    big: String,
    /// Optional small model server for the cascade.
    #[arg(long)]
    small: Option<String>,
    /// Same folder as llama-server `--slot-save-path`.
    #[arg(long)]
    slot_dir: Option<PathBuf>,
    /// Disable llama-server prompt caching (the baseline).
    #[arg(long)]
    no_cache: bool,
    /// How many of the fixed tasks to run, from the start of the list.
    #[arg(long, default_value_t = TASKS.len())]
    tasks: usize,
    /// Print each tool step.
    #[arg(short, long)]
    verbose: bool,
    /// Write the per-task rows as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Model name for Ollama or an OpenAI-compatible server.
    #[arg(long)]
    model: Option<String>,
    /// Model name for `--small` when it differs from `--model`.
    #[arg(long)]
    small_model: Option<String>,
    /// Ollama `num_ctx`. Ignored by llama-server.
    #[arg(long)]
    num_ctx: Option<u32>,
}

#[derive(Serialize)]
struct Row {
    task: String,
    ok: bool,
    answer: Option<String>,
    wall_s: f64,
    calls: usize,
    escalations: u32,
    invalid: u32,
    repeats: u32,
    prompt_tokens: u64,
    cached_tokens: u64,
    prompt_ms: f64,
    gen_tokens: u64,
    gen_ms: f64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let runtime = tokio::runtime::Runtime::new().context("start tokio")?;
    runtime.block_on(run(args))
}

async fn run(args: Args) -> anyhow::Result<()> {
    let big = make_backend(&args.big, "big", args.model.as_deref(), &args).await?;
    let small = match &args.small {
        Some(url) => {
            let model = args.small_model.as_deref().or(args.model.as_deref());
            Some(make_backend(url, "small", model, &args).await?)
        }
        None => None,
    };
    let work = make_workdir().context("create bench work folder")?;
    eprintln!("work folder: {}", work.display());
    let mut agent =
        Agent::new(big, small, default_tools(&work), DEFAULT_SYSTEM).with_verbose(args.verbose);

    let mut rows = Vec::new();
    let mut solved = 0usize;
    let limit = args.tasks.min(TASKS.len());
    for (index, (task, check)) in TASKS.iter().take(limit).enumerate() {
        let outcome = agent
            .run(task)
            .await
            .with_context(|| format!("task: {task}"))?;
        let ok = task_ok(&work, outcome.answer.as_deref(), *check);
        if ok {
            solved += 1;
        }
        let calls = outcome.stats.calls.len();
        let row = Row {
            task: (*task).to_string(),
            ok,
            answer: outcome.answer.clone(),
            wall_s: round2(outcome.stats.wall_s),
            calls,
            escalations: outcome.stats.escalations,
            invalid: outcome.stats.invalid,
            repeats: outcome.stats.repeats,
            prompt_tokens: outcome.stats.prompt_tokens,
            cached_tokens: outcome.stats.cached_tokens,
            prompt_ms: outcome.stats.wait_before_step_ms,
            gen_tokens: outcome.stats.gen_tokens,
            gen_ms: outcome.stats.gen_ms,
        };
        let mark = if ok { "PASS" } else { "FAIL" };
        let shown = outcome.answer.as_deref().unwrap_or("None");
        let task_short: String = task.chars().take(50).collect();
        let answer_short: String = shown.chars().take(60).collect();
        println!(
            "[{:2}] {mark} {:6.1}s  calls={calls} esc={} | {task_short}  -> {answer_short}",
            index + 1,
            outcome.stats.wall_s,
            outcome.stats.escalations,
        );
        rows.push(row);
    }

    let call_count = rows.iter().map(|row| row.calls).sum::<usize>().max(1);
    let gen_tokens: u64 = rows.iter().map(|row| row.gen_tokens).sum();
    let gen_s: f64 = rows.iter().map(|row| row.gen_ms).sum::<f64>() / 1000.0;
    let prompt_tokens: u64 = rows.iter().map(|row| row.prompt_tokens).sum();
    let cached_tokens: u64 = rows.iter().map(|row| row.cached_tokens).sum();
    let prompt_ms: f64 = rows.iter().map(|row| row.prompt_ms).sum();
    let walls: Vec<f64> = rows.iter().map(|row| row.wall_s).collect();
    let total: f64 = walls.iter().sum();
    let reused = prompt_tokens + cached_tokens;
    let reused_pct = if reused == 0 {
        0.0
    } else {
        100.0 * cached_tokens as f64 / reused as f64
    };
    let speed = if gen_s > 0.0 {
        gen_tokens as f64 / gen_s
    } else {
        0.0
    };
    println!("\n=== summary ===");
    println!("tasks solved        : {solved}/{}", rows.len());
    println!(
        "total time          : {total:.1} s (median task {:.1} s)",
        median(&walls)
    );
    println!(
        "avg time to 1st tok : {:.2} s per step",
        prompt_ms / call_count as f64 / 1000.0
    );
    println!("generation speed    : {speed:.1} tok/s");
    println!(
        "prompt tokens       : {prompt_tokens} processed, {cached_tokens} reused from cache ({reused_pct:.0}% reused)"
    );
    let invalid: u32 = rows.iter().map(|row| row.invalid).sum();
    let escalations: u32 = rows.iter().map(|row| row.escalations).sum();
    let repeats: u32 = rows.iter().map(|row| row.repeats).sum();
    println!("invalid tool calls  : {invalid} of {call_count} model calls");
    println!("escalations to big  : {escalations}");
    println!("repeated calls caught: {repeats}");
    if let Some(path) = &args.json {
        let text = serde_json::to_string_pretty(&rows).context("encode results")?;
        std::fs::write(path, format!("{text}\n"))
            .with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

async fn make_backend(
    url: &str,
    name: &str,
    model: Option<&str>,
    args: &Args,
) -> anyhow::Result<Backend> {
    let mut backend = Backend::connect(url)
        .await
        .with_context(|| format!("detect server at {url}"))?
        .with_name(name)
        .with_cache(!args.no_cache);
    if let Some(dir) = &args.slot_dir {
        backend = backend.with_slot_dir(dir.clone());
    }
    if let Some(model) = model {
        backend = backend.with_model(model);
    }
    if let Some(num_ctx) = args.num_ctx {
        backend = backend.with_num_ctx(num_ctx);
    }
    if backend.kind() == ServerKind::Ollama && model.is_none() {
        anyhow::bail!("Ollama at {url} needs --model (for example coder-agent:latest)");
    }
    if !backend.health().await {
        anyhow::bail!("server not reachable at {}", backend.url());
    }
    Ok(backend)
}

fn make_workdir() -> std::io::Result<PathBuf> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("liteagent_bench_{}_{millis}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    for (rel, text) in FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)?;
    }
    Ok(dir)
}

fn task_ok(work: &std::path::Path, answer: Option<&str>, check: Check) -> bool {
    match check {
        Check::File(name) => work.join(name).exists(),
        Check::Parts(parts) => {
            let Some(answer) = answer else {
                return false;
            };
            let lower = answer.to_lowercase();
            parts
                .iter()
                .all(|part| lower.contains(&part.to_lowercase()))
        }
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}
