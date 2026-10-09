# Backlog

Ordered. The top item is the next one to build.

## Done

- Saved chats and jobs, per project, with past chats listed in the sidebar.

## Next

1. **Parallel jobs per plan.** A plan runs one job at a time today; a second job waits as "queued". Let each plan run up to three at once (a number in `rules.json`). Editing jobs already have their own worktree.
2. **Keep talking to the lead while its jobs run.** The lead's turn lasts until every job it handed out has finished, and the Lead box is locked for all of it. Have the lead hand out its jobs and end the turn, then prompt it again with the results when they are in.
3. **First-run check.** Say clearly when the `claude` or Cursor CLI is missing or signed out, and how to fix it. The app must work with only one of them.
4. **Stop a job.** A button to cancel a running or queued job.
5. **GitHub through `gh`.** Add a project by picking one of your repos to clone; open a pull request from a merged change. Hidden when `gh` is not installed.
6. **Delete and rename chats.**

## Before a release

- Run every feature against the real CLIs, on a clean clone.
- Review what a worker may do when "Let it edit files" is on, and the Electron security settings.
- State the tested CLI versions; fail with a readable message when their output changes.
- CI on pull requests; installers and auto-update from GitHub Releases.

## Later

- Codex as a third worker (needs its own subscription).
- Save the direct Claude and Cursor terminal sessions.
- Search across chats and jobs.
- Browser tools, Linear, a mobile companion.
