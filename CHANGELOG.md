# Changelog

## Unreleased

- **Live usage** can be turned off in Settings → General. With it off, Usage updates only when you press Refresh or a job runs.
- The README says plainly what Legate runs on its own and how it uses each plan, with links to the providers' terms.
- A contributor guide, and winget manifests ready to submit.

## 0.1.0 — first public preview

Legate is a desktop app that hands your coding work to Claude Code, Cursor and Grok, on the plans you already pay for. This is the first version anyone outside the project gets to try. Please report what breaks.

- **One lead.** Tell it what you want. It reads the code, proposes jobs as cards you can edit, and starts nothing until you press **Run**. It reports back when the agents finish.
- **Agents beside your terminals.** Each running agent gets a live tab in the middle showing every file it reads, every edit and every command. **Take over** turns it into a real terminal on the same session.
- **Three workers.** Claude plans and reviews. Grok and Cursor do the coding, both on your Cursor plan. Legate sends work to whichever plan has allowance left.
- **Worktrees for everything.** Every agent that changes files works in its own git worktree. The sidebar lists every project with its worktrees, and **Review** beside the terminals shows the diff with Merge and Discard.
- **Goals and triggers.** Say what should be true and, if you like, give a check such as `pnpm test`. Legate works round by round, asks your OK each time, and stops when the check passes. Triggers start goals on a schedule, or when CI fails, an issue is opened, or your default branch breaks.
- **Live usage.** Claude's real 5-hour and weekly usage, read every 10 minutes. Finished agents you are not looking at sleep to free memory and resume when shown.
- **A command line and a terminal UI** (`legate`, `legate job`, `legate goal`) share one record of every job with the app.
- Windows only for now. The installer is not signed yet, so Windows asks before running it.

<details>
<summary>Internal builds before the preview</summary>

#### Internal build 1.9.0

- **One tree for every project.** The sidebar lists each project with its worktrees under it. Click a worktree of another project and Legate opens that project on it. Projects fold; the open one sits at the top.
- **Usage is live.** Legate asks Claude for its real usage every 10 minutes and when you open Usage or press Refresh, so use outside Legate counts too. It costs a tiny turn on the smallest model. Cursor reports no usage; its card now says its number is an estimate.
- **Finished agents sleep.** An agent terminal you have not looked at for a while, with no job working in its worktree, stops its process to free memory (a Cursor or Grok terminal is about 500 MB). Show it again and it resumes the same session.
- **The lead's own model.** Pick it next to Runs on in the Lead tab; workers keep the model set in Usage.
- **Interface size** in Settings → General, 90% by default, so the window feels less crammed. Terminals and code use JetBrains Mono Nerd Font when it is installed.
- Memory: hidden terminals give back their GPU context, finished jobs kept in memory are capped at 200 per project, and windows no longer load spellcheck dictionaries.
- A terminal that only renames its window no longer counts as working in the sidebar.

#### Internal build 1.8.0

- **The lead plans, you press Run.** The lead now reads the code in the open project (read only) before it plans, then shows its jobs as cards: title, worker, access and prompt. Edit or remove any of them, then **Run**, or **Dismiss**. Nothing starts before you say so.
- **Agents open beside your terminals.** When a plan runs, each agent gets a live tab in the middle: every file it reads, every edit and command, and what it says. Its worktree shows up in the sidebar as usual, and the lead reports when they are all done.
- **Take over.** Any agent's tab can become a real terminal on the same session, in its worktree, so you can keep talking to it. A running agent is stopped first; its work so far stays.

#### Internal build 1.7.0

- **Grok** is a third worker: the Cursor CLI on a Grok model (Grok 4.7 High unless you pick another Grok model in Settings). It spends your Cursor plan, so it costs nothing extra: the same allowance, the same rest after a limit, and the same jobs-at-once limit as Cursor.
- Coding work (debugging, refactors, boilerplate) goes to Grok first, then Cursor, then Claude. Planning and review still go to Claude first.
- Grok has its own face, a statue head under an orbit crossed by a comet, and appears wherever you pick a worker: New task, + Grok beside the terminals, Jobs, routines, the lead, and Settings. The usage meters stay one per plan; Grok's work counts on Cursor's.
- `legate job --worker grok`; `legate status` lists Grok as using the Cursor plan.

#### Internal build 1.6.1

- In a small window the right panel and the sidebar shrink before the panel covers the terminals. The panel only floats over them when even the narrowest panels do not fit (below about 840 pixels with the sidebar shown).
- The open terminal's tab stays readable when the workspace is narrow; the pane picker drops its label instead.

#### Internal build 1.6.0

- A new sidebar, laid out like Orca: one project switcher on top, then **New task**, **Goals** and **Automations**, then your worktrees as cards grouped by state (Pinned, In progress, Needs you, Ready to review, Quiet). Each card shows its branch, its lines changed and its terminals. Goals that wait for you show a count.
- **New task** (`Ctrl+N`) is one dialog for everything: pick Cursor or Claude, then a terminal you talk to, a job in the background, or a goal. It picks Claude when Cursor is not signed in.
- **Review** sits beside a worktree's terminals: its files, its diff, and Merge or Discard, without opening the side panel. The terminals keep running behind it.
- Panel tabs show icons, with their names when the tab bar has room.
- Coding work (debugging, refactors, boilerplate) goes to Cursor first; planning and review stay with Claude.
- `legate goal "<what should be true>" [--check "<command>"] [--rounds N] [--yes]` runs a goal from the command line, and `legate goals` lists them.
- History explains an empty repository and reads again after merges and finished jobs. The lead reports jobs that ended while another project was open. The goal form can fill in an example.
- The theme picker moved from the header to Settings → General.
- The lead knows which project is open. Asked to work on another project, it tells you to open that one first instead of sending the job there. A project with no commits yet says so plainly instead of git's `invalid reference: HEAD`.
- Worktrees no longer vanish from the sidebar in a narrow window or when a panel is docked under the projects.

