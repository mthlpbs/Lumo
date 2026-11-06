# Notes for coding agents

Codex, Claude Code, Kimi or any other agent working in this repo: read this before you change the app or cut a release.

## Keep it light

Lumo sits on screen all day, so how little it costs is part of the product. The maintainer's rule (24 Oct 2026): keep it very light on CPU, memory and download size, and fast, however many features it gets. Every change follows these:

- **Nothing runs at rest unless the person switched it on or its page is open.** No new thread, timer, poll or network call that keeps going while the island is idle. React to Windows' own notifications where there are some. When something has to repeat, give the interval a reason in a comment and stop it as soon as nothing needs it.
- **Anything that repeats rests when nobody is there:** while the session is locked, the display is off or battery saver is on. The app already knows all three (`App::locked`, `App::display_off`, `st.power.saver`), and ambient animation already pauses for the first two.
- **Motion costs a frame every refresh.** Something that moves by itself on the closed island (the equalizer) redraws for as long as it shows. Add one only where people watch it, and let it stop.
- **Load heavy Windows parts when the feature first needs them, not at start.** Performance counters, graphics devices and WinRT classes bring their own libraries: the System page's counters alone add about 6 MB once loaded.
- **Big data is an add-on, never part of lumo.exe.** Dictionaries, sounds, place lists and models go on a GitHub pre-release and download when first needed, checked against the size and SHA-256 in `lumo_core::addons` (`addons/`, `services/addons.rs`). `include_bytes!` is for small things the app can't start without, such as the icon and the Outfit font.
- **A new crate needs a reason and its size.** Build with `CARGO_PROFILE_RELEASE_STRIP=false cargo wbuild` before and after, and compare `x86_64-w64-mingw32-nm --size-sort -S -C` totals per crate. Code that runs for moments (a handshake, a parser, a file format) can be built for size in the workspace `Cargo.toml` (`[profile.release.package.<name>] opt-level = "s"`). Code that runs every frame stays at full speed.
- **Measure CPU in cycles, not milliseconds.** `TotalProcessorTime`, `GetProcessTimes` and Task Manager count in 15.6 ms clock ticks, so a reading that takes a fraction of a millisecond shows as zero. Use `QueryProcessCycleTime` or `QueryThreadCycleTime`.
- **Every PR says what it costs:** download size (the installer in the `lumo-setup` artifact), memory at rest (`smoke-out/metrics.txt` in the `lumo-screens` artifact), and anything new that runs at rest or on every frame. "Nothing new" is a fine answer.

## Names that stay

The application is Lumo, maintained at `mthlpbs/lumo`. Its executable is
`lumo.exe`, the crates are `lumo` and `lumo-core`, and its environment variables
start with `LUMO_`. Application metadata and shortcuts show only Lumo.
Legacy identifiers may remain only in migration cleanup, historical signature
test fixtures and required third-party attribution. Existing profiles and
received-file paths must remain usable after a rename.
Windows settings and cache live in `Lumo` under APPDATA and LOCALAPPDATA.
Old profiles are imported once; they must never be reused as the save location.
Supported themes are Classic and Pill. Retired theme values load as Classic.

## Repository scope

The app is Windows-only. The macOS app, Oyenza account integration and bundled landing website have been removed. Shared branding in `docs/brand/` is still used by the Windows app, installer and README.

In-app updates and update-feed deployment are currently disabled (see `docs/updates.md`). A release does not require a landing-page update.

## Cutting a release

Merges, tags and releases happen only when the maintainer says so.

Keep the version in `Cargo.toml`, `Cargo.lock`, `crates/lumo/res/lumo.rc`, `installer/lumo.nsi`, and `installer/lumo.iss` aligned. Add `docs/release-notes/vX.md` and use it for the release text. Publish a GitHub release with the matching `vX` tag after the workflow has been committed. The `ci` workflow builds and tests the Windows app, checks the installer, and attaches the installer, portable ZIP, and checksums to that release. Drafts and non-`v` add-on releases do not build the app. Pushing a tag alone does not start a release build.

The release workflow does not deploy an update feed while updates are disabled. The retained `UPDATE_FEED_KEY` secret stays in the `release` environment: never print it or move it.

To try the release path without releasing, run `ci` by hand with `release_dry_run` on (`gh workflow run ci.yml --ref <branch> -f release_dry_run=true`). Manual runs build and test, save artifacts, and publish nothing: no tag, release or update-feed deploy.
