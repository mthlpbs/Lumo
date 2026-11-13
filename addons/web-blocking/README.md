# Managed web protection data

This is the external data add-on for Brave's `adblock-rust` engine in Lumo.
Filter-list bodies are never embedded in `lumo.exe`. `manifest.json` records
the exact size and SHA-256 checked by `lumo_core::addons::WEB_BLOCKING`.

The snapshots were prepared on 13 November 2026 from:

- EasyList: <https://ublockorigin.github.io/uAssets/thirdparties/easylist.txt>
- EasyPrivacy: <https://ublockorigin.github.io/uAssets/thirdparties/easyprivacy.txt>
- Brave resources: <https://raw.githubusercontent.com/brave/adblock-resources/master/dist/resources.json>

`resources.json` retains Brave's resources and adds Lumo's tiny empty JavaScript,
CSS, text and HTML replacements (`noop.js`, `noop.css`, `noop.txt`, `noop.html`).
These replacements are MIT-licensed with Lumo. Brave's resource sources are
available at <https://github.com/brave/adblock-resources> under MPL-2.0; individual
resources retain their original authors' notices. EasyList and EasyPrivacy
retain their upstream GPL-3.0 / CC BY-SA licensing and headers; see
<https://easylist.to/pages/licence.html>.

Prepare a local build with:

```
python tools/prepare_web_blocking.py --input .tools/adblock-data --output .tools/brave-adblock-build
```

The app first checks its cache, then `protection/<tag>/` beside the executable.
Both paths require identical integrity checks. Development builds can also read
the verified `.tools/adblock-data` fixture. No unverified legacy subscription or
custom rule is imported into the new engine.

For first-use downloads in distributed builds, the maintainer must publish the
three verified files from `manifest.json` to the matching non-app GitHub
pre-release tag. Publishing is a separate maintainer action, not performed by
the preparation tool. Lumo fetches this add-on only after the person enables
blocking or selects Install protection data; there is no retry timer or updater
at rest. Cache data is stored under Lumo's existing add-ons directory.
