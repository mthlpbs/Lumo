<p align="center">
  <img src="docs/brand/lumo-logo.svg" width="88" alt="Lumo logo">
</p>

<h1 align="center">Lumo</h1>

<p align="center"><strong>A living island for your Windows desktop.</strong></p>

<p align="center">
  Your music, focus timer, files and AI companion, close at hand.<br>
  A small island at the top of your screen that opens when you need it.
</p>

<p align="center">
  <a href="#features">Features</a> &nbsp;·&nbsp;
  <a href="#install">Install</a> &nbsp;·&nbsp;
  <a href="#faq">FAQ</a> &nbsp;·&nbsp;
  <a href="#guides">Guides</a> &nbsp;·&nbsp;
  <a href="#development">Development</a>
</p>

<p align="center">
  <img src="docs/brand/readme/lumo/music-028b39114adb.png" width="880" alt="Lumo's Music page with album artwork, playback controls, a seek bar and synced lyrics">
</p>

<p align="center"><sub>Windows 10 &amp; 11 · x64 · Free and open source · No Lumo account needed</sub></p>
<p align="center"><sub>Maintained by <a href="https://github.com/mthlpbs">mthlpbs</a>. Based on <a href="https://github.com/lucacadalora/oyenza-studio">Oyenza Studio</a>. Screenshots use demo data.</sub></p>

## Install

