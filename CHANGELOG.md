# Changelog

## 1.0.1

- A first-run setup screen: install and sign in to each tool without leaving the app.
- The Paper look is readable: its text now has strong contrast.
- The Activity tab draws the flow as a diagram again.
- Worktree change totals follow an agent working in a terminal within about two seconds.
- The release is published as one draft with all its files.

## 1.0.0

- Lead chat that splits a request into jobs, hands them to workers, and reports back when they finish.
- Saved chats and jobs, per project, with past chats listed in the sidebar.
- Projects sidebar: one active folder at a time; clone a GitHub repo into the list when `gh` is signed in.
- Direct jobs to a worker by type (planning, debugging, review, refactor, boilerplate), up to three at once per plan, and stop a running or queued job.
- Compare: send the same prompt to both plans, read the results side by side, then merge one change and discard the other.
- File edits run in a git worktree; review the diff, then merge as staged changes or discard.
- Router picks a plan from a rule table and remaining allowance; Claude headroom comes from its CLI usage figure.
- Adapters spawn the official CLIs headless (`claude -p`, `agent -p`) and read `stream-json`.
- Failover: a plan that hits a limit rests, and its queued jobs move to the other plan.
- Usage meter in the window; a first-run check says what to do when a CLI is missing or signed out. The app works with only one of the two plans.
- Delete and rename chats.
- Terminal tabs with full Claude and Cursor sessions; open tabs come back after a restart.
- Open a pull request from a merged change when `gh` is signed in.
- Each plan shows its CLI version against the tested one; a job fails with a readable message when a CLI's output is not understood.
- Claude workers are denied a shell and the person's MCP servers, whatever their own settings allow.
- Windows installer, with updates from GitHub Releases installed on quit.
- Three worker access levels (read, edit, full). Full access runs commands without asking, in a git worktree; the person sets a ceiling for jobs the lead hands out.
