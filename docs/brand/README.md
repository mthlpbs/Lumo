# Lumo brand assets

`lumo/` contains the supplied Windows Dynamic Island logo, including its
original ICO, PNG and SVG files and monochrome variants.

`python tools/make_brand.py` copies the supplied ICO to
`crates/lumo/res/lumo.ico` and generates the installer images from that logo.
It also updates `lumo-logo.svg`, `lumo-logo-512.png` and `social-preview.png`.

The character theme is removed. Any historical artwork retained under `art/`
is unused source material and is not included in the application or installer.

`readme/lumo/` contains unmodified screenshots from Lumo's own graphical
self-test, rendered in Classic and Pill with demo data.
The README uses these current Windows captures rather than historical
upstream artwork. They are documentation assets, not bundled app resources.

To refresh them, run `lumo.exe --selftest <output>` with
`LUMO_SELFTEST_DPI=144` and `LUMO_SELFTEST_THEME=classic` or `pill`, then copy
the captures below without changing their contents. Use the scene suffix to
find each capture; its numeric prefix can change when scenes are removed.

Image filenames include the first 12 characters of their SHA-256 hash. Update
the README image paths when refreshing screenshots so previews request the
new images instead of reusing older cached assets:

| README image | Capture | Theme |
| --- | --- | --- |
| `music-028b39114adb.png` | `*-page-media.png` | Classic |
| `ai-b2b9678808b0.png` | `*-page-ai.png` | Classic |
| `phone-219381f90f1e.png` | `*-page-shelf-phone-ask.png` | Classic |
| `system-c0ec9b9543f4.png` | `*-page-system.png` | Classic |
| `focus-922e0d5da670.png` | `*-page-focus.png` | Classic |
| `pill-e2d7cae1be61.png` | `*-page-media.png` | Pill |

Search uses unmodified native launcher-check captures in Pill, with icons
from the Windows app catalog:

- `search-2fc837ec498f.png`: frequent results.
- `search-single-6045b9e9789c.png`: one matching result.
- `search-empty-dbb7ccf50536.png`: the compact web-search action.
- `search-suggestions-2662ff4cf84f.png`: opt-in suggestions with injected demo results.

Run the ignored
`app::tests::inline_search_renders_icons_keyboard_filters_and_collapses_in_both_themes`
test with `LUMO_SEARCH_QA` set to an output directory to refresh them.

The browser preferences capture `browser-preferences-b1370d7150ce.png` comes from
`settings_window::tests::search_preferences_configure_gestures_sources_and_explicit_actions`
with the same output variable. It shows a customized Wide layout; Tall
is the default, pages stay at normal size, and Google suggestions default to off.
Older browser screenshots are retained as historical package references.

The mini player preview `mini-player-f7c19e5dd0f8.png` is the unmodified
430x640-DIP capture from
`lyrics_window::tests::lyrics_player_renders_resizes_controls_scrolls_and_follows_the_current_track`.
Run it with `LUMO_LYRICS_QA` set to an output directory.

`search-visit-b8f75856755c.png` comes from the native launcher check and shows
a bare domain opening directly through Visit.

`search-sources-1b1967aab3b0.png` shows customized source toggles from the native
Settings check. Windows services default to off; VS Code commands default to on.
The retired browser-content and custom-action controls are absent.
`search-service-3aeea0b85788.png` uses a fictional service in the native launcher
check; the test validates the Stop command without executing it.

`native-context-menu-0b599d71a06b.png` is the unmodified native HMENU capture from
`native_menu::tests::native_menu_renders_and_dismisses_without_invoking`.
It previews app actions without invoking a command or changing the clipboard.
