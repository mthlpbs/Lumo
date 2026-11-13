# WinGet and Scoop distribution

**WinGet publishing is paused. Only Scoop publishing runs.** The workflows have
no WinGet publishing job or token forwarding, including manual runs with
`publish` on. Its local tooling and manifest generation are retained
for a future explicit re-enable; adding a token does not resume it.

Local manifest tooling is retained for both package managers. Lumo is **not yet
listed in WinGet or a Scoop bucket**. Adding workflow files alone does not
publish a listing; complete the one-time setup below, commit/push these changes,
then run the package-manager workflow against an existing stable release or
publish your next app release normally.

## What happens after a release

`ci` builds/tests the app, saves its three Actions artifacts for **seven days**,
and attaches the installer, portable ZIP and SHA256SUMS.txt to the GitHub release.
After that **upload job succeeds**, its final job calls
`.github/workflows/package-managers.yml`. This avoids racing the initial release
event, when the packages do not yet exist. Manual CI dry runs, failed builds,
drafts, prereleases and non-version add-on releases do not publish package-manager
updates. Reusable-workflow invocation also avoids relying on a second release
event from a GITHUB_TOKEN-generated release.

The package-manager workflow:

1. Reads the exact published stable release; numeric `year.monthday.build` and
   historical `vyear.monthday.build` tags are supported.
2. Downloads both existing packages and verifies their SHA-256 against the
   release checksum file. It checks that the portable ZIP contains Lumo/lumo.exe
   and no unsafe paths. It never builds Lumo or changes its version.
3. Saves the generated WinGet/Scoop manifests as the small
   **lumo-package-managers** artifact, also retained for seven days.
4. If Scoop is configured, creates/updates `bucket/lumo.json` in your own Scoop
   bucket. No WinGet PR is submitted while publishing is paused.

Missing credentials skip that publisher and are reported in the run summary.
Once configured, a publishing failure is visible as a failed job; it does not
remove the already uploaded app release. You can retry without rebuilding.
Already merged WinGet versions and identical open submission PRs are skipped.
Scoop skips identical/newer versions and refuses conflicting same-version data
or a downgrade after concurrent releases.

The checked-in examples under `packaging/package-managers/` were generated from
the existing **2026.1107.2** release and its verified checksums. They are reviewable
initial submissions, not evidence that either listing has been published.
Every workflow run generates fresh manifests from its selected release assets.

## One-time WinGet setup

The following is retained for later. Do not perform it to enable publishing
now; the maintainer must first explicitly restore the workflow's publishing job.

The package identifier is **mthlpbs.Lumo**. WinGet uses the native x64, per-user
NSIS setup produced by CI, with `/S` for unattended installation. Its uninstall
entry is `Lumo`, matching installer/lumo.nsi. Do not substitute a locally built
Inno installer under the same release filename; the manifest describes NSIS.

1. In your GitHub account, open **Settings > Developer settings > Personal access
   tokens > Tokens (classic)**. Create a token with **public_repo** only.
   WinGetCreate currently requires a classic token; do not use a fine-grained
   token for this publisher. Set an expiry and renew the secret when needed.
2. In **mthlpbs/Lumo > Settings > Secrets and variables > Actions > Secrets**, add
   the token as **WINGET_TOKEN**. Do not commit or paste it into a workflow.
3. The pinned Microsoft WinGetCreate tool creates/uses your winget-pkgs fork and
   opens a PR in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).
   If Microsoft requests a contributor agreement, follow the PR bot's CLA link.
   Address any initial installer/metadata validation feedback on that PR.
4. The listing becomes available only after upstream validation and merge.
   This workflow submits updates; it cannot guarantee Microsoft's acceptance
   or immediate availability in the WinGet source.

After the first PR has merged and propagated:

```powershell
winget install --id mthlpbs.Lumo --exact --source winget
winget upgrade --id mthlpbs.Lumo --exact --source winget
```

Any restored job must pass the secret through **WINGET_CREATE_GITHUB_TOKEN**,
never a `--token` argument, and use a pinned, SHA-256-verified WinGetCreate
download. The paused job used v1.12.13.0 with its required .NET 9 runtime.
This remains CI tooling only, not a dependency bundled with Lumo.

