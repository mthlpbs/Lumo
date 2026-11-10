# Lumo privacy

Lumo is maintained by mthlpbs, based on Lumo by Jatevo Labs. It needs no account and has no ads, analytics, or tracking. Update checks are opt-in.

## What leaves your PC

**Local lyrics, when selected.** **Local lyrics for music players** in Settings reads `.lrc`, `.txt` and `.lyrics` files from your selected Music folder, or a lyric file you choose for the current song. The app reads these files without changing or uploading them. It searches only when a song or source setting changes, with no background folder watcher. Desktop-player lyrics do not go to LRCLIB or Apple while this mode is on; browser sessions keep their online lyrics source. Turning it off restores online lookups.

**Lyrics lookups.** When a song plays and **Synced lyrics** is on, Lumo asks [LRCLIB](https://lrclib.net), a free public lyrics library, for that song's lyrics. The request carries the song's title, artist, album and length, plus a User-Agent that names Lumo and links to this project. LRCLIB is run by other people and has its own terms. Answers are cached on your PC, so a song isn't looked up again every time it plays.

To stop lookups, right-click the island or the tray icon and turn off **Synced lyrics**.

**Apple Music lyrics, only if you turn them on.** If you add your Apple Music token (right-click the island, then **Apple Music lyrics › Use token from clipboard**), Lumo also asks Apple Music for lyrics the way Apple's web player does. It checks the token, searches Apple Music's catalog for the playing song's artist and title, and downloads that song's lyrics. These requests go to Apple (`music.apple.com` and `amp-api.music.apple.com`) and carry your token; the token goes nowhere else. To stop, choose **Remove token** in the same menu.

**Phone drop, only while its panel is open.** When you open Phone drop on the Shelf, Lumo serves a small web page on your local network so your phone can send files to the Shelf and save Shelf files. Only a device on the same network with that session's secret link, available through the QR code or **Copy link**, can open it. Files go straight between your phone and your PC: nothing passes through the internet or through us. Lumo reads nothing from an upload until you click **Accept**. The page stops working when you click **Done**, or after 10 minutes without a phone.

**Local address and received files.** Phone Drop uses the PC's LAN IP in a temporary secret link. **Copy link** writes that link to the clipboard only when clicked. New accepted files save directly in Windows Downloads, or the folder chosen in **Settings > General > Phone Drop**. Folder changes apply to new sessions. Previously received files stay in place. Internet pairing has been removed, and saved legacy pair secrets are neither loaded nor used to connect.

**Focus completion lighting.** The optional spotlight draws a soft glow spreading from the island across the upper display when a focus session ends, or when you click **Preview** in Settings > Focus. Its window passes mouse input through and does not take keyboard focus. The effect captures no desktop contents, reads no input and sends no network requests. Its bitmap and window are released when the effect ends, is disabled, or Windows locks, turns off the display or enters battery saver. Automatic completion lighting is off by default.

**Add-ons are built in.** Script reader and Clipboard history require no downloads. Switching either off stops that feature. Prayer times, Health, Money, and Japanese readings are not included.

**Updates, only when requested or enabled.** Settings > About can check Lumo's public GitHub releases without an account. The request sends your IP address and a User-Agent naming Lumo and its version to GitHub. With automatic updates or new-version notifications enabled, it checks twice a day, resting while locked, the display is off, or battery saver is on. Downloads come only from this repository's versioned installers and are checked against GitHub's size and SHA-256. Automatic updates install on the next launch; switching them off cancels that installation. Both switches are off by default. No settings, personal files, or usage history are sent.

Nothing else is sent anywhere. Music, volume, Bluetooth battery levels, the date, and the system stats are all read from Windows on your PC.

**Show over games reads Windows' frame log, only while it's on and a game is in front.** To count frames per second, Lumo reads Windows' own log of the frames apps hand to the screen (event tracing, as PresentMon and Xbox Game Bar do) and keeps only the full-screen game's: when each frame came and which window it was for, the last two seconds of them, in memory. Nothing is loaded into the game, nothing is saved and nothing is sent anywhere. The one-time OK adds your Windows account to Windows' Performance Log Users group, which lets programs you run read Windows' performance logs; remove your account from that group to undo it (see the README).

**The Glass theme reads the screen behind the island, only while it's on.** To bend what's under the island, Lumo copies the part of your screen behind its window (the island and the space it opens into) whenever something there changes. It doesn't copy while the island is hidden, while a full-screen app is in front, or while the PC is locked. The copy stays in your PC's memory and on its graphics card, and each new copy replaces the last. It is never saved, never shown anywhere else and never sent anywhere. While Glass is on, Windows leaves Lumo out of screenshots, screen recordings and screen sharing. Switch to Classic or Oyen to stop both.

**AI companion, only when enabled.** AI uses your configured provider and API
key. Sending a message sends its conversation context and any selected
attachments to that provider. Clipboard images and copied image, PDF or text files are
read only when you paste or choose the paste action. Attachment thumbnails
are decoded locally; AI does not monitor the clipboard or capture your screen.
Dropped files are read locally only after you release them onto the open AI
workspace or its mini window. They are sent to the provider only with Send.
Opening the workspace, resizing it, switching between island
and mini window, and using prompt starters do not send a conversation. Existing
model discovery can contact your configured provider when its model list is
needed. Provider terms and usage charges apply.

**AI resume and local history.** Keep open and Resume chat are off by default.
Resume retains the current conversation, unsent draft, pending attachments and
reading position in memory during this app session; turning it on does not
save those drafts or attachments to disk. With Resume off, reopening starts a
fresh chat; closing also clears unsent text and pending attachment/image buffers.
Saved conversations remain available in local history. Moving to
the mini window or back always preserves the current state. Saved normal chats
use the existing Windows-account encryption; temporary chats stay out of saved
history. Searching history matches local titles and does not contact a provider.
The gesture, pin, resume and mini-window size preferences are saved in Lumo's
settings. Mini-window animations and timers stop while locked, display-off or
battery saver is on; the window is restored without taking focus afterward.

## What stays on your PC

**Outside-click dismissal.** While an unpinned Search or AI workspace is open,
Lumo temporarily observes mouse-button presses so it can close even if Windows
did not activate its window. The callback identifies the clicked window, ignores
clicks inside Lumo and native menus, and posts a local dismissal message. It does
not record positions, clicks, window titles, typed text or browsing activity.
It is removed when the workspace closes, Keep open is on, an attachment picker
or result menu is active, the screen locks or the display turns off. There is no
polling, background input log or network request.

**Local Search.** Windows app names and opaque launch identifiers, plus a
bounded set of native app/tool icons, are cached in memory. Search covers registered
apps, Windows Settings and optional local integrations; general file/folder
search and custom search locations are removed.
Query text is not logged or saved. Resume page can retain the current query
in memory; clearing that Search state discards it.
Frequently opened result identifiers, counts and last-use times are saved
in data.json. Remove them individually from a result's context menu or clear
them in Settings > Search. Retired file-result counts are filtered from the
loaded history, and old file-search configuration is ignored.

**Application icon copying.** Only an explicit result-menu action extracts an
app icon through Windows. Copy icon image publishes PNG and a Windows bitmap
to the clipboard. Copy icon file creates an ICO under
`%LOCALAPPDATA%\Lumo\icon-exports\` and places that file on the clipboard for
pasting in Explorer. Lumo keeps at most 32 generated exports, cleaning older
exports after successful file copies. Nothing is uploaded. Windows clipboard
history/sync follows your Windows preferences.

**Optional local search integrations.** Windows services are off by default.
PowerToys tools and VS Code commands are enabled by
default and can be switched off independently in Settings > Search. These
sources read only when Search opens or you refresh them. Services are read
through Windows' service manager; starting or stopping one requires an explicit
action and may show Windows' administrator prompt. Lumo does not change startup
types or stop dependent services.

VS Code command discovery reads open Code/Code Insiders window titles and
temporary window/process identifiers. Commands use an existing window and
never launch VS Code. PowerToys activation uses its existing local tool events.
Search text is never a shell command.

Browser bookmark/tab discovery, browser actions and saved custom app actions
have been removed. Lumo does not read your browser's bookmark files, tab strip,
history, password store or cookies for local Search. Retired browser-content
and custom-action preferences are ignored and omitted when settings are saved.
The previous App actions toggle migrates to VS Code commands.

**Optional web search.** Off by default. Choosing the explicit web-search action
sends the query to your selected Bing, Google, DuckDuckGo or Brave provider.
Typing in local Search does not send it unless you enable Google suggestions
as described below. Pages use the installed Edge WebView2
Runtime inside Lumo and follow the provider's privacy policy. WebView2 keeps
cookies and browser data in its separate SearchWeb profile under Lumo's local
cache. Closing Search closes its browser controller; the profile remains on
disk. The packages include Microsoft's small loader, not a bundled browser.

**Google suggestions, only if you enable them.** Off by default in
**Settings > Search**. When local search has no matches, or you type a search
in the focused address editor, Lumo sends that text to
`suggestqueries.google.com` after typing pauses for 350 ms. Website addresses
are excluded. Clicking a page title without typing does not request suggestions.
Google receives that text and your IP address under its privacy policy.
Suggestions are held in memory, never logged or saved by Lumo. Changing the
query, leaving address editing, closing Search or the mini browser, switching
suggestions off, locking the session,
turning off the display or entering battery saver cancels pending work.
Suggestions do not start the embedded browser. Nothing is requested while
Search is closed, except when you explicitly edit a detached mini browser.

| What | Where |
|---|---|
| Settings, tasks, focus count and the shelf's list of files (the files themselves stay where they are), and any drinks logged by the Water page of older versions | `%APPDATA%\Lumo\` |
| Lyrics cache and a log file (the log names songs that were looked up, to help fix problems) | `%LOCALAPPDATA%\Lumo\` |
| Your Apple Music token, if you added one, sealed so only your Windows account on this PC can read it | `%LOCALAPPDATA%\Lumo\` |
| Clipboard history, only while that add-on is on (it's off unless you switch it on) | Memory only, never written to disk. Text that password managers mark as private is skipped. |
| The Glass theme's copy of the screen behind the island, only while Glass is on | Memory only (the PC's and its graphics card's), replaced by each new copy, never written to disk |
| Legacy paired-phone secrets and names, inactive and sealed for your Windows account | `%LOCALAPPDATA%\Lumo\` |
| Accepted Phone Drop files | Windows Downloads directly, or your chosen folder, kept when you uninstall |
| Previously received files | Their existing locations, including `Downloads\Lumo\`, kept when you uninstall |

Uninstalling removes the app, its cache, its log, the add-ons' files, the Apple Music token, the paired phones. If you installed from the Microsoft Store, Windows also removes the files the app created. Data from an earlier copy installed from GitHub stays in `%APPDATA%\Lumo\` until you delete it.

**Retired AI usage add-on.** Lumo no longer reads provider quotas, connects account profiles or schedules usage refreshes. On upgrade it restores any Claude status-line handler installed by the retired add-on, without replacing a handler you edited afterward. This cleanup reads only the old configuration and its handler backup; it performs no provider request.

## Microsoft Store

If you install from the Microsoft Store, Microsoft handles the download and updates under the [Microsoft Privacy Statement](https://privacy.microsoft.com/privacystatement), and gives publishers summary reports such as install counts and crash reports.

## Questions

Open an issue at [github.com/mthlpbs/lumo/issues](https://github.com/mthlpbs/lumo/issues).
