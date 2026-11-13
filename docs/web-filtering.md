# Web ad blocking

Settings > Browser has a **Block ads and trackers** switch. It applies to both
the embedded browser and AI websites, using Brave's pinned `adblock-rust`
0.13.5 engine. The former Filter lists tab, URL subscriptions, per-list switches,
custom-filter editor, and handwritten matcher have been removed. Previously
saved custom rules and subscriptions are ignored and are not written back.
The old master-switch value is preserved when loading earlier preferences.

The engine uses managed EasyList and EasyPrivacy snapshots plus replacement
resources. Regex matching, exception rules, public-suffix-aware domain checks,
parameter removal, CSS cosmetics, dynamic class/id matching, and supported
procedural cosmetic actions are handled through Brave's engine. Page scripts
receive only resources selected for their actual document URL. No page-provided
URL is used to choose scriptlets. Trusted-only scriptlet permissions are not
granted to the managed lists.

Blocking applies to intercepted WebView2 resource requests, including iframe
requests. The page bridge handles main documents and direct child frames.
Cosmetic updates are driven by DOM mutations and stop while the page is hidden;
there is no polling or animation-frame loop. Session lock, display-off, and
battery saver cancel data preparation and use the existing WebView suspension
path. Reload open websites after changing blocking to restore previously blocked
resources or undo procedural/scriptlet changes.

This is Brave's filtering engine integrated into WebView2, not the full Brave
browser or uBlock Origin extension. WebSocket connections, response-body/CSP
rewriting, nested child-frame cosmetics beyond the SDK's frame events, and
browser-specific anti-adblock workarounds are not promised. Scriptlet execution
depends on WebView2's asynchronous document bridge and cannot guarantee execution
before every inline site script. The managed resource bundle contains Brave's
published resources and small empty replacements; it is not uBlock's entire
scriptlet library.

## Data and lifecycle

Protection bodies remain outside `lumo.exe`, as a size/SHA-256-verified add-on
declared by `lumo_core::addons::WEB_BLOCKING`. See
[the add-on manifest and staging instructions](../addons/web-blocking/README.md).
Enabling blocking or selecting **Install protection data** prepares it on a
short-lived cancellable worker. There is no scheduled updater or retry timer.
Views with blocking enabled do not silently open unprotected when data is absent;
they show a preparation error until the verified data is ready.

One engine is shared by the browser and AI websites. The cache holds a weak
reference, so closing the last retained web view drops compiled filter data.
Filters are not compiled at startup or while the feature is off. Opening
Settings does not start a worker. Browser controls remain outside the AI
provider dropdown; AI website choices are fixed, and native provider settings stay under AI companion.

The working local build includes the verified external add-on beside its EXE.
For fresh online installs, the maintainer must separately publish the prepared
non-app add-on pre-release; this change does not publish releases.

See [measured binary, memory and CPU costs](adblocking-cost.md).
