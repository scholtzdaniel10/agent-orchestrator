# winget

Manifests for the Windows Package Manager, ready to submit to
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs). Once accepted, people install with:

```bash
winget install scholtzdaniel10.Legate
```

## Check them

```bash
winget validate --manifest packaging/winget/manifests/s/scholtzdaniel10/Legate/0.1.0
```

## Submit (once per version)

1. Fork `microsoft/winget-pkgs` on GitHub.
2. Copy `manifests/s/scholtzdaniel10/Legate/<version>/` into the same path in the fork.
3. Open a pull request to `microsoft/winget-pkgs`. Their bots install it in a sandbox and scan it; a
   moderator merges it, usually within a few days.

For a new version, copy the folder, change `PackageVersion`, the URL and `InstallerSha256` (GitHub
shows it on the release asset), and validate again. `wingetcreate update` can do this in one step.
