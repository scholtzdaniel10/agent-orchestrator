![agent-orchestrator: one lead, every plan you already pay for](docs/banner.png)

# agent-orchestrator

A desktop app (Electron + TypeScript) that spreads your coding jobs across the AI subscriptions you already pay for, so no plan sits idle or runs out early.

![The main window: projects, the flow strip, the lead chat with the jobs it delegated, the job list and plan usage](docs/screenshot.png)

| Compare two plans side by side                                                  | Light mode                                                         |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| ![One prompt sent to Claude and Cursor, results side by side](docs/compare.png) | ![The same window in light mode, red ink on paper](docs/light.png) |

The screenshots use fake data and are made with `pnpm shots`.

**Status:** 1.0.0. Windows installer is on GitHub Releases. The pieces below are unit-tested; they have had little use against the real CLIs so far. See [BACKLOG.md](BACKLOG.md) for what is next.

## How it works

- **Lead chat.** You talk to one lead bot. It splits your request into jobs and hands them to workers, and you can keep talking to it while they run. When they finish it reports back. The lead is an ordinary CLI session that can only call the app's own tools.
  Each reply lists the jobs it handed out: which worker got it, why, and how it is going.
- **Saved chats and jobs.** Conversations and the job list are kept per project in a local SQLite file, and past chats are listed in the sidebar.
- **Projects.** A sidebar lists the folders you work in. One is active at a time; jobs and the lead run there. If the GitHub CLI (`gh`) is signed in, you can also clone one of your repos as a project.
- **Workers.** You can also hand a job straight to a worker and pick its type (planning, debugging, review, refactor, boilerplate). Each plan runs up to three jobs at once, and a job can be stopped.
- **Compare.** Choose "both (compare)" as the worker to send the same prompt to both plans, read the results side by side, and merge one change while discarding the other.
- **Changes.** A job allowed to edit files works in its own git worktree. You review the diff, then merge it as staged changes or discard it. After a merge, you can open a pull request from those staged changes.
- **Router.** A rule table plus how much allowance each plan has left picks the provider. Claude's headroom comes from the usage figure its CLI reports.
- **Adapters.** The app spawns each provider's official CLI headless (`claude -p`, `agent -p`) and reads its `stream-json` output.
- **Failover.** If a plan reports a limit error, it is marked resting and its queued jobs move to the other plan.
- **Usage meter.** The window shows how much allowance each plan has left, and says what to run when a CLI is missing or signed out. The app works with only one of the two plans.

Everything runs locally, per person. There is no server.

## The MCP bridge

The lead reaches the app through a small MCP server inside the main process, with four tools: `list_workers`, `send_job`, `get_status` and `get_result`. It listens on loopback only, requires a random token that changes every start, and refuses requests that carry a browser `Origin`.

## Subscription use

- It runs the unmodified official CLIs, each signed in by you with your own plan.
- It never reads, stores or forwards login tokens or session credentials.
- It does not use any Agent SDK and makes no pay-per-token API calls.
- Jobs are started by a person, not on a timer.

You are responsible for staying within each provider's terms.

## Security

- Everyone uses their own plans. The app has no accounts, no server and no way to reach anyone else's login.
- The window is sandboxed, cannot navigate away from the app, and reaches the main process only through a fixed list of functions.
- A worker reads files only, unless you tick "Let it edit files"; then it edits inside its own git worktree and still cannot run shell commands.
- Only add or clone folders you trust. A project can carry its own Claude Code or Cursor settings, including hooks that run commands when an agent works there.
- The terminals are full CLI sessions; what they may do is whatever you approve inside them.

## Install

Download the `-setup.exe` from [GitHub Releases](https://github.com/scholtzdaniel10/agent-orchestrator/releases). Windows only for now. Windows SmartScreen will warn because the installer is not signed: click **More info**, then **Run anyway**. You need the `claude` and/or Cursor `agent` CLI installed and signed in. Updates download in the background and install themselves when you quit.

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

### Release

Bump `"version"` in `package.json`, tag `vX.Y.Z`, and push the tag. That builds a draft GitHub Release with the Windows installer; publish the draft when you are ready.

| Variable     | Effect                                                                               |
| ------------ | ------------------------------------------------------------------------------------ |
| `ORCH_CWD`   | Folder the jobs run in (default: where the app was started)                          |
| `ORCH_RULES` | Path to your own edited copy of `src/core/router/rules.json`                         |
| `ORCH_LEAD`  | `claude` or `cursor` to choose the lead's plan (default: the one with more headroom) |

Usage is stored locally with Node's built-in `node:sqlite`, so there is no native module to rebuild.

## License

MIT
