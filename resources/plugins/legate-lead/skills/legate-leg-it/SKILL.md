---
name: legate-leg-it
description: >-
  How the Legate lead works while the person has LEG IT! on: an emergency one-shot where plans run
  the moment they are proposed, workers may get full access, and the person wants one report at
  the end instead of updates. Use only when your message says LEG IT! is on.
---

# LEG IT!

The person pressed LEG IT! because something has to be done now, in one go. They are not
watching each step. Plans you propose start at once; there is no Run button to wait for.

## Work

1. Read enough of the code to plan, then propose the whole plan in one `propose_jobs` call. A
   second call in the same turn is refused while the first plan runs.
2. Every plan names how it will be checked: a test, a build, a command and its expected output.
   A job that only edits is not done until a job has run that check.
3. Mind the person's computer. Every job is a full agent, and the tests or builds it runs take
   memory on top. Prefer fewer jobs: one job that edits and checks its own change beats several.
   Give `full` access only to jobs that must run a command. Each prompt names the narrowest check
   for that change (one test file or one package, not the whole suite) and says to run it once:
   no dev servers, no watch mode, nothing left running. Legate starts jobs one at a time and
   stops the newest if memory runs low; a job that ends with "Stopped to free memory" needs a
   smaller plan, not the same one again.
4. When results come back and the work is not done (a job failed, the check did not pass, a
   part is missing), propose the next plan straight away. Do not ask the person.
5. You may start at most five plans on your own per message from the person. If the fifth does
   not finish the work, stop and report.

## Reporting

- While work is still going, reply with exactly `Working.` and nothing else: no plan summary,
  no job list, no partial results.
- Report once, when you are sure it is done: the check passed, and you saw it pass in a job's
  output. Say in a few lines what changed, where, and the check that proves it. Changes made in
  worktrees still have to be merged in Changes; say so.
- If you stop without finishing (five plans used, no worker available, something only the person
  can do), report once: what is done, what is not, and the one thing they need to do next.
- Never call work done that no check has shown working.
