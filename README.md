# agent-orchestrator

A desktop app (Electron + TypeScript) that spreads your coding jobs across the AI subscriptions you already pay for, so no plan sits idle or runs out early.

**Status:** early. Working on the router core: provider adapters, job queue, routing rules, usage log and failover.

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
```

## License

MIT
