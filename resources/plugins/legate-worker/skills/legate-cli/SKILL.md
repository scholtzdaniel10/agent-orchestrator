---
name: legate-cli
description: >-
  Read Legate's state from a terminal with the `legate` command: running and finished jobs, a job's
  steps and result, open changes and their diffs, worktrees. Use when you are an agent inside Legate
  and need to know what other workers did or are doing, or which change a worktree belongs to.
---

# Reading Legate with `legate`

`legate` is Legate's command line. From an agent, use only the commands below: they read state and
change nothing. Starting jobs, merging and discarding are for the person and the lead, not for you.

| Command | Shows |
|---|---|
| `legate status` | Each plan (Claude, Cursor): ready, busy, or resting, and what is queued |
| `legate jobs --limit 10` | Recent jobs with id, worker, state, and the start of the prompt |
| `legate show <job-id>` | One job: prompt, every step it took, and its result |
| `legate changes` | Open changes (one per worktree) with their files |
| `legate diff <change-id>` | The full diff of one change |
| `legate worktree list` | Worktrees with name, branch and folder |
| `legate worktree path <id-or-name>` | The folder of one worktree |

Add `--project <path>` when you are not inside the project or one of its worktrees.

## Rules

- Do not run `legate job`, `legate routine run`, `legate merge`, `legate discard` or `legate pr`.
  If work should be handed out, say so in your answer and let the person or the lead do it.
- Read another worker's diff to avoid editing the same lines, not to copy or redo its work.
- If `legate` is not found, say so and continue without it. Do not install anything.
