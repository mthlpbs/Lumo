# Ask AI (prototype)

Chat uses compact 14-DIP text in both hosts. User bubbles fit their content;
assistant paragraphs, lists and tables keep a restrained hierarchy and readable
line spacing. The composer shares the same scale and native selection metrics.

Enable **AI companion** on the island's **Add-ons** page first. It is off by default. Turning it off hides Ask AI, its Ctrl-hover shortcut and Settings > AI companion, and stops an outstanding reply. Provider settings, saved keys, model preferences and chat history are kept when disabled; turning it back on restores the same settings.

Open AI by scrolling **left** over the closed island; right scroll opens Search. AI is a standalone, browser-sized workspace outside the ordinary page tabs. The existing Ctrl-hover sparkle shortcut also opens it. Its mini-window icon moves the same conversation into a freely resizable window at the top right. Use the matching return icon to bring it back, preserving the draft, attachments, model and reading position. The toolbar has separate **Keep open** and **Resume chat** switches, both off by default. Unpinned AI closes on outside activation; Resume preserves the session on reopening. With Resume off, closing clears unsent text and pending attachment/image buffers, and the next opening starts a fresh chat. Normal saved chats remain in local history. [Workspace controls and screenshots](ai.md).

Ordinary island clicks, hover and the tray icon continue to follow your current activity: music, timers, tasks and other enabled pages. AI drafts do not redirect these openings. Open AI explicitly through its left-scroll gesture, shortcut or reply notice. A reply can finish in the background while you use another page. An explicit mini-window handoff always preserves the current chat, regardless of Resume. Closing the mini window dismisses it; returning to the island is a separate action. Mini size, gesture, pin and resume preferences also live in Settings > AI companion.

**New chat** uses the compose icon; **Temporary chat** uses the dashed speech-bubble icon. Both are icon-only buttons with action labels on hover. Click Temporary to enable it and show a check mark inside the icon; click again to disable it. Switching modes starts a fresh conversation when messages already exist. Your unsent draft, attachments and selected model stay with you. Leaving Temporary never saves its private messages. Send becomes Stop while a request is running. The separate circular progress/status control has been removed. Provider errors still appear above the composer. Daily allowance UI and account stats requests have been removed.

In **Settings → AI companion**, select OpenAI, Claude, Gemini or OpenAI Compatible. Enter and save your own API key. Completion and model-list URLs are editable; Gemini completion URLs can contain `{model}`. Select a default model from the fetched list, or enter its exact ID. Each provider remembers its own configuration and key.

No Hack Club endpoint is configured by default. For a personal Hack Club account, select OpenAI Compatible and explicitly enter:

```
Chat completion: https://ai.hackclub.com/proxy/v1/chat/completions
Model list:      https://ai.hackclub.com/proxy/v1/models
```

**Save model list**, under Settings → AI companion, is enabled by default. The first successful fetch saves the catalog on this PC, separately for each provider and scoped to its configured endpoints. Later opens and restarts reuse it without a network refresh. Click the refresh icon in model selection to update it manually. A failed refresh keeps the usable list; temporary network/server failures receive one short retry. Authentication failures and generation requests are not automatically retried. Saving an API key finishes before its model request starts. Turn Save model list off to fetch once each session. Catalog storage is bounded to 5,000 models / 16 MB per provider; endpoint URLs are represented by a hash in the saved file, and API keys are never included. Corrupt or mismatched files are ignored.

Search by model, provider or capability; searching `free` shows only models advertised as free. Results are ordered by model name without provider headings. Short lists use a compact menu above the composer; long lists scroll. The selected model has a blue row and checkmark, and the list scrolls to it when opened. Free models have a yellow dot and models supporting adjustable reasoning have a violet star, both inside the picker only. The composer shows only the model name in a compact button that fits short names and caps long ones. Image and embedding models have a type label. The configured default is the actual starting model; there is no extra “Default” entry. Hover over a model’s **info** icon for a separate visual summary of advertised image/file input, reasoning, image creation, context size and price. Checkmarks show advertised support; a minus means it was not advertised. Provider names stay hidden in this preview. Click the icon for scrollable details with capability tiles, context and reasoning summaries, availability notices, pricing and additional information: inputs, outputs, capabilities, effort levels, context, price and description. Missing information stays labelled as unknown or not supplied. Embedding models are listed for completeness; choose a language or image model to send a chat.

The funnel icon opens a separate compact filter popover for **Free models**, **Accepts images**, **Accepts files** and **Creates images**. Selected filters have a blue background and checkmark; a count appears on the filter button. Filters combine with each other and the search text without showing provider groups. **Reset** clears them, and **Show models** returns to the matching list with its result count. Choices remain for the session. The picker has no catalog-status footer.

