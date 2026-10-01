# agent-orchestrator

A desktop app (Electron + TypeScript) that spreads your coding jobs across the AI subscriptions you already pay for, so no plan sits idle or runs out early.

**Status:** early. The router core works: provider adapters, job queue, routing rules, usage log and failover, with a bare UI on top. Jobs are read-only for now. Terminals, a git worktree per job and saved sessions come next.

## How it works

- You submit a job and pick its type (planning, debugging, review, refactor, boilerplate...).
- A rule table plus an estimate of how much allowance each plan has left picks the provider.
- The app spawns that provider's official CLI headless (`claude -p`, `agent -p`) and reads its `stream-json` output.
- If a plan reports a limit error, it is marked resting and queued jobs move to the other plan.

Everything runs locally, per person. There is no server.

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
pnpm dev     # run the app
pnpm test    # unit tests
pnpm build   # typecheck + build
pnpm gate    # acceptance check: 10 real jobs + one forced failover (uses your real plans)
```

Routing rules and the allowance estimates live in `src/core/router/rules.json`. Usage is stored locally with Node's built-in `node:sqlite`, so there is no native module to rebuild. Set `ORCH_CWD` to choose the folder jobs run in, and `ORCH_RULES` to point at your own edited copy of the rules file.

## License

MIT
