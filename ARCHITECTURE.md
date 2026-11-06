# Lumo architecture

Lumo is a "living notch" for Windows 10/11: a black island that hangs from the
top-centre of the screen, shows live activities (media, timers, pop-ups) while
collapsed, and expands on hover into pages (Music + synced lyrics, Focus,
Shelf, Tasks, Clipboard, Today, System). The Shelf's Phone drop serves a
small web page on the local network so a phone can send files to it.

It was called Lumo until 0.2.0, and plain Lumo before that. The code keeps the
old short name where people don't see it: the crates (`lumo`, `lumo-core`), `lumo.exe`,
its folders (`%LOCALAPPDATA%\Programs\Lumo`, `%APPDATA%\Lumo`), the uninstall key and
Run value, `Local\Lumo.Island`, the `LUMO_*`
variables. Changing those would break upgrades and settings, so they stay.

## Crates

| Crate | What | Builds on |
|---|---|---|
| `crates/lumo-core` | Pure Rust models for geometry, animation, lyrics, Korean/Chinese pronunciation, tasks, focus, shelf, clipboard, script reader, system metrics, phone transfers, settings, storage, and the retained updater | any OS (`cargo test -p lumo-core`) |
| `crates/lumo` | The Windows app (Win32 + Direct2D + WinRT via `windows` 0.58) | Windows only (`cargo wcheck` / `cargo wbuild` cross from macOS/Linux) |

## Threads and data flow

```
 services (own threads / COM callbacks)          UI thread (owns everything below)
 ─────────────────────────────────────          ─────────────────────────────────
 media (GSMTC)      ─┐                           App ── AppState (state.rs)
 lyrics (LRCLIB)    ─┤  EventSink.send(AppEvent)  │      ├─ pages/*      draw via Ui, push Commands
 audio (volume)     ─┼──► mpsc + PostMessage ───► │      ├─ activities  collapsed pill content
 bluetooth          ─┤     (WM_APP_EVENT)         │      └─ island       spring-animated outline
 imaging (thumbs)   ─┤                            │
 phone drop (LAN)   ─┘ on demand, while the Shelf's phone panel is open
 frame clock thread ── WM_APP_FRAME (display vblank, only while animating)
```

* **Rendering**: `gfx.rs`. The island window is a fixed-size, per-pixel-alpha
  layered window big enough for the largest island (widest, tallest page plus
  spring overshoot); the island is drawn centred in it, so a morph never moves
  or resizes the window and the centre stays on one pixel.
  * GPU renderer (default): Direct2D on a Direct3D 11 device (hardware, else
    WARP) draws into a premultiplied flip-model swap chain that
    DirectComposition shows on the window. The layered window's own bitmap is
    then an invisible hit mask (alpha 1/255 where the island takes the mouse,
    0 elsewhere), refreshed at ~30 Hz while the island moves and exactly at
    rest, so transparent pixels stay click-through.
  * Software renderer (fallback: no DirectComposition, e.g. Wine; repeated
    device loss; `LUMO_RENDERER=software`): Direct2D in software into a DIB
    pushed with `UpdateLayeredWindowIndirect`, copying only the rectangle the
    island covered this frame and last.
  * Content that fades in or out during a morph is drawn into an offscreen
    layer and composited (`Canvas::layered`): group opacity, and its text is
    rasterised once instead of re-hinting while it scales.
  * All coordinates are **DIPs** (the render target DPI = monitor DPI), in
    window space: the island body is `Island::body_rect_at(cx)`.
* **Frame clock**: `window.rs`. A thread waits on the island display's vblank
  (`IDXGIOutput::WaitForVBlank`; DwmFlush or a steady timer as fallbacks) and
  posts `WM_APP_FRAME`. Timestamps are phase-locked to the display
  (`lumo_core::frametime`), and each frame is stamped with the vblank it will
  be shown on, so every animation step is a whole number of refresh periods
  (smooth at 60, 144 or 240 Hz). A morph that starts from rest advances one
  refresh on its first frame. Morphs log a `motion` line (frames, fps,
  hitches, worst gap) for the first 20, then only when frames were dropped.
* **Idle = no frames**: frames are produced only while the island animates,
  a page calls `ui.request_anim()`, input arrives, or a live activity is
  showing. Real motion runs on every vblank; ambient motion on every n-th
  (the equalizer near 60 fps, a drifting progress bar near 30). Otherwise the
  frame clock thread is parked and one-shot timers wake the UI for the next
  visible change (countdown second, HUD expiry, minute tick). The one thing
  that reads at rest is the System page's kept minute (`system_history`, on
  by default): a light reading a second, about 0.35 ms of CPU. It rests while
  the session is locked or the display is off, and on battery saver
  (`lumo_core::sysmon::reads`). See "Keep it light" in AGENTS.md.
