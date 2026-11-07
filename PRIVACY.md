# Lumo privacy

Lumo is maintained by mthlpbs, based on Lumo by Jatevo Labs. It needs no account and has no ads, analytics, or tracking. Update checks are opt-in.

## What leaves your PC

**Local lyrics, when selected.** **Local lyrics for music players** in Settings reads `.lrc`, `.txt` and `.lyrics` files from your selected Music folder, or a lyric file you choose for the current song. The app reads these files without changing or uploading them. It searches only when a song or source setting changes, with no background folder watcher. Desktop-player lyrics do not go to LRCLIB or Apple while this mode is on; browser sessions keep their online lyrics source. Turning it off restores online lookups.

**Lyrics lookups.** When a song plays and **Synced lyrics** is on, Lumo asks [LRCLIB](https://lrclib.net), a free public lyrics library, for that song's lyrics. The request carries the song's title, artist, album and length, plus a User-Agent that names Lumo and links to this project. LRCLIB is run by other people and has its own terms. Answers are cached on your PC, so a song isn't looked up again every time it plays.

To stop lookups, right-click the island or the tray icon and turn off **Synced lyrics**.

**Apple Music lyrics, only if you turn them on.** If you add your Apple Music token (right-click the island, then **Apple Music lyrics › Use token from clipboard**), Lumo also asks Apple Music for lyrics the way Apple's web player does. It checks the token, searches Apple Music's catalog for the playing song's artist and title, and downloads that song's lyrics. These requests go to Apple (`music.apple.com` and `amp-api.music.apple.com`) and carry your token; the token goes nowhere else. To stop, choose **Remove token** in the same menu.

**Phone drop, only while its panel is open.** When you open Phone drop on the Shelf, Lumo serves a small web page on your local network so your phone can send files to the Shelf and save Shelf files. Only a device on the same network with that session's secret link, which only the QR code carries, can open it. Files go straight between your phone and your PC: nothing passes through the internet or through us. Lumo reads nothing from an upload until you click **Accept**. The page stops working when you click **Done**, or after 10 minutes without a phone.

**Paired phones, only after you pair one.** **Pair a phone** (Shelf › Phone) shows a QR code with a pairing secret made on your PC. The phone that scans it keeps the secret, and so does your PC, sealed so only your Windows account can read it. From then on, while Lumo runs, it keeps one request open at the upstream meeting server, `drop.oyenza.com`, so the phone can find the PC from any network. The server sees your PC's and phone's IP addresses, when they are online, and an address for the pair that is derived from the secret; it never sees the secret itself, a device name, a file name or a file. Every message through it is signed with the secret, so it can't pretend to be your phone or your PC. Files then go straight between phone and PC, encrypted by WebRTC with keys made on the two devices. When both networks block a direct link, they pass through a relay run by Cloudflare that forwards the encrypted bytes and can't read them. To find its own internet address, Lumo also asks a public STUN server (Cloudflare's or Google's) when a phone connects. **Remove** a phone in **Paired phones** and the pair stops working on both sides; with no paired phones, nothing is sent at all.

**Add-ons are built in.** Script reader and Clipboard history require no downloads. Switching either off stops that feature. Prayer times, Health, Money, and Japanese readings are not included.

**Updates, only when requested or enabled.** Settings > About can check Lumo's public GitHub releases without an account. The request sends your IP address and a User-Agent naming Lumo and its version to GitHub. With automatic updates or new-version notifications enabled, it checks twice a day, resting while locked, the display is off, or battery saver is on. Downloads come only from this repository's versioned installers and are checked against GitHub's size and SHA-256. Automatic updates install on the next launch; switching them off cancels that installation. Both switches are off by default. No settings, personal files, or usage history are sent.

Nothing else is sent anywhere. Music, volume, Bluetooth battery levels, the date and calendar, and the system stats are all read from Windows on your PC.

**Show over games reads Windows' frame log, only while it's on and a game is in front.** To count frames per second, Lumo reads Windows' own log of the frames apps hand to the screen (event tracing, as PresentMon and Xbox Game Bar do) and keeps only the full-screen game's: when each frame came and which window it was for, the last two seconds of them, in memory. Nothing is loaded into the game, nothing is saved and nothing is sent anywhere. The one-time OK adds your Windows account to Windows' Performance Log Users group, which lets programs you run read Windows' performance logs; remove your account from that group to undo it (see the README).

**The Glass theme reads the screen behind the island, only while it's on.** To bend what's under the island, Lumo copies the part of your screen behind its window (the island and the space it opens into) whenever something there changes. It doesn't copy while the island is hidden, while a full-screen app is in front, or while the PC is locked. The copy stays in your PC's memory and on its graphics card, and each new copy replaces the last. It is never saved, never shown anywhere else and never sent anywhere. While Glass is on, Windows leaves Lumo out of screenshots, screen recordings and screen sharing. Switch to Classic or Oyen to stop both.

## What stays on your PC

| What | Where |
|---|---|
| Settings, tasks, focus count and the shelf's list of files (the files themselves stay where they are), and any drinks logged by the Water page of older versions | `%APPDATA%\Lumo\` |
| Lyrics cache and a log file (the log names songs that were looked up, to help fix problems) | `%LOCALAPPDATA%\Lumo\` |
| Your Apple Music token, if you added one, sealed so only your Windows account on this PC can read it | `%LOCALAPPDATA%\Lumo\` |
| Clipboard history, only while that add-on is on (it's off unless you switch it on) | Memory only, never written to disk. Text that password managers mark as private is skipped. |
| The Glass theme's copy of the screen behind the island, only while Glass is on | Memory only (the PC's and its graphics card's), replaced by each new copy, never written to disk |
| Paired phones (their pairing secrets and names), sealed so only your Windows account on this PC can read them | `%LOCALAPPDATA%\Lumo\` |
| Files you accepted from your phone with Phone drop, and files your paired phones sent | `Downloads\Lumo\`, kept when you uninstall |

Uninstalling removes the app, its cache, its log, the add-ons' files, the Apple Music token, the paired phones. If you installed from the Microsoft Store, Windows also removes the files the app created. Data from an earlier copy installed from GitHub stays in `%APPDATA%\Lumo\` until you delete it.

## Microsoft Store

If you install from the Microsoft Store, Microsoft handles the download and updates under the [Microsoft Privacy Statement](https://privacy.microsoft.com/privacystatement), and gives publishers summary reports such as install counts and crash reports.

## Questions

Open an issue at [github.com/mthlpbs/lumo/issues](https://github.com/mthlpbs/lumo/issues).
