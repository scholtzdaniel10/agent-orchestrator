---
name: legate-worktree
description: >-
  House rules for an agent working inside Legate: you are in a git worktree on a branch of your own,
  and a person reviews your diff before anything reaches their project. Use at the start of any task
  in Legate that changes files, and before you say you are done.
---

# Working in a Legate worktree

Legate gives each job or terminal its own git worktree, usually under `~/.orchestrator/wt/<id>` on a
branch named `orch/<id>`. The person sees your work in Legate's **Changes** panel as one diff, then
merges or discards it. Merging stages your diff on their project; they make the commit.

## The diff is everything you leave behind

Legate diffs the whole worktree, tracked and untracked files, against where it branched off. So:

1. Change only what the task needs. No drive-by reformatting, renames or dependency bumps.
2. Delete scratch files, logs and test output you created, unless `.gitignore` already covers them.
3. Committing is optional: committed and uncommitted work end up in the same diff.

## Never

- Touch the person's project folder or any path outside this worktree.
- Switch branches, rebase, reset, or push. The branch is Legate's, and nothing goes to a remote.
- Install global tools or change config in the home folder.
- Use the `orca` CLI or skills of other agent apps: they are not running here.

## When you finish

End with a short summary the person can review in under a minute:

```
Changed: <files, one line each, with what changed>
Why: <one line>
Check: <the exact command or click that shows it works>
Not done: <anything left, or "nothing">
```

If you ran the tests, give the command and the result. If you could not, say so plainly.
