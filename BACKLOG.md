# Backlog

Ordered. The top item is the next one to build.

## Done

- Saved chats and jobs, per project, with past chats listed in the sidebar.
- Parallel jobs per plan (`maxParallel` in `rules.json`).
- First-run check. Say clearly when the `claude` or Cursor CLI is missing or signed out, and how to fix it. The app works with only one of them.
- Stop a job. A button to cancel a running or queued job.
- GitHub through `gh`: clone a repo into the projects list (hidden when `gh` is missing or signed out).
- Delete and rename chats.
- CI on pull requests; Windows installer and auto-update from GitHub Releases.

## Next

1. **Open a pull request from a merged change** via `gh` (clone already ships; `createPr` is ready in core).

## Before a release

- Run every feature against the real CLIs, on a clean clone.
- Review what a worker may do when "Let it edit files" is on, and the Electron security settings.
- State the tested CLI versions; fail with a readable message when their output changes.

## Later

- Codex as a third worker (needs its own subscription).
- Save the direct Claude and Cursor terminal sessions.
- Search across chats and jobs.
- Browser tools, Linear, a mobile companion.