The composer’s compact, text-only model button opens model selection directly. Selecting a different violet-starred model uses its own default effort and closes the picker, clearing any earlier override for that newly chosen model. Selecting the already active starred model again opens the original compact slider. Drag to adjust its supported levels; release saves the setting once. Arrow keys, Home and End also work. The reset icon returns to the model default, and clicking the model name in the popup returns to the model list. Clicking outside or pressing Escape closes the slider. Models with fixed, automatic reasoning select directly without offering unsupported controls. The model picker stays inside the workspace in either host. Clicking outside it or pressing Escape dismisses it and consumes the press, so clicking above Send cannot submit a message. Closing AI dismisses model and attachment popovers. There is no below-island picker, compact-height control or separate model window.

Effort controls use explicit catalog metadata where supplied, with small verified profiles for known native OpenAI, Claude and Gemini models whose list APIs omit effort details. Unknown models and budget-only thinking models do not receive guessed sliders. Compatible endpoints need to advertise `reasoning.supported_efforts`; reasoning capability alone does not imply adjustable effort. Catalog rows are cached only when the list or search changes, and slider drags save once on release. These controls add no idle polling, worker or animation.

Effort references: [OpenAI reasoning and model settings](https://developers.openai.com/api/docs/guides/latest-model), [Claude effort](https://platform.claude.com/docs/en/build-with-claude/effort), [Gemini thinking](https://ai.google.dev/gemini-api/docs/thinking), [compatible catalog effort metadata](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens), [Hack Club model catalog](https://docs.ai.hackclub.com/api/get-models.html).


The **+** menu has separate **Images** and **Documents** choices, with file type/size guidance, plus **Paste image or file**. Capture screen has been removed. File pickers accept PNG, JPEG, WebP or GIF images and PDF or UTF-8 text documents (.txt, .md, .csv, .json, .log, .yaml and .yml), according to the chosen action. The island stays open while a picker is active, duplicate opens are prevented, and cancellation is handled without retrying or showing an error. Picker initialization failures receive one retry; remaining failures are reported. **Ctrl+V** in the message field pastes a copied image or up to four copied image, PDF or text files in both the island and mini chat window; text paste still works in chat and other fields. Both Windows bitmap and registered PNG clipboard formats are supported. Clipboard access happens only for an explicit paste. Image thumbnails and document cards sit inside the composer above the draft, with filename tooltips and individual remove buttons. Thumbnails decode once on the existing attachment worker. Files are sent only when you press Send. Each attachment is limited to 6 MB; up to four can be attached at once, with an 18 MB combined request-context budget. The provider and selected model decide which inputs they support. There is no AI clipboard listener or capture timeout.

Choose an image model to generate images. Generated images have **Copy image**, **Download** and **Use in prompt** actions. Reuse adds the image to the composer for a follow-up instruction. OpenAI uses its image generation/edit endpoints; Gemini and compatible image-chat endpoints use multimodal chat requests. Claude accepts image/PDF inputs but cannot generate images.

Replies display native Markdown headings, bold/italic text, lists, tables, quotes, links and code blocks. Fenced code and display LaTeX (`$$…$$` or `\[…\]`) have their own **Copy** button, which stays reachable while scrolling a long block and copies its exact source. LaTeX is shown as source. Message actions appear as icons only while hovering over that message; each icon has a tooltip. **Copy as plain text** copies readable text; **Copy Markdown** preserves the original response. **Retry** resends the last user message. Enter sends; Shift+Enter adds a line. Ctrl+A and Ctrl+V work in text fields. A pending reply shows three animated dots and a stop control. The dots animate only while visible, pause when locked, the display is off or battery saver is on, and stop when a reply completes. Stopping cancels Lumo's outstanding HTTP operation; a provider may still bill work it has already completed.


**History** filters local chat titles and reopens or deletes individual chats. **Temporary** starts a conversation that is never saved and opens a dedicated screen explaining its history behavior; the toolbar's **New** action keeps the current normal/temporary mode. Ctrl+N starts a normal chat and Ctrl+Shift+N a temporary chat. Settings can delete all saved chats. History and API keys are encrypted on this PC using the Windows account's DPAPI key. Nothing is uploaded until a request is sent to the configured provider. History is capped at 40 chats / 32 MB; a chat accepts 12 turns, with an explicit prompt to start a new chat instead of silently discarding context.

When a reply arrives while its chat is out of view, a quiet, clickable **AI reply ready** notice appears on the closed island, including over Music, Focus and maximized/full-screen apps. It stays available until opened or dismissed and plays no sound. Clicking opens Ask AI at that exact reply. Other notices may appear briefly; an unread AI reply returns afterward. Windows Widgets retains priority. The ready notice is static and has no expiry timer or background polling. Chat and model transitions use finite animation; there is no idle AI worker, timer or network polling. History, HTTP clients and image decoding load only when used. Markdown parsing/layout is cached and bounded. A Ctrl-key observer exists only while hovering over the closed island, and is removed on leave, expansion, lock, display-off or battery saver; there is no modifier polling. No new crates or bundled models are required.

Protocol references: [OpenAI file inputs](https://developers.openai.com/api/docs/guides/file-inputs), [OpenAI image edits](https://developers.openai.com/api/reference/resources/images/methods/edit), [Claude Messages](https://platform.claude.com/docs/en/api/http/messages), [Gemini content generation](https://ai.google.dev/api/generate-content), [Hack Club image generation](https://docs.ai.hackclub.com/api/image-generation.html).

The composer model selector sits at the bottom right beside Send. Model lists show model names without provider section headings. Toolbar and message action icons show explanatory hover labels. The composer’s Attach, model selector and Send/Stop controls show no hover text; model provider names stay hidden on hover. Explicit model information still includes provider details. Provider search, capability filters and reasoning controls remain available.

Drag supported files onto the open AI workspace or mini window to attach them without switching to Shelf. This also respects Keep open. Single clicks place the caret in the message and Search text fields; double-click selects all text. Wrapped-line selections use native DirectWrite geometry, including Unicode. Replies and ordinary text use native Windows color emoji.

Empty AI and Search text fields have no placeholder text or visible caret.
Once the field contains text, its focused caret blinks at the Windows setting
using one-shot paints. Clearing the text hides the caret and stops its blink wake. Attachment menus dismiss on a
background press and consume it without triggering the underlying button. AI
Settings is reached from the tray Settings action; the toolbar gear is removed.


## Model availability

Model selection disables models with explicit deprecation/unavailability metadata,
including announced `shutdown_date` or `expiration_date` retirement dates. Future
retirement dates also count as deprecated. Unknown status stays selectable; a
model's name, age or description is never used as a retirement signal. Select
**ⓘ** to inspect the reason. A disabled current model also disables Send; Stop
remains available during an existing request. Lumo preserves the model and draft
rather than substituting a different model.

**Hide unavailable models** defaults to on in **Settings > AI companion**. This
switch appears only when a configured endpoint uses the exact Hack Club AI host
(`ai.hackclub.com`). Turning it off displays disabled rows with accessible details.
The preference is saved with the provider profile; other providers always retain
visible disabled rows.

Hack Club documents `GET /proxy/v1/models` as its live catalog in OpenRouter's
format. Its public catalog includes `expiration_date`; a null value does not
prove that generation will work. The static model-list page can lag the live
catalog. After a successful refresh, a saved/current Hack Club model missing
from that catalog remains disabled with an explanation. Saved lists require
**Refresh** to learn about later changes. The service-wide health check reports
infrastructure status rather than individual model retirement, so Lumo does not
use it to disable models.

An actual Send can also reveal a model-specific refusal: for example,
`model_not_found`, `model_deprecated`, or an explicit message naming the rejected
model. Lumo disables that ID for the request's endpoint during this app session.
It does not disable another model selected while the request was running.
Authentication failures, rate limits, outages, invalid request parameters and
ambiguous routing failures remain retryable. These checks cannot identify every
failure before the first request.

A successful live **Refresh** clears session refusal flags and rechecks catalog
metadata. Loading a cache or a failed refresh keeps them. No paid test requests,
model-by-model health probes, background polling or automatic model substitution
are added. Existing catalog requests and the explicit generation response supply
all the information.

References: [Hack Club live models](https://docs.ai.hackclub.com/api/get-models.html),
[Hack Club model-list freshness](https://docs.ai.hackclub.com/models-list.html),
[Hack Club health check](https://docs.ai.hackclub.com/api/healthcheck.html),
[OpenRouter model schema](https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties),
[OpenAI shutdown metadata](https://developers.openai.com/api/docs/deprecations).


AI, browser and music mini windows support diagonal resizing from all four
rounded corners, in addition to horizontal/vertical edge resizing. Corner hit
targets scale with Windows DPI and extend into the visible rounded surface.
Windows supplies the resize cursors and sizing interaction. Resizing preserves
the current conversation, page or playback.

The filter popover keeps the model list visible behind it. Its count badge shows
how many filters are active, and **Reset** clears them. Click outside or press
Escape to dismiss only the filter; the next Escape returns from details or
closes the picker. Background controls do not activate through the popover,
including in Settings. No hover delay timer, health probe or idle animation is
needed for the quick capability preview.

The filter popover stays anchored while options change, even when a long catalog
shrinks to just a few matches. The result count updates without moving the
checkboxes.


With a mouse, wheel **up** over the closed island to open AI, or **down** to open
Search. Over the Search/AI header, another deliberate gesture returns to Time before a later gesture opens a workspace.
Inside the chat, wheel scrolling still reads the conversation. Touchpad left/right
gestures retain their current behavior. **Settings > General > Mouse wheel
shortcuts** is enabled by default; Shift+wheel retains the compact clock/system/
network views. See [input compatibility](search.md#mouse-and-touchpad-input).


The AI toolbar starts eight DIPs closer to the top in both hosts; the composer
keeps its bottom spacing. Unpinned AI dismisses on an outside press even when
wheel opening did not activate it. Its short-lived outside-click observer does
not record input and is absent while closed, pinned, locked or display-off.
Touchpad reversals and renewed strokes work without a finger lift; small inertia
is ignored. A workspace scroll gesture closes to Time first instead of jumping
directly from AI to Search. Chat content scrolling remains reserved for reading.
