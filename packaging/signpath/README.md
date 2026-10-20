# Free code signing through SignPath Foundation

SignPath Foundation signs open-source releases for free. The key stays in their HSM; our GitHub
Actions workflow sends each unsigned installer to them and gets the signed one back. Their current
requirements and the application form are on [signpath.org](https://signpath.org). Check them there
before applying, since they may have changed since this was written.

## Before applying

- [x] OSI-approved license: MIT ([LICENSE](../../LICENSE)).
- [x] Releases are built in public CI from a tag: [.github/workflows/release.yml](../../.github/workflows/release.yml).
- [x] Public source, public issue tracker, a first public release (0.1.0).
- [ ] Two-factor authentication on the GitHub account that can push tags (Settings → Password and
      authentication).
- [ ] The code signing policy below is published in the main README (they ask for it once approved).

## Answers for the application form

- **Project name:** Legate
- **Repository:** https://github.com/scholtzdaniel10/agent-orchestrator
- **License:** MIT
- **What it is:** A Windows desktop app (Electron + TypeScript) that runs the official Claude Code
  and Cursor CLIs on the user's own machine, each in its own git worktree, and shows every step
  they take. It hands one request to several agents and lets the user review and merge each
  change.
- **What gets signed:** The NSIS installer `legate-<version>-setup.exe` built by GitHub Actions on
  `windows-latest` from a `v*.*.*` tag, and the executables inside it.
- **Why signing matters:** Without a signature Windows SmartScreen warns on every install, which is
  the main thing stopping people from trying an open-source tool from a new maintainer.
- **Maintainer:** scholtzdaniel10 (sole maintainer; reviews and approves every release).

## Code signing policy (to add to the main README once approved)

> Free code signing provided by [SignPath.io](https://signpath.io), certificate by
> [SignPath Foundation](https://signpath.org).
>
> - Committers and reviewers: [scholtzdaniel10](https://github.com/scholtzdaniel10)
> - Approvers: [scholtzdaniel10](https://github.com/scholtzdaniel10)
>
> Only installers built by this repository's release workflow from a tagged commit are signed.
> Legate sends nothing to any server of its own; see [Terms and plans](#terms-and-plans) for what it
> runs on your machine.

## After approval

SignPath gives an organization id, a project slug, a signing policy slug and an API token. Then:

1. Add the token as a repository secret named `SIGNPATH_API_TOKEN`.
2. In the release workflow, after `electron-builder --win`, upload the installer as an artifact and
   call SignPath's GitHub action (`signpath/github-action-submit-signing-request`) with those ids,
   waiting for the signed file.
3. Upload the signed installer to the release in place of the unsigned one, and regenerate
   `latest.yml` so its checksum matches the signed file (auto-update checks it).
