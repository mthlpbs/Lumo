# Windows updates

All update controls are in **Settings > About**. The version and update status
stay together; there are no data-folder or release-note links on this page.

- **Check for updates** checks now, even when both preferences are off.
- **Download update** downloads a newer stable Windows installer. Once its
  size and SHA-256 match, **Install and restart** becomes available.
- **Automatically update Lumo** checks in the background, downloads a verified
  installer, and installs it on the next launch. It never closes an active
  session automatically. Switching it off cancels the pending installation.
- **Tell me about new versions** controls the island notification separately.
  Its notification opens Settings > About. Notifications alone do not download
  installers.

Both preferences default to off. A Microsoft Store copy displays that Windows
manages its updates and does not use this updater.

## Source and verification

The app reads `https://api.github.com/repos/mthlpbs/Lumo/releases?per_page=10`
over HTTPS. It skips drafts, pre-releases, add-on tags, and releases whose
versioned installer has not finished uploading. It compares numeric version
components, so date-based build counters sort correctly.

The installer must be named `Lumo-Setup-<version>.exe` and belong to that
version's release in `mthlpbs/lumo`. The GitHub response must supply a valid
SHA-256 digest and size between 1 byte and 32 MiB. Missing or invalid integrity
metadata fails the check instead of allowing an unverified installer.
Responses are bounded to 1 MiB. No GitHub token is required by the app.

Downloads are written through a `.part` file in `%LOCALAPPDATA%\Lumo\update`.
Their size and SHA-256 are checked before accepting them, and again immediately
before execution. Cached state cannot authorize another repository or an
arbitrary executable name. Install uses `/S /reopen`; the app saves and quits,
the per-user NSIS installer preserves Start with Windows, then reopens Lumo.
An installation failure leaves the app usable and offers a retry.

Trust comes from HTTPS to GitHub's own API and this repository's release
metadata. The asset hash checks file integrity; it is not an Authenticode
publisher signature. Legacy signed-feed parsing and signature fixtures remain
for historical tests, but upstream feed URLs and keys are not used at runtime.
`UPDATE_FEED_KEY` stays untouched in the `release` environment.

## Idle cost

There is no permanent updater thread, HTTP client, or animation. A check or
download creates a short-lived worker only when requested. The two preferences
share a Windows timer: the first automatic check waits two minutes after start,
then checks run every 12 hours. This interval limits requests while still
noticing new releases within half a day. Failures retry after one hour.

The timer and automatic downloads rest while the session is locked, the display
is off, or battery saver is on; Windows notifications resume eligible work.
Explicit manual checks and downloads can still run on battery saver. Nothing
is scheduled with both preferences off. Painting changes only on input or
worker completion, including while a request is in progress.

## Releases

Publish a matching version tag and release after committing the workflow.
The `ci` workflow attaches the installer and checksums. Once a stable release
has its uploaded installer and GitHub digest, existing copies can discover it.
There is no update-feed or website deployment. A local `dist` build alone does
not publish an update to other installations.

The upstream approach is documented in
[Oyenza Studio's updater](https://github.com/lucacadalora/oyenza-studio/blob/main/docs/updates.md).
Lumo retains its installer verification and restart flow, with opt-in
preferences and its own GitHub releases as the source.