#### Internal build 1.5.0

- Triggers start goals by themselves: every day at a time, every few hours, when CI fails on your pull request from the branch you have checked out, when a GitHub issue is opened, or when the newest run on the default branch fails. A failing run's log goes with the goal. Every goal they start still waits for your OK each round, and you get a notification.
- Turning on a GitHub trigger first records what already exists, so old issues do not start goals. A schedule does not start again while its last goal is still open.

#### Internal build 1.4.0

- Goals: say what should be true and, optionally, a check command. The lead reads the code and proposes a round of jobs; you approve or edit it; the jobs run, their changes are committed to the goal's own worktree, and the check runs. It repeats, asking you each round, until the check passes, the lead judges it done, the round limit is reached, or the same failure comes twice. The finished goal waits in Changes.
- Small tasks too: add a check to a job in Jobs (for example `pnpm test -- auth`). The job runs at once; if the check fails, a fix round waits for your OK.
- A first start no longer replaces an existing settings file when it brings over data from agent-orchestrator.

#### Internal build 1.3.1

- Activity lays its flow out left to right (you → lead → router → plans) when its place is wider than it is tall, such as above or below the terminals, with the event log beside or below it.
- Docked and windowed panels scroll when their content does not fit, instead of cutting it off.

#### Internal build 1.3.0

- Panel tabs dock anywhere. Drag Files, Lead, Jobs, Changes, Routines, Usage, Activity or History above the terminals, below them, under the projects, or back to the right panel; empty places light up while you drag. Each dock remembers its tabs and its height.
- A tab dragged out of the window, or "Open in its own window" from its right-click menu, gets a window of its own. Closing that window brings it back.
- Right-click a tab (or Shift+F10) to move it to any dock. `Ctrl+1` … `Ctrl+8` open tabs wherever they are; Settings → General puts everything back on the right.

#### Internal build 1.2.0

- History tab (`Ctrl+8`): the commit log of the selected project or worktree with a branch graph, like VS Code. Click a commit for its message and files, and a file for its diff.
- Jobs keep running when you switch project. Each runs in its own project, the sidebar shows which projects have jobs running, and a notification still arrives when one ends. Only a lead reply in progress holds a switch.
- Drag the panel tabs into any order (or Alt+Shift+Left/Right). `Ctrl+1` … `Ctrl+8` follow the order; Settings → General resets it.
- Right-click a plan in Activity to bring up its terminals.
- The open project's worktrees and chats sit indented under it, so the next project no longer looks like part of the chat list.

#### Internal build 1.1.1

- The terminal UI (`legate ui`) has the app's look: round borders, the focused pane outlined in red, quiet headings with counts, plan names in their own colours, line counts in green and red, and a footer that always keeps help and quit. Its boxes fill the terminal, and the project row explains itself instead of repeating the title.
- `legate jobs`, `changes` and `routine list` say what to do when there is nothing to list; `status` colours each plan and its state.
- Each project in the sidebar shows commits ahead of or behind upstream, uncommitted files, and the lines waiting in changes.

#### Internal build 1.1.0

- A Settings page (`Ctrl+,`): look, jobs at once, notifications, agents and the lead, skills, projects, terminal text size, shortcuts, and where your data lives.
- Direct chats: New chat offers the lead, just Claude, or just Cursor. A direct chat talks to one plan with no delegation, and can open in a terminal.
- A calmer look: neutral surfaces with red as the accent, softer corners, pill markers for selected rows, card-style delegated jobs and message bubbles.
- Skills for Legate's agents. Workers and terminal agents get worktree rules (keep the diff small, never touch main, end with a summary) and a read-only `legate` CLI. The lead gets how to delegate. Both get how to review a change.
- Your own skills: Settings → Skills → Add folder. They load only into agents Legate starts, never into your other Claude or Cursor sessions.

#### Internal build 1.0.5

- Every agent Legate starts is told it runs in Legate. Orca installs its skills for the whole user account, so agents here tried to run `orca` commands and could even start the Orca app.

#### Internal build 1.0.4

- A tool installed from Setup is found when you press Re-check. Before, Legate kept the PATH it started with, so Claude Code, the Cursor CLI or the GitHub CLI stayed "missing" until you restarted it.

#### Internal build 1.0.3

- Setup opens on the first start of an install, also when every tool is already in place, so a new person sees what Legate needs and that each tool is signed in. After that it opens only when something is missing.

#### Internal build 1.0.2

- Updates work. 1.0.0 and 1.0.1 never checked for a new version, so they have to be updated by hand once: download and run this installer. From 1.0.2 on, a new version downloads in the background and installs when you quit.

#### Internal build 1.0.1

- A first-run setup screen: install and sign in to each tool without leaving the app.
- The Paper look is readable: its text now has strong contrast.
- The Activity tab draws the flow as a diagram again.
- Worktree change totals follow an agent working in a terminal within about two seconds.
- The release is published as one draft with all its files.

#### Internal build 1.0.0

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

</details>