* **Window**: `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`.
  It never takes focus on click; text fields temporarily drop NOACTIVATE
  (`Command::FocusText`) and give focus back afterwards.
* **Helper window** (hidden): tray icon, AppBar registration (full-screen
  notifications), `WM_POWERBROADCAST`, `WM_CLIPBOARDUPDATE`.
* **Step aside** (Settings › Over maximized windows, on by default): the
  full-screen watcher also keeps the app windows whose top edge is at the
  top of the island's monitor, within 8 DIPs (maximized, snapped or dragged
  up; one placed lower keeps its title below the island), as
  `AppEvent::TopWindows`, from the foreground and location WinEvents it
  already listens to (no polling). Each frame the app checks whether one is
  under where the island rests (`activities::resting_mode`'s geometry) and,
  if so, rests as `Mode::Aside`: the Hidden look's invisible strip until the
  pointer reaches it. Pop-ups, notices and a new song's title still drop
  down; over a full-screen app the full-screen choice decides.

## Contracts

### Pages (`crates/lumo/src/pages/*.rs`)

```rust
pub fn height(st: &AppState) -> f32;                      // content height in DIPs
pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect);  // draw + handle input
```

* `area` is the content rect below the header (x/y absolute in canvas DIPs);
  its width is `settings.expanded_width - 2*PAD` (default 620 - 36 = 584).
* Immediate mode: `ui.button(id, rect) -> Resp { hovered, pressed, clicked, rclicked, drag }`.
  Ids from `wid("name")` / `wid_n("row", i)`; must be unique per frame.
  A click arrives on the frame after mouse-up (the app repaints automatically).
* Widgets: `icon_button`, `pill`, `progress_bar` (returns seek fraction),
  `text_input` / `text_input_row` (return submitted/cancelled), `wheel_in(rect)`.
* Drawing: `ui.c` is a `Canvas`: `fill_rrect`, `stroke_rrect`, `fill_circle`,
  `ring`, `arc`, `line`, `text(s, rect, &TextStyle)`, `measure`, `image`,
  `icon(Icon, rect, color)`, `clip/unclip`, `with_alpha`.
* Text: `TextStyle::new(size).weight(Weight::Semibold).color(c).center()/.right().tabular().wrap().top()`.
  Single-line text is ellipsized automatically.
* Colours: `lumo_core::geom::palette` (island is pure black; text is white at
  0.96 / 0.62 / 0.50, disabled 0.30; accents GREEN, ORANGE, RED, BLUE, CYAN,
  PURPLE, YELLOW).
* Type scale: `gfx::ts` (11 / 12 / 13 / 15 / 18 / 30 / 50) via `TextStyle::caption()`,
  `footnote()`, `body()`, `headline()`, `value()`, `display()`, `hero()`. Nothing below 11.
* Buttons: `ui.pill(id, rect, label, icon, PillStyle::Utility | Neutral | Primary(c) | UtilityAccent(c))`
  at `PILL_SMALL` (28) or `PILL_REGULAR` (34) height, `PILL_GAP` 8 apart.
* Settings (`pages/settings.rs`) has no tab (`PageKind::tabbed`): the gear right of
  the tabs opens it, and pressing it again goes back. It changes what the tray menu
  changes: settings through `Command::SaveSettings` (applied as a tray change is),
  Start with Windows through `Command::SetAutostart` (read and set off the UI thread).
* The Add-ons tab exposes Script reader and Clipboard history. Saving a switch applies it through `App::settings_changed`; built-in add-ons show or hide their page without downloads.
* Local lyrics use the existing lyrics worker. `settings.local_lyrics` selects local sidecars for desktop players; browser sessions retain their online source. `music_directory` is empty for the Windows Music known folder or holds the custom folder. Lookups carry a request id so old source/folder answers are discarded, including for the same song. Native file/folder dialogs run only when requested on a short-lived STA thread. No local-folder watcher, index or timer is created.
* Timed page changes (an Undo pill lapsing) report their deadline through
  `pages::next_wake` so the app wakes exactly then instead of polling.
  `st.accent` = Windows accent (lightened); `st.media_accent()` = album-art colour.
* Continuous animation: call `ui.request_anim()` while something must move
  every frame (lyrics scroll, progress bar while playing). Never animate idle.
* Side effects: push `Command`s (`st.cmd(...)`, `st.media_cmd(...)`, `st.save()`,
  `st.chime()`, `st.notify(...)`). Never call services from pages.
* Transient page state lives in `st.ui` (`PageUiState`); add-only requests for
  new fields go through the lead.

### Services (`crates/lumo/src/services/*.rs`)

* `start(sink: EventSink) -> windows::core::Result<Self>`; failures are logged
  and the feature is skipped. Never panic, never block the UI thread.
* Talk to the UI only through `sink.send(AppEvent::…)` (see `events.rs`).
* COM on worker threads: initialise with `CoInitializeEx(None, COINIT_MULTITHREADED)`.
* WinRT async: call `.get()` only on worker threads.

### Events (`events.rs`) and commands (`commands.rs`)

`AppEvent` = everything services report. `Command` = everything pages request.
`app.rs` maps both onto `AppState` and service calls.

## Files

| File | Owner area |
|---|---|
| `app.rs` | message loop, island state machine, input, timers, commands |
| `activities.rs` | collapsed modes + their geometry and drawing |
| `pill/*.rs` | the Pill theme (`Theme::Pill`, `look::PILL` with `Shape::Pill`): the island floats `pill::TOP_GAP` below the top edge (`Island::set_floating`); `shape` draws the floating outline, shadow and hit area, `motion` its springs, and one module per surface (`idle`, `status`, `media`, `lyric`, `live`, `hud`, `notice`, `expanded`, `music`) behind hooks in `activities.rs` and `pages` that fall back to Classic's drawing |
| `gfx.rs`, `icons.rs`, `ui.rs`, `island.rs`, `window.rs` | rendering/UI foundation |
| `state.rs`, `events.rs`, `commands.rs` | shared contracts |
| `pages/*.rs` | expanded pages |
| `services/*.rs` | media (Apple Music's own progress slider and back button through UI Automation in `applemusic_ui`: it declines both from Windows' media controls), lyrics/http, audio, bluetooth, fullscreen, clipboard, imaging, power, sound, sysstats, phonedrop (started and stopped by the Shelf's phone panel), sysmon (the System page's readings: each second while the page is open or a game's readout shows, a light reading a second for the kept minute while it's closed; all of it rests while nobody is at the PC, the kept minute also on battery saver; while the page is open on a desktop it also loads NVML for an NVIDIA card's watts and reads a power supply that reports over USB, both closed with the page), psu (that power supply: found among the HID devices by its USB ids, read with requests only, replies matched to them), addons (add-on files: downloaded from their GitHub pre-release when an add-on is switched on, checked against the size and SHA-256 in `lumo_core::addons`, deleted when it is switched off; asleep otherwise), frames (Show over games' frame count: a real-time event tracing session of the game's Present calls, only while the readout shows; the one-time OK that lets the app read it is `lumo.exe --allow-fps <SID>`, run elevated) |
| `dragdrop.rs`, `shell.rs`, `tray.rs`, `autostart.rs` | OS integration |
| `version.rs` | which build this is for the menu's About line: the version, and from `build.rs` release, test build (commit) or local build, and the day CI built it |
| `updates.rs` | new versions, the app's side: when to read the update file (`update.json` remembers), the island's notice ("Update available x", Hide), the offer the menu and Settings show, and on Windows the new version's Setup (downloaded first, run with `/S /reopen` now or before the next start); the Update sheet it opens is drawn by `pages/settings.rs` (rules in `lumo_core::update`, the read and signature check in `services/update.rs`; docs/updates.md) |
| `selftest.rs` | `lumo.exe --selftest <dir>` scripted screenshots for CI |

## Data

Add-ons lists Script reader and Clipboard history. Prayer, Health, Money, and Japanese readings are removed. Updates are retained behind `lumo_core::update::ENABLED = false`; no updater starts in this build.


* Settings: `%APPDATA%\Lumo\settings.json` (created with defaults; tray → "Open settings file").
* Data (tasks, shelf, focus stats): `%APPDATA%\Lumo\data.json`. Drinks logged by the old Water page stay in it untouched (`water`).
* Cache + log: `%LOCALAPPDATA%\Lumo\` (`lyrics\*.json`, `lumo.log`), plus the Apple Music
  token sealed with DPAPI (`apple-music-token.bin`) and the web player's public
  developer token (`apple-music-web.json`).
* Phone drop: received files go to `Downloads\Lumo` (`paths::phone_inbox_dir`),
  staged in a `.lumo-incoming-<n>` folder there until every file of a send has arrived.

* `LUMO_HOME` overrides both (the self-test uses it).
* Clipboard history is memory-only and skips content marked
  `ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory=0`
  (password managers set these).

## Building

```
cargo test -p lumo-core          # logic tests, any OS
cargo wcheck                     # type-check the Windows app from macOS/Linux
cargo wbuild                     # cross-build target/x86_64-pc-windows-gnu/release/lumo.exe (needs mingw-w64)
cargo build -p lumo --release    # on Windows (MSVC)
```

