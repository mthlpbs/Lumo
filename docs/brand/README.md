# Lumo brand assets

`lumo/` contains the supplied Windows Dynamic Island logo, including its
original ICO, PNG and SVG files and monochrome variants.

`python tools/make_brand.py` copies the supplied ICO to
`crates/lumo/res/lumo.ico` and generates the installer images from that logo.
It also updates `lumo-logo.svg`, `lumo-logo-512.png` and `social-preview.png`.

The character theme is removed. Any historical artwork retained under `art/`
is unused source material and is not included in the application or installer.
