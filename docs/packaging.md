# Local Windows packages

Run from PowerShell:

```powershell
./tools/package.ps1
```

The script updates the version, tests the app, builds an optimized Windows x64
executable, and creates these files in `dist/<version>/`:

- `lumo.exe`: unpacked portable executable.
- `Lumo-<version>-portable.zip`: the executable, WebView2 loader and licenses inside `Lumo/`.
- `Lumo-Setup-<version>.exe`: per-user setup installer.
- `WebView2Loader.dll`: Microsoft's small locator for the installed shared Runtime.
- `LICENSE`, `THIRD-PARTY-NOTICES.txt`, and `SHA256SUMS.txt`.

Keep WebView2Loader.dll beside lumo.exe for optional embedded web search.
The browser engine itself is never packaged. The unpacked executable keeps
its required name, `lumo.exe`. There is no
unpacked `dist/Lumo` directory. Version-specific output paths allow an older
running copy to remain open while the new packages are prepared.

## Version behavior

Versions are `year.monthday.build`. The date defaults to Sri Lanka time
(UTC+05:30). A later date starts at build 1; a changed build on the same date
increments the final number. All six version files are updated together:
Cargo.toml, the workspace packages in Cargo.lock, the Windows resources and
manifest, and both installer scripts. Date validity, leading zeros and Windows'
65535 field limit are checked before files are written.

A source fingerprint is recorded locally in `.tools/package-state.json`.
Unchanged rebuilds, including retries after a failed build, keep their version.
Changes to version fields alone and release-note edits do not cause another
bump. The state file contains the source fingerprint and packaging status.

```powershell
./tools/package.ps1 -KeepVersion                # explicitly reuse current version
./tools/package.ps1 -BumpVersion                # intentionally start another build
./tools/package.ps1 -Version 2026.1108.2         # choose an exact valid version
./tools/package.ps1 -Date 2026-11-08             # reproducible build date
./tools/package.ps1 -Installer Inno             # choose an installer compiler
./tools/package.ps1 -Installer NSIS
./tools/package.ps1 -Target x86_64-pc-windows-msvc
```

The three version-selection flags are mutually exclusive. `-SkipTests` is
available for packaging a change already verified separately; normal runs test
the version tool, CI version/tag validation, and the full Rust workspace.

## Prerequisites and verification

Use Windows, Python 3.11+, Rust stable and the matching x64 linker. Inno Setup
6.5.2+ or NSIS with native amd64 support is required. Auto selection prefers
Inno Setup when available, then NSIS. Standard tools are found on PATH; the
workspace's complete `.tools` GNU Rust/MinGW installation takes precedence over
a global Cargo installation, including when the global Rust host is MSVC.
`-Target x86_64-pc-windows-msvc` explicitly selects the system MSVC setup instead.
The script also finds a bundled `.tools/nsis*/compiler/makensis.exe` without
manual PATH changes. Build-environment changes, including the Rust toolchain
and NSIS directory, are restored when the script exits.

Packages are prepared in an isolated `.tools/package-<id>/` folder. The script
checks EXE and installer product versions, the app's x64 architecture, portable
archive integrity, licenses and byte-for-byte EXE identity. When `innounp` is
installed, Inno installer integrity and its extracted EXE are checked too.
Sources are checked again before publishing files to the local output folder.
A same-version output EXE already running is reused if its hash is identical;
otherwise close that copy or select a new version.

New version notes are created if missing and clearly marked for review. Edit
them before a public release. This script only creates local artifacts: it
does not commit, push, tag, publish a GitHub release, or deploy an update feed.
The normal release rules in AGENTS.md still apply.