References: [Microsoft submission process](https://learn.microsoft.com/windows/package-manager/package/winget-validation),
[WinGetCreate token permissions and environment variable](https://github.com/microsoft/winget-create/blob/main/doc/token.md).

## One-time Scoop setup

Scoop will use **your own public bucket**, so no Scoop community merge is
required. This does not automatically add Lumo to ScoopInstaller/Extras or Main.
Users add your bucket once before installing. The manifest uses the portable ZIP
and extracts its Lumo/ directory, including the optional WebView2 loader and
licenses shipped in that release. User data remains outside Scoop in APPDATA.
Close a running Lumo before installing/updating it with Scoop.

1. Create **mthlpbs/scoop-bucket** as a **public** GitHub repository with a README
   so its default branch exists. Use `main` as its default branch, or set the
   optional branch variable below to its actual default branch.
2. Create a **fine-grained personal access token** for that bucket repository
   only, with **Contents: Read and write** (Metadata read is automatic).
3. In Lumo's **Actions > Secrets**, add it as **SCOOP_BUCKET_TOKEN**.
4. In Lumo's **Actions > Variables**, set **SCOOP_BUCKET_REPO** to
   **mthlpbs/scoop-bucket**. Optionally set **SCOOP_BUCKET_BRANCH** if its default
   branch is not `main`. The publishing branch must be the bucket's default
   branch, which Scoop clones. That branch must allow this token to commit
   `bucket/lumo.json`; a rule requiring all writes through PRs blocks this mode.

Equivalent repository configuration using GitHub CLI (tokens are entered in
the secret prompt, not the command line):

```powershell
gh repo create mthlpbs/scoop-bucket --public --add-readme --description "Scoop bucket for Lumo"
gh variable set SCOOP_BUCKET_REPO --repo mthlpbs/Lumo --body mthlpbs/scoop-bucket
gh secret set SCOOP_BUCKET_TOKEN --repo mthlpbs/Lumo
```

After the first successful Scoop publishing job:

```powershell
scoop bucket add lumo https://github.com/mthlpbs/scoop-bucket
scoop install lumo/lumo
scoop update lumo
```

Use Scoop for upgrades of this installation. Lumo's optional built-in Setup
updater targets the ordinary LOCALAPPDATA/Programs installation instead. This
bucket is updated by the release workflow, so it needs no scheduled polling or
checkver job. See [Scoop buckets](https://github.com/ScoopInstaller/Scoop/wiki/Buckets)
and [manifest properties](https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests).

## First submission, preview and retry

After these workflow/script files are committed and pushed to Lumo's default
branch, open **Actions > Package managers > Run workflow**:

- **release_tag:** an existing stable release, for example `2026.1107.2`.
- Leave **publish** off first to review the generated manifests. This does not
  open a PR, commit to the bucket or create any app packages.
- With Scoop secrets/variables configured, run again with **publish** on to submit
  the first Scoop manifest. WinGet stays paused. Future stable app releases
  invoke this automatically after their asset uploads finish.

CLI equivalents:

```powershell
# Preview from existing release downloads; no build or publication.
gh workflow run package-managers.yml --repo mthlpbs/Lumo --ref main -f release_tag=2026.1107.2 -f publish=false
# Publish to Scoop, or retry its failed publisher without a rebuild.
gh workflow run package-managers.yml --repo mthlpbs/Lumo --ref main -f release_tag=2026.1107.2 -f publish=true
```

To generate locally without GitHub writes:

```powershell
python tools/package_managers.py generate --tag 2026.1107.2 --output .tools/package-manager-preview
```

If a publisher fails, fix its token/repository permissions or upstream feedback
and rerun that failed job or this workflow. Do not delete/recreate an app release
or rebuild packages simply to retry a package-manager submission. Public release
download URLs must stay immutable for a version, since clients verify hashes.

## Resource cost

Only release automation and manifest metadata change here. No app crate,
executable payload, runtime memory, idle work or per-frame work is added.
The extra Actions artifact contains only a few KiB of manifests and lasts seven
days. No WinGet Windows job or tool download runs. Scoop
still runs only when publication is requested and its configuration is present.
