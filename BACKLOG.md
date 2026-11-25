# Backlog

Ordered. The top item is the next one to build. Shipped work is in `CHANGELOG.md`.

The rule: finish the product first, then cut one release at the end. Until then everything lands on `main` and is checked locally (fmt, clippy for native and wasm, tests).

## Now

- Decide what the benchmarks should show: Legate against the plain CLIs, a public scoreboard, or numbers for tuning the router.
- Decide whether a cancelled plan (today, Cursor) stays as a plan that's switched off or is hidden by default.

## Release (once Now is empty)

1. Publish 1.0.0 and check the installer on a clean machine.
2. Turn on auto-update (`UPDATER_ENABLED` in `apps/desktop/src-tauri`; needs a release endpoint and signing key).
3. Sign the Windows installer so SmartScreen stops warning.
4. macOS and Linux builds.
5. Open a pull request against real GitHub from an installed build.

## Later (not decided for 1.0)

- Run agents on a remote machine over SSH.
- Browser tools.
- Linear.
- A mobile companion.
