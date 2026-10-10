# Backlog

Ordered. The top item is the next one to build.

## Done

- Saved chats and jobs, per project, with past chats listed in the sidebar.
- Parallel jobs per plan (`maxParallel` in `rules.json`).
- First-run check. Say clearly when the `claude` or Cursor CLI is missing or signed out, and how to fix it. The app works with only one of them.
- Stop a job. A button to cancel a running or queued job.
- GitHub through `gh`: clone a repo into the projects list (hidden when `gh` is missing or signed out).
- Delete and rename chats.
- Open a pull request from a merged change via `gh`.
- CI on pull requests; Windows installer and auto-update from GitHub Releases.
- Tested CLI versions are stated; a readable error when a CLI's output changes.
- Terminal sessions are saved per project and come back after a restart.
- Reviewed what a worker may do and the Electron security settings; Claude workers are denied a shell and the person's MCP servers.
- Live step list for each job: every file a worker reads or edits and every command it runs, with the diff of each edited file.

## Next

1. Run the lead chat and a Claude job against the real CLIs from an installed build (the Cursor path is covered by `GATE_WORKERS=cursor pnpm gate worktrees`).
2. Sign the Windows installer so SmartScreen stops warning.
3. macOS and Linux builds.

## Later

- Codex as a third worker (needs its own subscription).
- Run agents on a remote machine over SSH.
- Search across chats and jobs.
- Browser tools, Linear, a mobile companion.
