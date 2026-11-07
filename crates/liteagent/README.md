# liteagent

A small async agent loop for local models. It does not make the model faster at
generating tokens. It stops the agent loop from redoing the same prompt work,
and it stops the model from emitting tool calls that will not parse.

The loop only appends messages, so the prefix stays byte-for-byte the same on
every step.

| Piece | What it does |
| --- | --- |
| Prefix store | On llama-server, the system prompt and tool list are processed once. The KV cache is saved and restored with `/slots/{id}?action=save\|restore` (`--slot-save-path`). |
| Tool grammar | One JSON schema covers every tool. llama-server enforces it as a grammar. Ollama receives it as `format`. |
| Cascade | The small model tries each step. Hard, invalid, or stuck steps go to the big model. |
| Loop guard | A repeated tool call is not run again. The model is shown the old result, then the grammar allows only `final_answer`. A small model that repeats is handed to the big model instead. |

Ollama has no slot files. The prefix store is a no-op there. The message list is still append-only, so Ollama's own prompt cache can hit. Pass context length with `Backend::with_num_ctx` (`options.num_ctx`).

Built-in tools, all jailed to a work folder: `calculator`, `list_files`, `read_file`, `write_note`. There is no shell. Register your own tools with `Tool::new` or `Agent::register_tool`. `final_answer` ends the task. `ask_bigger_model` is added only on the small model.

## llama-server on Windows

```powershell
mkdir C:\liteagent\slots
C:\llama\llama-server.exe -m C:\models\big.gguf --port 8080 -c 16384 -fa on -np 1 --jinja --slot-save-path C:\liteagent\slots
```

Optional small model, second terminal, port 8081, its own `--slot-save-path` or the same folder.

```powershell
$env:CARGO_TARGET_DIR = "target\liteagent"
cargo run -p liteagent --bin liteagent-bench -- --big http://127.0.0.1:8080 --slot-dir C:\liteagent\slots
cargo run -p liteagent --bin liteagent-bench -- --big http://127.0.0.1:8080 --small http://127.0.0.1:8081 --slot-dir C:\liteagent\slots -v
cargo run -p liteagent --bin liteagent-bench -- --big http://127.0.0.1:8080 --slot-dir C:\liteagent\slots --no-cache
```

`--no-cache` turns off `cache_prompt`. That is the baseline.

## Ollama on Windows

Ollama must already have the model. This crate does not pull one.

```powershell
ollama list
cargo run -p liteagent --bin liteagent-bench -- --big http://127.0.0.1:11434 --model coder-agent:latest --num-ctx 65536 -v --tasks 1
```

`Backend::connect` checks `/api/version` for Ollama and `/props` or `/health` for llama-server. Anything else that answers `/v1/models` is treated as an OpenAI-compatible server (`/v1/chat/completions` with tools).

## Library

```rust
use liteagent::{default_tools, Agent, Backend, DEFAULT_SYSTEM};

let big = Backend::connect("http://127.0.0.1:8080").await?
    .with_slot_dir(r"C:\liteagent\slots");
let mut agent = Agent::new(big, None, default_tools(r"C:\work"), DEFAULT_SYSTEM);
let outcome = agent.run("What is 17.5% of 2340?").await?;
println!(
    "{}  processed {}  reused {}  {:.1} tok/s",
    outcome.answer.unwrap_or_default(),
    outcome.stats.prompt_tokens,
    outcome.stats.cached_tokens,
    outcome.stats.tokens_per_sec,
);
```

`agent.run_stream(task)` yields `ToolCall`, `ToolResult`, `Escalation`, `Repeat`, and a final `Final` that carries the same stats.

## Tests

No model required. A fake llama-server, a fake Ollama, and a fake OpenAI server run inside the tests.

```powershell
$env:CARGO_TARGET_DIR = "target\liteagent"
cargo test -p liteagent
```
