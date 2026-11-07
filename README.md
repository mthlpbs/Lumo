# Lumo

A lightweight living notch for Windows 10 and 11, maintained by **mthlpbs**.
Lumo is a modified version of [Oyenza Studio](https://github.com/lucacadalora/oyenza-studio).
Hover over the island to open its pages. Swipe horizontally with two fingers
on your touchpad to switch pages, or scroll over the header. Keep swiping to
move through several tabs; scrolling stops at the first and last tabs. Right-click the
island or tray icon for options.

## Features

- **Ask AI (prototype):** Markdown replies, Ctrl+hover access, an expandable compact chat and a resizable full chat window, your own provider/key/models, images/PDFs and screen snips, image generation, temporary chats and encrypted local history. [Setup and limits](docs/ai-companion.md).

- **Music:** controls for Windows media sessions, synced lyrics from LRCLIB,
  optional Apple Music lyrics, and local lyric files for desktop players.
- **Focus:** a Pomodoro timer with a countdown on the closed island.
- **Shelf:** keep links to files, drag files in or out, and send files from a
  phone over Wi-Fi. Paired phones can connect from other networks.
- **Tasks:** a small local to-do list.
- **Today:** the date and calendar.
- **System:** CPU, GPU, memory, drives, network, battery, and power information.
  Optional FPS, GPU, and CPU readouts over borderless full-screen games.
- **Add-ons:** turn **Script reader** and **Clipboard history** on or off.
  Script reader is a teleprompter under the webcam; Clipboard history keeps
  copied text in memory and skips text marked private by password managers.
- **Themes:** Classic and Pill, with volume, charging, and headset notices.

Prayer times, Health, Money, Oyenza accounts, and Japanese readings are not
included. This repository builds the Windows app only.

## Install

Run `Lumo-Setup-2026.1107.2.exe` from `dist`, or extract
`Lumo-2026.1107.2-portable.zip` and run `Lumo/lumo.exe`.
The installer uses your Windows account and needs no administrator rights.

The application uses `lumo.exe`, Lumo folders and `LUMO_*` environment variables.
Existing profiles and received-file inboxes remain readable after the rename:

- App: `%LOCALAPPDATA%\Programs\Lumo\lumo.exe`
- Settings and data: `%APPDATA%\Lumo\`
- Cache and log: `%LOCALAPPDATA%\Lumo\`
- Files received from phones: `Downloads\Lumo\`; an existing inbox from a
  previous version is reused to preserve Shelf links.

Uninstalling keeps your settings, tasks, focus history, and received files.
Existing profiles are imported into Lumo once. Original folders remain as backups;
retired themes, old logs and obsolete update metadata are not carried forward.

## Settings and options

The gear opens a separate Settings window with General, Display, Appearance,
Music & lyrics, AI companion, Focus, Notifications, and About categories. Changes apply and
save immediately. Horizontal touchpad scrolling also switches Settings categories.
Close Settings with the button at the top right; the island keeps running.
The compact island stays visible over normal and split-screen windows. Over
a maximized app, hover at the top edge to reveal it, then click to open.
Choose **Settings > General > Open island with** to enable hover opening over
normal windows; maximized windows still require a click. Maximized-window
visibility remains configurable in **Display**.
The tray menu contains only **Start with Windows**,
**Settings**, and **Exit**. Right-clicking the island has no action.
When an app is maximized, the window icon beside the gear keeps the island
visible over that app. Its tooltip names the app. Lumo remembers that choice until
you untick the icon; the pin beside it keeps the expanded island open.

Music uses a grey lyrics speech-bubble button for compact-island lyrics; the
icon is crossed out when disabled. The expanded player and popup retain neighbouring cues
around the current lyric, including in compact popup sizes. When lyrics
are available, the grey pop-out button at the upper right opens a resizable player at
the top right, with album-coloured backgrounds, synced lyrics and playback
controls. Drag an edge to resize it; the size is remembered. Hover the corner
dot to reveal Close and return to Music on the island. The island stays visible
while the popup is open; its compact lyrics and expanded lyrics column are
temporarily hidden and return when the popup closes. While it is open, the
lyrics toggle is disabled and the pop-out button points inward; click it to
close the popup and return lyrics to the island.
Hide the pop-out button
in **Settings > Music & lyrics > Lyrics window button**.

### Add-ons

Open the **+ (Add-ons)** tab and switch **Script reader** on. Its Script tab
appears and its shortcut becomes available. Switch it off to hide the tab and
stop the reader. Clipboard history has its own switch on the same page.
Both add-ons are built in and require no download.

### Local lyrics

Turn on **Use local lyrics** in **Settings > Music & lyrics** and choose your music
folder. Local music tries sidecar `.lrc`, `.lyrics`, and `.txt` files, then
embedded MP3 (USLT/SYLT), FLAC, M4A and Opus/Ogg lyrics in matching song files.
Other audio formats can use Windows' installed metadata handlers. LRC stored
inside a song retains its timestamps; plain embedded lyrics remain unsynced.
Lookups read bounded metadata on demand and never modify your songs. Choose
the containing music folder, using title or Artist - Title filenames.
LRCLIB supplies missing lyrics. **Save downloaded lyrics** automatically saves
its results using the matched audio file's exact basename and `.lrc` extension.
The default places the sidecar beside that song; a custom download folder keeps
the same basename. Matching uses filenames and audio tags in the selected Music
folder, with bounded metadata reads. Ambiguous/unmatched songs are not saved;
their LRCLIB lyrics still work, including streaming music.
Explicit badges in metadata and `(Explicit)` filename labels match each other;
clean and explicit editions remain distinct.
Change the
download folder or disable saving under **Settings > Music & lyrics > Downloaded
lyrics**. Enable local lyrics to use the Music folder shown above, or choose a
download folder. Global lyrics alone enable no file writes. Existing files
are kept. Local music can reuse saved lyrics from a separate download folder;
streaming music still asks LRCLIB directly. Source details stay in the lyrics
icon's tooltip, along with its closed-island On/Off action.
Music services such as Spotify and
Deezer use LRCLIB directly, even with local lyrics enabled. The LRC control
appears only for local music and identified music services, not other media.
Korean and Chinese pronunciation
remain available; Japanese lyrics display their original text.

### Phone drop

Open **Shelf > Phone**, scan its code with your phone, and send files over
your Wi-Fi. Accept the incoming request on your PC. Pair a phone in that panel
to send from another network. See [Paired phone drop](docs/paired-drop.md) and
the [server guide](server/README.md) for the protocol and self-hosting.

### Show over games

Enable it in **System > Games**. Windows frame tracing
requires a one-time permission to join the Performance Log Users group.
**Alt+Shift+F** hides or shows the readout while a game is in front.
Choose the brief peeks option to avoid leaving an overlay over the game.

### Updates

Open **Settings > About** to check for updates, download a newer release,
and install it with a restart. **Automatically update Lumo** downloads
verified installers and installs them on the next launch. **Tell me about
new versions** controls update notifications separately. Both are off by
default; manual checks work with both off. Automatic work rests while the
session is locked, the display is off, or battery saver is on.
See [Update implementation](docs/updates.md).

## Build and test

```powershell
cargo test --workspace
cargo build -p lumo --release
```

For a GNU cross build, `cargo wcheck` checks the Windows target and
`cargo wbuild` builds `lumo.exe` with a MinGW toolchain. The graphical self-test
uses an isolated data directory:

```powershell
.\target\release\lumo.exe --selftest selftest-out
```

Settings scenes capture the current standalone Settings window. Older saved
Settings page selections return to Music instead of restoring the retired panel.

No new background timers, network calls, or per-frame work were added for the
Lumo branding or Add-ons page. Disabled add-ons stay asleep. Features that
repeat rest while the session is locked, the screen is off, or battery saver
is on. [PRIVACY.md](PRIVACY.md) describes network and local storage behavior.

## Release builds

Versions use `year.monthday.build`: `2026.1107.1` is 7 November 2026,
build 1. Another release on that date uses `2026.1107.2`; the next day
starts at `2026.1108.1`. January 7 uses `2027.107.1`, without leading zeros.
Each release gets a new number; rebuilding the same source keeps its version.
All three fields fit Windows version numbers; the build counter is 1–65535.

Publish a GitHub release with a tag matching the app version, such as `v2026.1107.2`.
The [ci workflow](.github/workflows/ci.yml) tests and builds the Windows app,
checks the installer, and attaches `Lumo-Setup-<version>.exe`,
`Lumo-<version>-portable.zip`, and `SHA256SUMS.txt` to that release.
The release's title and notes stay as you wrote them. Saving a draft does not
start a build; publishing a release or pre-release does.

Commit the workflow before creating the release tag. Update `Cargo.toml`,
`Cargo.lock`, `crates/lumo/res/lumo.rc`, `crates/lumo/res/lumo.manifest`,
`installer/lumo.nsi`, and `installer/lumo.iss` together before tagging.
Add `docs/release-notes/v<version>.md` and use it for the release text.
The workflow rejects mismatched versions. Non-`v` tags are reserved for add-ons and skipped.

Re-running a failed release job uses its original tagged commit. To include
fixes, commit them first and publish the matching new version. Keep existing
release tags intact.

CI uses the latest Windows runner and current action major versions.
Dependabot checks daily and opens action-update PRs for review.

For a local branch's release rehearsal, use Actions > ci > Run workflow, or
`gh workflow run ci.yml --ref <branch> -f release_dry_run=true`.
Manual runs only build and test; they never publish.
Installer downloads are saved in `lumo-setup`; screenshots and idle-memory
metrics are in `lumo-screens`. The app reads GitHub releases directly; no update-feed deployment is needed.

## Credits and license

Lumo is maintained by mthlpbs and is a modified version of
[Oyenza Studio](https://github.com/lucacadalora/oyenza-studio)
(formerly Oyen Poni), by Jatevo Labs and its contributors.
Original copyright notices and third-party licenses are preserved.
See [LICENSE](LICENSE) and [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt).
