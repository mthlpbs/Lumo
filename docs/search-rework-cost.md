# Search and AI continuity: cost and verification

Measured locally on Windows x64 on 13 November 2026, compared with the preceding
information-card build. Both local artifacts keep version 2026.1112.2, use
`CARGO_PROFILE_RELEASE_STRIP=false cargo wbuild --offline`, and use the same
stripping tool and NSIS compiler. No tag, merge or release was published.

| Artifact | Before (bytes) | After (bytes) | Change (bytes) |
|---|---:|---:|---:|
| Stripped EXE | 7,492,608 | 7,449,600 | -43,008 |
| NSIS installer | 2,795,190 | 2,778,192 | -16,998 |

No crate, browser engine or dataset was added. Information-card networking and
custom-site management were removed. The existing external ad-blocking add-on
is unchanged and is staged beside the local EXE for testing.

## Idle work, memory and active work

There is no new idle thread, poll or repeating timer. The shortcut guide is
static text rendered only in Settings. Recent-app history uses existing bounded
usage records; no queries are recorded. Its icon strip displays at most 16
items, and reuses the existing 64-entry/48-pixel icon cache. The short-lived icon
worker can fill a complete visible strip in one bounded batch.

Search's icon/field and clearing fades last 160 ms, then stop. They run only in
the open workspace and snap in battery saver. Native touchpad source registration
uses Windows notifications, with no polling or worker. Browser mode switching
retains one page and suspends it while the launcher is showing.

AI websites now keep one current suspended WebView after closing the island.
This deliberately retains renderer/page memory to preserve the conversation and
draft; it does not free that memory merely because the island closes. Selecting
another AI choice or exiting releases that controller. Local API chat's Resume
preference is independent. Lock, display-off and battery saver retain the
existing suspension behavior, including while a reply is in progress.

The AI bridge observes mutations only during an explicit submission. Muse has
a bounded startup deadline and one response-settling timer, reset by text
mutations; both disconnect after completion or cancellation. There is no idle
observer, network interception or periodic completion check. The quiet-response
fallback is a heuristic for layouts without a reliable generation marker, so an
unusually long pause may look complete. Signed-in Muse pages were not tested.

Whole-app idle working set/private bytes and active CPU cycles were not
remeasured in this local run; no numerical memory/CPU delta is claimed. The
retained-page memory tradeoff above is material. CI smoke metrics remain the
source for an app-level memory comparison.

## Verification

366 core tests and 261 app tests passed. Eight focused native tests passed:
compact recent-app layout in Classic/Pill, width/hide/clear transitions,
right-button wheel routing, same-controller browser mode switching, real
suggestion mouse down/up and Tab completion, chat-page continuity with Resume
off, Muse hidden-response completion, settings separation and shortcut-guide
rendering, existing horizontal gestures, and shared ad blocking. The Google
suggestion endpoint was also checked online successfully. Provider behavior is
covered by offline fixtures; no prompt was sent to a signed-in AI account.

Screenshot review confirmed the empty launcher shows only the bar, optional
browser icon and small recent-app icons, with full rows only after typing.
See [Search](search.md) and [AI websites](ai-sites.md) for user-facing behavior.
