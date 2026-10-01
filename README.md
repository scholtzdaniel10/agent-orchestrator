# agent-orchestrator

A desktop app (Electron + TypeScript) that spreads your coding jobs across the AI subscriptions you already pay for, so no plan sits idle or runs out early.

**Status:** early. The router and the lead bot work: provider adapters, job queue, routing rules, usage log, failover, a local MCP bridge and a lead chat. Jobs are read-only for now. Terminals, a git worktree per job and saved sessions come next.

## How it works

- **Lead chat.** You talk to one lead bot. It splits your request into jobs, hands them to workers, and reports back. The lead is an ordinary CLI session that can only call the app's own tools.
- **Workers.** You can also hand a job straight to a worker and pick its type (planning, debugging, review, refactor, boilerplate).
- **Router.** A rule table plus how much allowance each plan has left picks the provider. Claude's headroom comes from the usage figure its CLI reports.
- **Adapters.** The app spawns each provider's official CLI headless (`claude -p`, `agent -p`) and reads its `stream-json` output.
- **Failover.** If a plan reports a limit error, it is marked resting and its queued jobs move to the other plan.
- **Usage meter.** The window shows how much allowance each plan has left.

Everything runs locally, per person. There is no server.

## The MCP bridge

The lead reaches the app through a small MCP server inside the main process, with four tools: `list_workers`, `send_job`, `get_status` and `get_result`. It listens on loopback only, requires a random token that changes every start, and refuses requests that carry a browser `Origin`.

## Subscription use

- It runs the unmodified official CLIs, each signed in by you with your own plan.
- It never reads, stores or forwards login tokens or session credentials.
- It does not use any Agent SDK and makes no pay-per-token API calls.
- Jobs are started by a person, not on a timer.

You are responsible for staying within each provider's terms.

## Develop

Requires Node 22+ and pnpm.

```bash
pnpm install
pnpm dev           # run the app
pnpm test          # unit tests, no real CLIs
pnpm build         # typecheck + build
pnpm gate router   # acceptance: 10 real jobs + one forced failover
pnpm gate lead     # acceptance: the lead splits a two-part request across both plans
```

The two `gate` commands use your real plans and spend real allowance.

| Variable | Effect |
| --- | --- |
| `ORCH_CWD` | Folder the jobs run in (default: where the app was started) |
| `ORCH_RULES` | Path to your own edited copy of `src/core/router/rules.json` |
| `ORCH_LEAD` | `claude` or `cursor` to choose the lead's plan (default: the one with more headroom) |

Usage is stored locally with Node's built-in `node:sqlite`, so there is no native module to rebuild.

## License

MIT
