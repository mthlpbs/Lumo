# Lumo brand assets

`lumo/` contains the supplied Windows Dynamic Island logo, including its
original ICO, PNG and SVG files and monochrome variants.

`python tools/make_brand.py` copies the supplied ICO to
`crates/lumo/res/lumo.ico` and generates the installer images from that logo.
It also updates `lumo-logo.svg`, `lumo-logo-512.png` and `social-preview.png`.

The character theme is removed. Any historical artwork retained under `art/`
is unused source material and is not included in the application or installer.

`readme/lumo/` contains unmodified screenshots from Lumo's own graphical
self-test, rendered in Classic and Pill at 150% scaling with demo data.
The README uses these current Windows captures rather than historical
upstream artwork. They are documentation assets, not bundled app resources.

To refresh them, run `lumo.exe --selftest <output>` with
`LUMO_SELFTEST_DPI=144` and `LUMO_SELFTEST_THEME=classic` or `pill`, then copy
the captures below without changing their contents:

| README image | Capture | Theme |
| --- | --- | --- |
| `music.png` | `09-page-media.png` | Classic |
| `ai.png` | `57-page-ai.png` | Classic |
| `phone.png` | `14-page-shelf-phone-ask.png` | Classic |
| `system.png` | `22-page-system.png` | Classic |
| `focus.png` | `10-page-focus.png` | Classic |
| `pill.png` | `09-page-media.png` | Pill |
