# AI websites

Ask AI's provider selector contains native **Ask AI**, **ChatGPT**, **Gemini**,
**Claude** and **Muse**. These website choices are fixed.
Custom website management and the Chat websites settings subsection are removed;
legacy custom sites are ignored when preferences load. Provider/API configuration
stays under Settings > AI companion. Shared ad blocking stays in Settings > Browser.

A website fills the island or detached window. The small overlay contains its
provider selector, Keep open, mini-window/return and Close. The local API chat's
Resume preference does not discard a website. Closing and reopening the island
retains the same current website, including its response, page state and draft.
Only one selected website is retained; switching providers or exiting Lumo releases
it. Moving between hosts reparents the same controller.

Sites share the existing WebView2 user-data profile with Lumo's browser. Their
cookies and storage persist according to the website. This does not import
cookies from desktop browsers or send Lumo's native API key to the sites. Each
website controls its own chat history, login and temporary-chat behavior.

## Reply-ready previews

After an explicit website submit finishes while hidden, Lumo shows a persistent
reply-ready notice with a short assistant preview. Opening it returns to the
current conversation. Hiding the notice does not reset that page. Responses are
not imported into the native API chat history.

The bridge watches DOM changes only while an explicit request is active. It
recognizes assistant-message roles and generation controls. Muse additionally
supports agent/AI-element roles and a single trailing four-second quiet check
when a new assistant response arrives without a stop-button phase. Response
text mutations reset that check even beyond the bounded preview. This fallback
is a UI heuristic; an unusual long pause without any generation indicator can
look complete. Provider markup changes may require adapter updates.

Startup deadlines prevent failed or unsupported submits retaining a running
observer indefinitely: 15 seconds normally, two minutes for Muse's agent startup.
A real generation signal or new Muse response cancels the startup deadline.
There is no polling, request interception or idle observer. Completion/cancellation
clears the observer and timers. Only a bounded assistant preview is posted to
native code; login fields and cookies are not read by this bridge.

## Resource behavior and verification

No site starts at app launch or when opening the provider menu. The selected
site is created only when opened, using the installed shared WebView2 Runtime.
Hidden idle pages are suspended, preserving their state in memory. An explicitly
submitted response can finish while the island is closed, then the page suspends.
Lock, display-off, battery saver, Widgets and minimizing the detached window
suspend it even during a request. If the runtime cannot suspend, Lumo retains
the conversation address for restoration rather than resetting to the homepage.

Retaining a suspended website trades memory for continuity; it does not keep
an idle renderer loop running. One retained site is the bound. No new crate,
bundled browser or startup network request is introduced.

Offline native fixtures cover all five adapters, Muse's hidden-host quiet
completion, persistent notifications, same-controller handoffs, page retention
with local Resume off, cookies in an isolated profile, and rest states. No test
prompt is sent to an AI service. Signed-in provider flows were not tested against
user accounts.