Get the [latest release](https://github.com/mthlpbs/Lumo/releases/latest),
or use these pinned **2026.1107.2** downloads:

| Download | Get started |
| :--- | :--- |
| **[Installer (.exe)](https://github.com/mthlpbs/Lumo/releases/download/2026.1107.2/Lumo-Setup-2026.1107.2.exe)** | Recommended. Installs for your Windows account without administrator rights. |
| **[Portable ZIP](https://github.com/mthlpbs/Lumo/releases/download/2026.1107.2/Lumo-2026.1107.2-portable.zip)** | Extract the whole ZIP, then run `Lumo/lumo.exe`. |
| [Checksums](https://github.com/mthlpbs/Lumo/releases/download/2026.1107.2/SHA256SUMS.txt) | Compare the download's SHA-256 with `SHA256SUMS.txt`. |

To build the current source, use the [local packaging script](docs/packaging.md).
It creates a standalone EXE, portable ZIP and setup installer in `dist/<version>/`.

WinGet and Scoop distribution is prepared but not yet published. Maintainers can
follow the [one-time setup and automatic release updates](docs/package-managers.md).

**First time here?** Click the island to open it. Scroll vertically over its
header to switch pages. Swipe horizontally in either direction over the closed
island to expand it into Search; swipe again to close it. The gear opens
Settings; the pin keeps the island open.

Over a maximized app, hover at the top edge to reveal the island, then click
to open it. To open by hover over normal windows, choose **Settings > General >
Open island with > Hover**.

## Features

The main pages are **Music**, **Focus**, **Shelf**, **Tasks**, **System** and
**Add-ons**. **Ask AI**, **Script reader** and **Clipboard history** appear
when enabled. Open **Settings** from the gear in the header.

### Search apps and Windows settings

Swipe left or right over the closed island to expand it into Search. Swipe
again anywhere over Search to collapse it. Open pages keep their normal
horizontal navigation. Clicking outside closes Search unless its Keep open
pin is enabled, including after dismissing a result's context menu. The compact
list shows frequently opened results first, with native Windows icons and a
clear selection marker. The island fits the
number of results, so a single match uses just one row. When nothing matches,
the island shrinks around a web-search card with your selected engine and
query preview. Optional Google suggestions can appear beneath it; turn them
on in **Settings > Search**. They send unmatched text or address-bar search text to Google after you pause
typing and are off by default. Website addresses are excluded.

**Settings > Search** controls applications, Windows Settings and optional web
search. Right-click results for dark native Windows menus with administrator
actions where supported, copy-path and separate icon-file/icon-image copying.
Frequent results can be removed individually. Lumo commands and preference pages are excluded.

Search can also find **Windows services**, installed **PowerToys tools**, and
**VS Code commands**. Enable services to see their last-read state and explicit
Start/Stop buttons; Windows asks for administrator access when needed.
PowerToys results include tool settings and enabled tools from its running app.
VS Code commands appear only for open Code or Code Insiders windows. Selecting
a command uses that existing window; Lumo never starts VS Code for it. Closed
windows are discarded from cached search results.
These extra sources refresh on opening Search or through **Refresh search
sources**. Each source has its own toggle in **Settings > Search**.

Optional web search opens Bing, Google, DuckDuckGo or Brave inside the island,
using the installed Edge WebView2 Runtime. It starts only when you choose a
web action, or press <kbd>Ctrl+Enter</kbd> to search the current query with your
chosen engine (an empty field opens its home page). The neutral search field has
no browser button. A unified address bar contains navigation, refresh/stop,
default-browser, mini-browser, pin and resume icons. It shows the webpage title
while browsing; click or press <kbd>Ctrl+L</kbd> to edit the full URL with all icons
hidden. Double-click selects the entire URL, including slashes. Optional Google
suggestions also work while typing an address-bar search. Loading fills the
rounded bar and follows its bottom edge. Search has no close button; press
<kbd>Esc</kbd> or swipe horizontally to close it.
The browser expands vertically at the same width as Search by default.
**Settings > Search > Browser** offers Tall or Wide layout and height presets.
Pages open at their normal size with a desktop user agent; Lumo does not zoom
them to fit the window. Type a URL or bare domain such as `hirunews.lk` to see
**Visit** and open it directly instead of searching for it.
The mini browser icon moves the current page into a separate, resizable window
that stays on top. Return it to the island with the matching return icon;
the page and its history stay intact. Local app search remains available
while the mini browser is open. Resize it freely from 280 × 200 up to your
screen's available space; its size is remembered. Loading progress fills the
address field, and the search-to-browser transition fades smoothly.
No browser engine is bundled. [Search controls and scope](docs/search.md).

<p align="center">
  <img src="docs/brand/readme/lumo/search-2fc837ec498f.png" width="680" alt="Search inside the Pill island with native Windows app icons and a compact result list">
</p>

### Music that stays within reach

Control the music apps that use Windows media controls, seek through a track,
and follow matched lyrics from LRCLIB or your local library. The current line
can stay on the closed island. Select the mini player icon in Music to open a
separate, resizable player with lyrics. Its exit mini player icon returns to
Music in the island; the same icon is available in Music while the mini player
is open.

<p align="center">
  <img src="docs/brand/readme/lumo/mini-player-f7c19e5dd0f8.png" width="320" alt="Lumo mini player with synced lyrics, playback controls and the exit mini player icon at the top right">
</p>

Korean and Chinese pronunciation can appear beneath the original text.
[Music setup and local files](#guides) are covered below.

### Ask AI without leaving your desktop

Turn on **AI companion** in **Add-ons**, then open **Ask AI**, or hold <kbd>Ctrl</kbd> while hovering over the closed
island's trailing edge. Chat in the island, expand it, or open a full chat
window. Bring your own provider, key and models; add images, PDFs or a screen
snip when your model supports them.

AI is off by default. Switching it off also hides its Settings section;
your provider settings, saved key, models and chats stay for when you turn it back on.

<p align="center">
  <img src="docs/brand/readme/lumo/ai-b2b9678808b0.png" width="880" alt="Ask AI inside Lumo, with a compact conversation, history, model selection and an attachment button">
</p>

Replies support Markdown, tables and copyable code blocks. History is
encrypted for your Windows account, and temporary chats are not saved.
**Ask AI is a prototype**; provider capabilities, usage limits and charges
still apply. [Setup and limits](docs/ai-companion.md).

### Files from your phone, with your permission

Keep files and links on the **Shelf**, drag them into another app, or scan
its QR code to send files from your phone over Wi-Fi. Lumo asks before
accepting an incoming transfer. Pair a phone once to send from other networks.

<p align="center">
  <img src="docs/brand/readme/lumo/phone-219381f90f1e.png" width="880" alt="The Shelf showing an incoming phone transfer with separate Decline and Accept actions">
</p>

Works through your phone's browser. [Paired phone drop](docs/paired-drop.md)
explains pairing and how the connection works.

### Know what your PC is doing

**System** brings CPU, GPU, memory, drives, network and battery information
into one place. Open a module for its history and details. Optional game
readouts show FPS, GPU and CPU information over borderless full-screen games.

<p align="center">
  <img src="docs/brand/readme/lumo/system-c0ec9b9543f4.png" width="880" alt="Lumo's System page with CPU gauges, usage history, per-core activity and a process list">
</p>

Counters are active while their page or an enabled readout needs them.
**Show over games** has its own switch under **System > Games**.

### Make room for the small things

| Page | What it keeps close |
| :--- | :--- |
| **Focus** | A Pomodoro timer, with its countdown on the closed island. |
| **Tasks** | A small local to-do list. |
| **Script reader** | A teleprompter beneath your webcam, enabled from Add-ons. |
| **Clipboard history** | Copied text in memory, with password-manager privacy markers respected. |

<p align="center">
  <img src="docs/brand/readme/lumo/focus-922e0d5da670.png" width="880" alt="Lumo's Focus page showing a Pomodoro session, timer controls and focus history">
</p>

Script reader and Clipboard history are built in. Switch them on from the
**+ (Add-ons)** page when you need them, and off to stop them.

### Classic or Pill

Choose an island that meets the top edge, or a floating pill with a little
space around it. Both themes keep the same pages and controls, with brief
volume, charging and headset notices.

<p align="center">
  <img src="docs/brand/readme/lumo/pill-e2d7cae1be61.png" width="880" alt="The Music page in Lumo's Pill theme, floating below the top edge with rounded corners">
</p>

Change themes under **Settings > Appearance**. The rest of the app stays
familiar.

## FAQ

<details>
<summary><strong>Is Lumo free? Do I need an account?</strong></summary>

Lumo's code is MIT licensed, and its core features need no Lumo account.
Ask AI uses your own provider and API key; that provider may charge for requests.
See [PRIVACY.md](PRIVACY.md) for what each network feature sends.

</details>

<details>
<summary><strong>How do updates work?</strong></summary>

Open **Settings > About** and choose **Check for updates**. A newer release
offers **Download update**, followed by **Install and restart** once verified.

**Automatically update Lumo** downloads verified updates and installs them on
the next launch. **Tell me about new versions** controls notifications
separately. Both switches default to off; manual checks work with both off.
Automatic work rests while the session is locked, the display is off, or
battery saver is on. [How updates are verified](docs/updates.md).

</details>

<details>
<summary><strong>Will it cover my tabs or full-screen apps?</strong></summary>

Over maximized apps, the island reveals itself when you hover at the top
edge. The window button beside the gear can keep it visible over a particular
app; Lumo remembers that choice. **Settings > Display** controls maximized
and full-screen behavior. The pin keeps the expanded island open.

</details>

<details>
<summary><strong>Does it need administrator rights?</strong></summary>

The installer runs for your Windows account without administrator rights.
The optional FPS readout needs a one-time permission to join Windows'
Performance Log Users group. Its System page explains that permission before
requesting it.

</details>

<details>
<summary><strong>What stays on my PC?</strong></summary>

Settings, tasks, focus history and saved chats stay on your PC. AI requests
go to the provider you choose, lyrics lookups use their configured services,
and enabled update checks contact this project's public GitHub releases.
Phone transfers ask before accepting files. Details are in
[PRIVACY.md](PRIVACY.md).

Uninstalling keeps settings, tasks, focus history and received files.

</details>

<details>
<summary><strong>Where can I report a problem?</strong></summary>

[Open an issue](https://github.com/mthlpbs/Lumo/issues/new) with your Windows
version, the Lumo version from **Settings > About**, and steps to reproduce it.
Review `%LOCALAPPDATA%\Lumo\lumo.log` before attaching it; logs can contain
file paths or media information.

</details>

## Guides

<details>
<summary><strong>Music, local lyrics and downloaded lyrics</strong></summary>

Under **Settings > Music & lyrics**, enable **Use local lyrics** and choose
your Music folder. Lumo reads matching sidecar `.lrc`, `.lyrics` and `.txt`
files, plus embedded MP3, FLAC, M4A and Opus/Ogg lyrics. Windows metadata
handlers may support other formats. Timed lyrics keep their timestamps;
plain text remains unsynced. LRCLIB supplies missing lyrics.

**Save downloaded lyrics** uses the matching song's exact filename with a
`.lrc` extension. Save beside the song or choose a separate download folder.
A Music folder or download folder must be configured before files are saved.
Existing files are kept, and ambiguous or unmatched songs are not saved.
Clean and explicit editions remain distinct.

The speech-bubble button controls closed-island lyrics. The pop-out button
opens a resizable lyrics player; clicking it again returns lyrics to the
island. Hide that button with **Lyrics window button** in Music settings.
While the player is open, the island's lyrics column is hidden and its lyrics
toggle is disabled; closing the player restores them.

</details>

### Apple Music lyrics

Apple Music playback uses Windows media controls. Matched lyrics from LRCLIB
and local lyric files work without an Apple Music token. See
[PRIVACY.md](PRIVACY.md) for details about optional Apple Music requests.

<details>
<summary><strong>Phone drop and paired phones</strong></summary>

Open **Shelf > Phone**, scan the QR code and select files on your phone.
Accept the transfer on your PC. Same-Wi-Fi transfers use your local network.
Pair a phone from that panel to send from other networks.

See [Paired phone drop](docs/paired-drop.md) and the
[server guide](server/README.md) for connection details and self-hosting.

</details>

<details>
<summary><strong>Game readouts and shortcuts</strong></summary>

Enable **Show over games** under **System > Games**. Windows frame tracing
asks for its one-time permission there. Press
<kbd>Alt</kbd> + <kbd>Shift</kbd> + <kbd>F</kbd> to hide or show the readout while a game is in front. Choose
brief peeks if you do not want a persistent readout.

The tray menu contains **Start with Windows**, **Settings** and **Exit**.
Right-clicking the island has no action. Settings changes save immediately,
and horizontal touchpad swipes on the closed island open Search. A second
swipe over Search closes it. Open pages and Settings keep their horizontal
page and category navigation. **Settings > Search > Open with horizontal
scroll** controls only the closed-island opening gesture.

</details>

<details>
<summary><strong>Settings, saved data and migration</strong></summary>

| Location | Path |
| :--- | :--- |
| Installed app | `%LOCALAPPDATA%\Programs\Lumo\lumo.exe` |
| Settings and data | `%APPDATA%\Lumo\` |
| Cache and log | `%LOCALAPPDATA%\Lumo\` |
| Received files | `Downloads\Lumo\` |

Existing profiles are imported into Lumo once, with the original folders kept
as backups. An existing received-file inbox is reused to preserve Shelf links.
Retired themes load as Classic; old logs and obsolete update metadata are not
carried forward. Lumo uses `lumo.exe` and `LUMO_*` environment variables.

</details>

## Development

Lumo is a Windows-only Cargo workspace with a native Win32, Direct2D and
WinRT interface.

| Component | Purpose |
| :--- | :--- |
| `crates/lumo` | The Windows application and its UI and services. |
| `crates/lumo-core` | Shared models, parsing, settings and other core logic. |
| `server` | The paired-phone meeting server and phone page. |

<details>
<summary><strong>Build and test</strong></summary>

Use Rust stable with the Windows MSVC toolchain:

```powershell
cargo test --workspace --locked
cargo build -p lumo --release --locked
```

To update the product version and build the standalone EXE, portable ZIP and
setup installer together, run `./tools/package.ps1`. Outputs go to
`dist/<version>/`. [Packaging options and checks](docs/packaging.md).

The executable is `target/release/lumo.exe`. For a GNU cross-build with a
MinGW toolchain, `cargo wcheck` checks the Windows target and `cargo wbuild`
builds `target/x86_64-pc-windows-gnu/release/lumo.exe`.

The graphical self-test uses an isolated data directory and demo data:

```powershell
.\target\release\lumo.exe --selftest selftest-out
```

It writes screenshots and `report.json` with layout and timing checks.
`LUMO_SELFTEST_THEME=classic` or `pill` selects a theme;
`LUMO_SELFTEST_DPI=144` renders at 150% scaling.
Read [AGENTS.md](AGENTS.md) before changing the app. Keep background work
opt-in, load heavy Windows components on demand, and measure CPU in cycles.

</details>

<details>
<summary><strong>Versioning and release builds</strong></summary>

Versions use `year.monthday.build`: `2026.1107.2` means 7 November 2026,
build 2. A later release that day uses `2026.1107.3`; the next day starts at
`2026.1108.1`. January 7 uses `2027.107.1`, without leading zeros.
The build counter runs from 1 to 65535. Rebuilding a commit keeps its version;
every published app release gets a new number.

Keep these files aligned before tagging:

- `Cargo.toml` and the `lumo` / `lumo-core` entries in `Cargo.lock`.
- `crates/lumo/res/lumo.rc` and `crates/lumo/res/lumo.manifest`.
- `installer/lumo.nsi` and `installer/lumo.iss`.

Add `docs/release-notes/v<version>.md` and use it as the release text. Commit
the workflow and source first, then publish a GitHub release with the matching
numeric tag, such as `2026.1107.2`. Historical `v`-prefixed tags also work.

The [ci workflow](.github/workflows/ci.yml) tests and builds Windows, checks the
installer, and attaches the installer, portable ZIP and checksums. Saving a
draft or pushing a tag alone does not build; publishing a release or
pre-release does. Non-version add-on tags are skipped, and mismatched app
versions fail validation. Re-running a failed job uses its original tagged
commit; publish a new matching version to include fixes instead of moving an
existing release tag.

For a release rehearsal, use **Actions > ci > Run workflow**, or:

```powershell
gh workflow run ci.yml --ref <branch> -f release_dry_run=true
```

Manual runs publish nothing. Installers are saved in `lumo-setup`; screenshots
and idle metrics are in `lumo-screens`. Dependabot checks action versions
daily. The app reads GitHub releases directly, so no update-feed or website
deployment is needed. Packaging writes the portable ZIP directly, without
leaving an unpacked `dist/Lumo` folder.

</details>

## Credits and license

Lumo is maintained by **mthlpbs** and is a modified version of
[Oyenza Studio](https://github.com/lucacadalora/oyenza-studio), by Jatevo Labs
and its contributors. Original copyright notices and third-party licenses
are preserved.

Synced lyrics use [LRCLIB](https://lrclib.net). The app is built with Rust,
[windows-rs](https://github.com/microsoft/windows-rs) and
[serde](https://serde.rs), with other dependencies listed in the notices.

Code is licensed under [MIT](LICENSE).
[THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt) ships with every download.
