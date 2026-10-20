# Contributing to Legate

Thanks for looking. Legate is a small project, and help with any of the issues labelled
[good first issue](https://github.com/scholtzdaniel10/agent-orchestrator/labels/good%20first%20issue)
is very welcome. Questions are fine too: open an issue and ask.

## Set up (about five minutes)

You need Node 22.5 or newer, pnpm and Git. You do **not** need a Claude or Cursor plan to work on
Legate: the tests use fake CLIs and never spend anyone's allowance.

```bash
git clone https://github.com/scholtzdaniel10/agent-orchestrator.git
cd agent-orchestrator
pnpm install
pnpm test        # about 600 unit tests, a minute or two
pnpm dev         # the app, with hot reload
```

`pnpm shots` builds the app and takes screenshots with fake data into `.orchestrator/shots/`. It is the
quickest way to see a UI change without signing in to anything.

## Where things live

| Folder              | What is in it                                                                        |
| ------------------- | ------------------------------------------------------------------------------------ |
| `src/core`          | Everything that is not UI: routing, jobs, worktrees, goals, terminals. No Electron.  |
| `src/main`          | The Electron main process: windows, IPC, and wiring `src/core` together.             |
| `src/preload`       | The typed `window.api` bridge between the window and the main process.               |
| `src/renderer`      | The React UI. Styles and the seven themes are in `src/renderer/src/assets/main.css`. |
| `src/cli`           | The `legate` command and its terminal UI.                                            |
| `resources/plugins` | The skills Legate loads into the agents it starts.                                   |

The app, the command and the terminal UI are three ways into the same `src/core`.

## Before you open a pull request

```bash
pnpm typecheck
pnpm lint
pnpm test
```

All three must pass; CI runs them on Windows and Linux. Please also:

- Keep the change focused. A fix does not need a refactor next to it.
- Add or update a test for behaviour you change. Core code is tested with fakes, not real CLIs.
- Match the code around you: Prettier formats it (no semicolons, single quotes, 100 columns), and
  functions have explicit return types.
- UI text is short, plain and in sentence case. An error says what happened and what to do next.
- Colours come from the CSS variables in `main.css`, never hard-coded, so every theme keeps working.
- For a UI change, a screenshot in the pull request helps a lot.

## Things that are easy to get wrong

- **Never spend allowance in tests.** `pnpm test` must run without any CLI signed in. The `pnpm gate`
  commands use real plans on purpose and are only run by hand.
- **Never touch credentials.** Legate starts the official CLIs, which the person signs in to
  themselves. Legate must not read, store or send their tokens.
- **Agents that change files work in their own worktree**, never in the person's project folder.

## Releases

The maintainer bumps `"version"` in `package.json`, tags `vX.Y.Z` and pushes the tag. GitHub Actions
builds a draft release with the Windows installer.

## License

By contributing you agree that your work is released under the [MIT license](LICENSE).
