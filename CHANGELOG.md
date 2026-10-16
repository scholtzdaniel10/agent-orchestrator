# Changelog

## 1.1.0

- Skills for Legate's agents. Workers and terminal agents get worktree rules (keep the diff small, never touch main, end with a summary) and a read-only `legate` CLI. The lead gets how to delegate. Both get how to review a change.
- Your own skills: Usage → Skills → Add folder. They load only into agents Legate starts, never into your other Claude or Cursor sessions.
- The Usage page scrolls as a whole, so the plans and Skills never squeeze each other.

## 1.0.5

- Every agent Legate starts is told it runs in Legate. Orca installs its skills for the whole user account, so agents here tried to run `orca` commands and could even start the Orca app.

## 1.0.4

- A tool installed from Setup is found when you press Re-check. Before, Legate kept the PATH it started with, so Claude Code, the Cursor CLI or the GitHub CLI stayed "missing" until you restarted it.

## 1.0.3

- Setup opens on the first start of an install, also when every tool is already in place, so a new person sees what Legate needs and that each tool is signed in. After that it opens only when something is missing.

## 1.0.2

- Updates work. 1.0.0 and 1.0.1 never checked for a new version, so they have to be updated by hand once: download and run this installer. From 1.0.2 on, a new version downloads in the background and installs when you quit.

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
