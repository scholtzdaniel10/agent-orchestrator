---
name: legate-review
description: >-
  Review a change a Legate worker made before the person merges it: read the diff, run the checks,
  name the risks, and end with merge, discard, or a precise fix request. Use when asked to review,
  check, or compare a worker's change or worktree in Legate.
---

# Reviewing a Legate change

A change is one worktree's diff against where it branched off. The person merges or discards it in
Legate's Changes panel; your review is what they decide on.

## Steps

1. **Read the task.** Find what the worker was asked (`legate show <job-id>`, or the person's words).
   Judge the diff against that task, not against what you would have built.
2. **Read the whole diff** (`legate diff <change-id>`, or `git diff` from merge-base in the worktree).
   Note every file outside the task's scope.
3. **Run the checks** the project has: tests, type check, lint. Run them in the worktree, never in
   the person's project folder. Give each command and its result.
4. **Look for the usual breakage:** removed or weakened tests, swallowed errors, secrets or
   machine-specific paths, leftover debug output, scratch files, edits to lock files or config
   the task did not need.

## Verdict

End with exactly one of:

- **Merge:** what it does, and the checks that passed.
- **Fix first:** a numbered list. Each item names the file and line, what is wrong, and what to do.
  Precise enough that a worker who never saw this review can do it.
- **Discard:** why it cannot be saved, in one or two lines.

When comparing several worktrees for the same task, rank them, name the best one, and say what (if
anything) is worth taking from the others.
