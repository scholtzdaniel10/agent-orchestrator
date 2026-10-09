# Backlog

Ordered. The top item is the next one to build.

## Done

- Saved chats and jobs, per project, with past chats listed in the sidebar.
- Parallel jobs per plan (`maxParallel` in `rules.json`).

## Next

1. **First-run check.** Say clearly when the `claude` or Cursor CLI is missing or signed out, and how to fix it. The app must work with only one of them.
2. **Stop a job.** A button to cancel a running or queued job.
3. **GitHub through `gh`.** Add a project by picking one of your repos to clone; open a pull request from a merged change. Hidden when `gh` is not installed.
4. **Delete and rename chats.**

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
