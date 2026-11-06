# The update file (temporarily disabled)

Lumo disables updates through `lumo_core::update::ENABLED = false`. No updater
starts, update requests and installer execution are blocked, and Settings keeps
the update switch disabled. Saved settings cannot override this policy.
Both update-feed workflow jobs are skipped. The implementation and persisted
state remain intact for future use.

Before enabling updates, mthlpbs must configure Lumo's feed URL, release URL
allowlist, installer names, and trusted signing keys together. The legacy feed,
keys, signed fixture, and parser below describe the retained upstream format.
The packaging tools now produce Lumo installers.

## Retained upstream implementation

Every copy of Lumo learns that a new version is out from one small file, <https://raw.githubusercontent.com/mthlpbs/lumo/main/latest.json>. This page says what is in it, where it is made, and how it stays trustworthy.

## What's in it

```json
{
  "about": "Lumo's update file. …",
  "payload": "{\"schema\":1,\"version\":\"0.2.3\",\"date\":\"2026-10-25\",\"headline\":\"Sing along from the closed island\",…}",
  "signature": "base64 of r and s, 64 bytes",
  "key": "4cb3280eaea65b50"
}
```

`payload` is JSON kept as a string, so its exact bytes are the ones that were signed. Inside it:

| Field | What it holds |
|---|---|
| `schema` | `1`. A copy that doesn't know a newer schema ignores the file. |
| `version` | The latest release, such as `0.2.3`. |
| `date` | The day it was published (UTC), such as `2026-10-25`. |
| `headline` | The first bold line of its release notes, at most 90 characters. The island shows it. |
| `notes` | The release's page on GitHub. |
| `windows` | `url`, `size` and `sha256` of `Lumo-Setup-<version>.exe`, from that release. |

`signature` is ECDSA P-256 over the SHA-256 of the payload's UTF-8 bytes: r and s, 32 bytes each, big-endian, then base64. `key` names the key that made it.

## Where it is made

The Update feed workflow (`.github/workflows/update-feed.yml`) runs `tools/make_update_feed.py` in its `feed` job and publishes only `latest.json` through GitHub Pages. No landing page or website assets are bundled. Signing stays in the `release` environment. If signing fails, deployment does not run and the existing feed stays available.

The tag's `ci` run starts this workflow after the Windows release is published. Changes to the feed generator or trusted keys on main also refresh the feed, and a manual run can refresh it after changing which release is latest. Release dry runs never start a deployment.

To see what the file would say without a key: `python3 tools/make_update_feed.py --check`.

## The keys

`crates/lumo-core/data/update-keys.txt` lists the public keys the app trusts, and the app is built with that list. The script refuses to sign with a key that isn't on it.

- **The first key signs.** Its private half is the `UPDATE_FEED_KEY` secret in the `release` environment, which only `main` and `v*` tags can use. Never print it or move it out of that environment.
- **The second key is the spare.** Its private half is kept offline by the maintainer. Copies already out there trust both keys.
- **If the first key leaks or is lost,** put the spare in `UPDATE_FEED_KEY`, and in the next release replace the old first key in `update-keys.txt` with a new spare. Every copy keeps hearing about updates.
- GitHub never shows a secret again after it's saved. If both private halves were lost, copies that trust only those keys could never be told about an update again; they would need to download an update from GitHub by hand.

## Taking a release back

The file follows GitHub's latest release. If a release turns out bad, mark the previous release as the latest in the release list, then run the Update feed workflow (Actions › Update feed › Run workflow). Copies that haven't updated stop hearing about the bad one; copies already on it wait for the next release.

## What the app does with it

Copies from 0.2.6 on read the file; 0.2.5 and older don't, so they need one last update by hand. Settings has **Tell me about new versions**, on by default.

- **When it looks:** about two minutes after start, then every 12 hours (`lumo_core::update::CHECK_EVERY_S`), remembered across starts in `update.json` in the data folder, so starting the app often costs nothing. A look that comes due while nobody is at the computer (locked, display off) or on battery saver waits until someone is back. A failed look tries again in an hour.
- **How:** one request for the file, about 1 KB, on the add-on downloader's thread (`services/update.rs`), which sleeps the rest of the time. The signature is checked with the system's own crypto (Windows CNG), then `lumo_core::update::parse_payload` refuses anything that isn't this repository's own release of that version.
- **Windows downloads first, then tells:** once a look finds a later version, the add-on thread downloads its Setup (about 2.5 MB, once per release) into `update` in the cache folder (`%LOCALAPPDATA%\Lumo\update`) and keeps it only if its size and SHA-256 match the signed file's. Older versions' Setups there are deleted, and all of them once the running version is the latest. A download that fails doesn't hold the news back: the island tells anyway, the sheet offers the release page, and the next look tries again.
- **The island:** a later version than the one running, with a Windows installer, shows once per version as **Update available x** (the version dimmer), over the release's headline, with **Hide** (`crate::updates`). It waits until the island is closed and free, and leaves after 15 seconds. Clicking it opens the Update sheet.
- **The Update sheet** (in Settings, in place of its rows): what's new, what gets the new version, and **See what's new in Lumo x** (the release page). **Not now** closes the island. On Windows with the Setup in, it reads **Update ready to install**, with two cards:
  - **Install now** checks the Setup again and runs it silently (`/S /reopen`); the app quits as on any quit, saving first. The Setup replaces `lumo.exe`, leaves Start with Windows as it was, and opens the new version.
  - **Install on next start** remembers the version (`install_on_start` in `update.json`). The next start runs the Setup the same way before any window opens, once: a Setup that fails doesn't keep the app from opening.
- **Until it is installed:** Settings has a last row, **Update available x**, with **Update**, which opens the sheet.
- **Without the Setup** (its download failed): the card opens the release's page.
- **The installer's side** (`installer/lumo.nsi`): in silent mode it leaves the Start with Windows component out, so the Run value stays as the person set it, and `/reopen` starts `lumo.exe` once it's done (the finish page's "Launch Lumo now" never shows in silent mode). The CI installer check runs exactly this over a running copy.
- **After updating:** the first start of a newer version says **Updated to x** once, with **What's new**.
- A Microsoft Store copy doesn't look: the Store updates it.
