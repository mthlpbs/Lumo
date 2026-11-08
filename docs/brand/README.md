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
| `music-a65be8ef97c6.png` | `*-page-media.png` | Classic |
| `ai-b2b9678808b0.png` | `*-page-ai.png` | Classic |
| `phone-219381f90f1e.png` | `*-page-shelf-phone-ask.png` | Classic |
| `system-c0ec9b9543f4.png` | `*-page-system.png` | Classic |
| `focus-922e0d5da670.png` | `*-page-focus.png` | Classic |
| `pill-5c0bee0a71c2.png` | `*-page-media.png` | Pill |

Search uses an unmodified native launcher-check capture in Pill, with icons
from the Windows app catalog: search-ff4a6d099aad.png. Run the ignored
app::tests::inline_search_renders_icons_keyboard_filters_and_collapses_in_both_themes
test with LUMO_SEARCH_QA set to its output directory to refresh it.

The compact empty state uses the unmodified Pill capture `search-empty-6d7da2b04cc4.png`
from the same native Search check.

The embedded-browser preview `search-browser-7e75ac302428.png` is a native screen capture in Pill,
using the installed WebView2 Runtime and a public Google query. It is captured
by `app::tests::web_results_render_inside_the_layered_island_and_close_with_it`
with `LUMO_SEARCH_QA` set and optional `LUMO_BROWSER_QA_URL`. The fixture
runs below the screen edge to keep other running islands out of the capture.

The unpackaged Search changes use `search-browser-fe983f733aa2.png`, captured by the same native
browser check in Pill with Keep open and Resume page enabled. The earlier
preview is retained as the 2026.1108.9 package reference.
