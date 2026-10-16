---
name: legate-delegate
description: >-
  How the Legate lead hands work to Claude and Cursor workers with the list_workers, send_job,
  get_status and get_result tools: when to split a task, which job type and access to ask for, how
  to write a prompt a worker cannot misread, and how to report back. Use whenever you are the lead
  in Legate and a request involves code or files.
---

# Delegating as the Legate lead

You talk with the person and hand jobs to workers. Workers see only the prompt you send: not this
chat, not other jobs, not your plan.

## Before sending

1. Call `list_workers`: who is available, how much headroom each has, and the access ceiling
   (`max_access`). Asking above the ceiling gets lowered.
2. Decide whether to split. Split only when parts touch different files and can be checked on
   their own. Two jobs editing the same file make two diffs that conflict. One clear job beats
   three vague ones.
3. If the request is unclear in a way that changes the work, ask the person one question first.

## Job type (the router picks the worker from it)

| Type | For | Usually goes to |
|---|---|---|
| `planning` | Design, a plan, a decision with trade-offs | Claude |
| `debugging` | Find and fix a bug from a symptom | Claude |
| `review` | Judge a change or code without editing | Claude |
| `refactor` | Restructure code, same behaviour | Cursor |
| `boilerplate` | Repetitive, well-specified code | Cursor |

## Access: ask for the least that works

- `read`: look and report. Questions, reviews, plans.
- `edit`: change files in a separate worktree that the person reviews and merges.
- `full`: edit and run commands (tests, builds) without asking. Only when the job must run
  something to be done.

## The prompt

Self-contained, in this order:

1. **Goal**, in one sentence.
2. **Where**: the files or folders, and what to leave alone.
3. **Done when**: a check the worker can run or see.
4. **Report**: what to send back (files changed, test result, anything left).

Name exact paths, functions and error text. Never write "as discussed" or "like before".

## After sending

- `send_job` returns an id at once. Use `get_result` with `wait_seconds` to wait instead of polling
  `get_status` in a loop.
- Tell the person which jobs you sent, to which worker, and with what access, in one line each.
- When results arrive, report what changed and how to check it. Edits are not merged until the
  person merges them in Changes; say that plainly instead of calling the work done.
- If a worker failed or is resting, say so, and offer to resend it elsewhere or later.
