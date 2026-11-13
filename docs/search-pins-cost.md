# Search app pins: cost and verification

Search now uses a full-width field and a shared strip of pinned/recent apps.
Lumo Browser is a built-in entry, pinned by default. New profiles enable Browser;
existing explicit preferences remain unchanged. A disabled Browser stays hidden.
The browser runtime still starts only for an explicitly opened website; the
native starting hint needs no WebView or network request.

Pins store only up to 32 app IDs (maximum 4096 bytes per ID) in the existing
profile, without copied shell targets or queries. Loading sanitizes at most 256
records. The strip resolves enabled catalog apps, omits duplicate recent apps,
and reuses the existing bounded icon cache and on-demand icon worker. At most
16 icons are visible; scrolling/keyboard input reveals overflow. Matching pins
are ranked before other results, before the 100-result cap is applied.

Dragging reacts to pointer events and commits one store save on release. There
is no drag timer, poll, idle worker or automatic animation. The small pin badge
uses the existing vector pin icon. Removing the separate browser button also
removes its field-width animation. The existing finite result-clearing fade
remains and snaps in battery saver.

No dependency or asset was added. Installer/download size and idle memory were
not remeasured: packaging is left to the maintainer as requested. No package or
release is created for this change.
