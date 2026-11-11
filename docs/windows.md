# Windows

## Installer

From `apps/desktop/src-tauri`, `cargo tauri build` writes a per-user NSIS installer under the workspace `target/release/bundle/nsis/` directory. `apps/desktop/src-tauri/tauri.conf.json` sets `bundle.targets` to `nsis` and `bundle.windows.nsis.installMode` to `currentUser`. When WebView2 is missing, the installer uses the download bootstrapper (`webviewInstallMode.type` is `downloadBootstrapper`).

Leave the Cargo target directory as the repository `target` folder. The Tauri config loads the built UI from `target/dx/legate-ui/release/web/public`.

## Signing

The installer is not code-signed. The Tauri config has no certificate, and this branch has no workflow that signs a build. Windows SmartScreen warns on first run: choose **More info**, then **Run anyway**.

## Updates

The updater plugin is in the shell and left off. `UPDATER_ENABLED` is false in `apps/desktop/src-tauri`, and `bundle.createUpdaterArtifacts` is false. An installed build does not check GitHub Releases.

## winget

Legate itself is not a winget package. The Setup screen offers these commands for the tools it checks:

```
winget install --id Git.Git -e
winget install --id Anthropic.ClaudeCode -e
winget install --id GitHub.cli -e
```

The Cursor CLI is installed with Cursor's script, not winget:

```
irm 'https://cursor.com/install?win32=true' | iex
```
