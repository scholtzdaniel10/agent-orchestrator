![agent-orchestrator: one lead, every plan you already pay for](docs/banner.png)

# agent-orchestrator

A desktop app (Electron + TypeScript) that spreads your coding jobs across the AI subscriptions you already pay for, so no plan sits idle or runs out early.

![The main window: projects, the flow strip, the lead chat with the jobs it delegated, the job list and plan usage](docs/screenshot.png)

| Compare two plans side by side                                                  | Light mode                                                          |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| ![One prompt sent to Claude and Cursor, results side by side](docs/compare.png) | ![The same window in light mode, blue ink on paper](docs/light.png) |

The screenshots use fake data and are made with `pnpm shots`.

**Status:** early. The router and the lead bot work: provider adapters, job queue, routing rules, usage log, failover, a local MCP bridge, a lead chat, terminals, and a git worktree per editing job with diff, merge and discard. Saved sessions come next.

## How it works

- **Lead chat.** You talk to one lead bot. It splits your request into jobs, hands them to workers, and reports back. The lead is an ordinary CLI session that can only call the app's own tools.
  Each reply lists the jobs it handed out: which worker got it, why, and how it is going.
- **Projects.** A sidebar lists the folders you work in. One is active at a time; jobs and the lead run there.
- **Workers.** You can also hand a job straight to a worker and pick its type (planning, debugging, review, refactor, boilerplate).
- **Compare.** Choose "both (compare)" as the worker to send the same prompt to both plans, read the results side by side, and merge one change while discarding the other.
- **Changes.** A job allowed to edit files works in its own git worktree. You review the diff, then merge it as staged changes or discard it.
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
pnpm shots         # screenshots of the UI with fake data, in .orchestrator/shots/
pnpm gate router   # acceptance: 10 real jobs + one forced failover
pnpm gate lead     # acceptance: the lead splits a two-part request across both plans
```

The two `gate` commands use your real plans and spend real allowance.

| Variable     | Effect                                                                               |
| ------------ | ------------------------------------------------------------------------------------ |
| `ORCH_CWD`   | Folder the jobs run in (default: where the app was started)                          |
| `ORCH_RULES` | Path to your own edited copy of `src/core/router/rules.json`                         |
| `ORCH_LEAD`  | `claude` or `cursor` to choose the lead's plan (default: the one with more headroom) |

Usage is stored locally with Node's built-in `node:sqlite`, so there is no native module to rebuild.

## License

MIT
