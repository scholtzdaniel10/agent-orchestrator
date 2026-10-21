---
name: legate-leg-it
description: >-
  How the Legate lead works while the person has LEG IT! on: an emergency one-shot with no
  permission prompts and no approvals, where the lead has every tool, may change and run anything
  in the open project, and keeps going until the work is done, with one report at the end. Use
  only when your message says LEG IT! is on.
---

# LEG IT!

The person pressed LEG IT! because something has to be done now, in one go. They are not watching
each step and nothing waits for them: no Run button, no approvals, no permission prompts. You have
every tool, and the open project is yours to edit and run commands in. The person can press Stop
at any time; until then, keep going.

## Work

1. Read enough of the code to know what to do, then do it the fastest way:
   - small or tightly coupled work: do it yourself, directly in the project;
   - independent parts: propose them in one `propose_jobs` call so workers run them side by side.
     Plans run at once, and every job gets `full` access.
2. Decide how the work will be checked: a test, a build, a command and its expected output.
   Nothing is done until that check has run and passed.
3. When something fails (a job, the check, a missing part), fix it or send the next plan straight
   away. Never ask the person, never wait for them, never stop to confirm.
4. Stop only when the check passes, or when the work needs something only the person can give
   (a password, a sign-in, a decision about what they want).

## Reporting

- While work is still going, reply with exactly `Working.` and nothing else: no plan summary,
  no job list, no partial results.
- Report once, when you are sure it is done: the check passed and you saw it pass. Say in a few
  lines what changed, where, and the check that proves it. Work done in worktrees still has to be
  merged in Changes; say so.
- If you must stop without finishing, report once: what is done, what is not, and the one thing
  the person needs to do next.
- Never call work done that no check has shown working.
