# Browser layout and blank-page cost

This local change removes vertical Search/Browser switching over the address
field, tightens browser chrome to a 34 DIP toolbar with 8 DIP outer insets and a
6 DIP page gap, and reduces the browser's outer/page corner radii to 16/12 DIP.
Resume restores the retained page on explicitly opening Browser. Search keeps its
fresh launcher. The AI website control popup now has an 8 DIP right inset after
Close.

Browser starts with a native document and a static starting hint. The previous home layout,
pinned-site controls, page search field, and toolbar Home/star actions are
absent. Existing saved profile data is retained, but has no active UI. A blank
page starts no WebView, network request, worker, timer or per-frame animation.
No dependency or dataset was added. The external protection add-on is unchanged.

## Verification

The core suite passes 368 tests, and the app suite passes 261 tests. Native
fixtures verify blank startup in Classic/Pill and the 280 x 200 detached host,
GPU/software rendering, normal webpage clipping and resume continuity, AI
control alignment, and existing mouse/touchpad workspace gestures. The blank
page exposes no content widgets and starts no web renderer. These are local
checks, not a published release.

## Local artifact comparison

Built on Windows x64 on 13 November 2026 with
`CARGO_PROFILE_RELEASE_STRIP=false cargo wbuild --offline`, then the same strip
and NSIS tools as `.tools/search-rework-build`. Version remains 2026.1112.2;
no tag, merge or release was published.

| Artifact | Before (bytes) | After (bytes) | Change (bytes) |
|---|---:|---:|---:|
| Stripped EXE | 7,449,600 | 7,467,520 | +17,920 |
| NSIS installer | 2,778,192 | 2,785,141 | +6,949 |

The local portable ZIP is 4,864,812 bytes and includes the unchanged, verified
external protection add-on alongside the executable. It was created directly
with `tools/package_portable.py`, then the verified add-on files were added.
No unpacked `dist/Lumo` folder was created.

At-rest memory comparison remains unverified: the before sample with the panels
closed recorded 100.7 MiB working set / 70.9 MiB private, but the after interval
included real expand/collapse activity (127.8 MiB / 86.2 MiB). These are not
comparable idle readings. The raw samples and logs are in
`.tools/browser-home-metrics`; no claim of a measured memory saving is made.
CPU samples use `QueryProcessCycleTime`, but the active after interval is also
unsuitable for an idle comparison. There is no new work running at rest and no
new recurring work per frame; the starting hint paints only when its native page is visible.

## Address editing follow-up

Clicking outside the address field now cancels its edit in either host, restores
the normal controls and cancels pending suggestions. Suggestion rows remain
inside the editing group so pointer selection still works. Blur and Escape also
restore an empty address on the native starting document rather than exposing
its internal URL.

The idle field displays "Search or enter a web address" without inserting that
prompt into editable text. While focused it is empty and shows the normal text
caret, including after the last character is deleted. Empty focused addresses
use the existing one-shot caret wake; it stops on blur, close, lock, display off
or battery saver. There is no new idle timer or worker. The static "Start
browsing" hint hides during address editing. It loads no webpage or remote
assets and adds no new dependency.

The app suite passes 261 tests. Native regression coverage types, deletes and
clicks the background in both themes and both hosts; additional checks preserve
suggestion clicking, normal webpage focus and outside dismissal. GPU and
software starting-page checks pass.

The address follow-up uses the same local release, strip and installer tools.

| Artifact | Blank-page baseline (bytes) | Address fix (bytes) | Change (bytes) |
|---|---:|---:|---:|
| Stripped EXE | 7,467,520 | 7,468,544 | +1,024 |
| NSIS installer | 2,785,141 | 2,785,088 | -53 |

The portable ZIP with the unchanged verified external add-on is 4,864,999 bytes.
Idle memory was not remeasured for this follow-up; the earlier desktop samples
remain unsuitable for a before/after idle comparison. There is no new idle work.
