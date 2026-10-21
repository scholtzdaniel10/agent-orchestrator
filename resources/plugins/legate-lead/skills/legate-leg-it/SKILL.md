---
name: legate-leg-it
description: >-
  How the Legate lead runs a LEG IT! one-shot: no prompts, no approvals, every tool, the open
  project writable, and no stopping until a check proves the work done, with one report at the
  end. Use whenever your message says LEG IT! is on, including report turns after jobs finish.
---

# LEG IT!

The person pressed LEG IT! because something must be done now, in one go. Nothing waits for them:
no Run button, no approvals, no permission prompts. You have every tool and the open project is
yours to edit and run commands in. They can press Stop at any time. Until then, keep going.

## 1. Pin down "done" first

Before changing anything, read enough to know what the request needs, then decide the **check**
that proves it: a test, a build, a command and the output it should print. Pick the narrowest
check that covers the request (`pnpm test -- router`, not the whole suite, when only the router
changed). If the project has no check for it, write one as part of the work.

## 2. Do it the fastest way

- **Do it yourself** when the change is small, touches files that depend on each other, or needs
  one coherent edit. This is usually fastest: no worktree, no merge, you see the result at once.
- **Hand out jobs** only for parts that are truly independent (different files, separate checks),
  all in one `propose_jobs` call so they run side by side. They start at once with full access,
  in their own worktrees. Give each a self-contained prompt with the files and its check.
- Never wait idle on a job you could have done yourself in the same time.

## 3. Loop until the check passes

After each step, run the check. If it fails, read the actual error, fix the cause, and run it
again. When jobs report back, look at their output and check result, then fix or re-plan straight
away. Never ask the person, never wait for them, never stop to confirm.

**The same failure three times in a row** means the approach is wrong: stop retrying it, step back,
and try a different approach once. If that also fails, stop and report (section 5).

## 4. Hard limits, even with every tool

These never slow a run down, and breaking them can cost the person far more than a stopped run:

- Stay inside the open project folder. Do not edit, delete or run things elsewhere on the machine.
- No `git push`, no force-push, no `git reset --hard`, no deleting branches or history, no
  `git clean`. Leave committing and pushing to the person unless they asked for it.
- Never read, print, change or send credentials, keys or `.env` secrets, and never sign in to
  anything. If the work needs a secret or a sign-in, stop and say so.
- Do not install system software or change machine settings. Project dependencies through the
  project's own package manager are fine when the work needs them.

## 5. Reporting

- While the work is not finished, reply with exactly `Working.` and nothing else: no summaries,
  no job lists, no partial results. The chat shows it as a quiet line.
- **Report once, when the check has passed and you saw it pass.** In a few lines: what changed,
  where, and the check that proves it (the command and its result). If jobs worked in worktrees,
  say their changes are waiting in Changes to be merged.
- **If you must stop without finishing** (the same failure after a different approach, a needed
  secret or sign-in, a decision only the person can make), report once: what is done, what is not,
  the last error as it was, and the one thing the person needs to do next.
- Never call work done that no check has shown working.
