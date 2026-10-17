# Changelog

## 1.5.0

- Triggers start goals by themselves: every day at a time, every few hours, when CI fails on your pull request from the branch you have checked out, when a GitHub issue is opened, or when the newest run on the default branch fails. A failing run's log goes with the goal. Every goal they start still waits for your OK each round, and you get a notification.
- Turning on a GitHub trigger first records what already exists, so old issues do not start goals. A schedule does not start again while its last goal is still open.

## 1.4.0

- Goals: say what should be true and, optionally, a check command. The lead reads the code and proposes a round of jobs; you approve or edit it; the jobs run, their changes are committed to the goal's own worktree, and the check runs. It repeats, asking you each round, until the check passes, the lead judges it done, the round limit is reached, or the same failure comes twice. The finished goal waits in Changes.
- Small tasks too: add a check to a job in Jobs (for example `pnpm test -- auth`). The job runs at once; if the check fails, a fix round waits for your OK.
- A first start no longer replaces an existing settings file when it brings over data from agent-orchestrator.

## 1.3.1

- Activity lays its flow out left to right (you → lead → router → plans) when its place is wider than it is tall, such as above or below the terminals, with the event log beside or below it.
- Docked and windowed panels scroll when their content does not fit, instead of cutting it off.

## 1.3.0

- Panel tabs dock anywhere. Drag Files, Lead, Jobs, Changes, Routines, Usage, Activity or History above the terminals, below them, under the projects, or back to the right panel; empty places light up while you drag. Each dock remembers its tabs and its height.
- A tab dragged out of the window, or "Open in its own window" from its right-click menu, gets a window of its own. Closing that window brings it back.
- Right-click a tab (or Shift+F10) to move it to any dock. `Ctrl+1` … `Ctrl+8` open tabs wherever they are; Settings → General puts everything back on the right.

## 1.2.0

- History tab (`Ctrl+8`): the commit log of the selected project or worktree with a branch graph, like VS Code. Click a commit for its message and files, and a file for its diff.
- Jobs keep running when you switch project. Each runs in its own project, the sidebar shows which projects have jobs running, and a notification still arrives when one ends. Only a lead reply in progress holds a switch.
- Drag the panel tabs into any order (or Alt+Shift+Left/Right). `Ctrl+1` … `Ctrl+8` follow the order; Settings → General resets it.
- Right-click a plan in Activity to bring up its terminals.
- The open project's worktrees and chats sit indented under it, so the next project no longer looks like part of the chat list.

## 1.1.1

- The terminal UI (`legate ui`) has the app's look: round borders, the focused pane outlined in red, quiet headings with counts, plan names in their own colours, line counts in green and red, and a footer that always keeps help and quit. Its boxes fill the terminal, and the project row explains itself instead of repeating the title.
- `legate jobs`, `changes` and `routine list` say what to do when there is nothing to list; `status` colours each plan and its state.
- Each project in the sidebar shows commits ahead of or behind upstream, uncommitted files, and the lines waiting in changes.

## 1.1.0

- A Settings page (`Ctrl+,`): look, jobs at once, notifications, agents and the lead, skills, projects, terminal text size, shortcuts, and where your data lives.
- Direct chats: New chat offers the lead, just Claude, or just Cursor. A direct chat talks to one plan with no delegation, and can open in a terminal.
- A calmer look: neutral surfaces with red as the accent, softer corners, pill markers for selected rows, card-style delegated jobs and message bubbles.
- Skills for Legate's agents. Workers and terminal agents get worktree rules (keep the diff small, never touch main, end with a summary) and a read-only `legate` CLI. The lead gets how to delegate. Both get how to review a change.
- Your own skills: Settings → Skills → Add folder. They load only into agents Legate starts, never into your other Claude or Cursor sessions.

## 1.0.5

- Every agent Legate starts is told it runs in Legate. Orca installs its skills for the whole user account, so agents here tried to run `orca` commands and could even start the Orca app.

## 1.0.4

- A tool installed from Setup is found when you press Re-check. Before, Legate kept the PATH it started with, so Claude Code, the Cursor CLI or the GitHub CLI stayed "missing" until you restarted it.

## 1.0.3

- Setup opens on the first start of an install, also when every tool is already in place, so a new person sees what Legate needs and that each tool is signed in. After that it opens only when something is missing.

## 1.0.2

- Updates work. 1.0.0 and 1.0.1 never checked for a new version, so they have to be updated by hand once: download and run this installer. From 1.0.2 on, a new version downloads in the background and installs when you quit.

## 1.0.1

- A first-run setup screen: install and sign in to each tool without leaving the app.
- The Paper look is readable: its text now has strong contrast.
- The Activity tab draws the flow as a diagram again.
- Worktree change totals follow an agent working in a terminal within about two seconds.
- The release is published as one draft with all its files.

## 1.0.0

- Lead chat that splits a request into jobs, hands them to workers, and reports back when they finish.
- Saved chats and jobs, per project, with past chats listed in the sidebar.
- Projects sidebar: one active folder at a time; clone a GitHub repo into the list when `gh` is signed in.
- Direct jobs to a worker by type (planning, debugging, review, refactor, boilerplate), up to three at once per plan, and stop a running or queued job.
- Compare: send the same prompt to both plans, read the results side by side, then merge one change and discard the other.
- File edits run in a git worktree; review the diff, then merge as staged changes or discard.
- Router picks a plan from a rule table and remaining allowance; Claude headroom comes from its CLI usage figure.
- Adapters spawn the official CLIs headless (`claude -p`, `agent -p`) and read `stream-json`.
- Failover: a plan that hits a limit rests, and its queued jobs move to the other plan.
- Usage meter in the window; a first-run check says what to do when a CLI is missing or signed out. The app works with only one of the two plans.
- Delete and rename chats.
- Terminal tabs with full Claude and Cursor sessions; open tabs come back after a restart.
- Open a pull request from a merged change when `gh` is signed in.
- Each plan shows its CLI version against the tested one; a job fails with a readable message when a CLI's output is not understood.
- Claude workers are denied a shell and the person's MCP servers, whatever their own settings allow.
- Windows installer, with updates from GitHub Releases installed on quit.
- Three worker access levels (read, edit, full). Full access runs commands without asking, in a git worktree; the person sets a ceiling for jobs the lead hands out.
