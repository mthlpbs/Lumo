# Lumo brand assets

## Current README screenshots

Refreshed on 13 November 2026 from version **2026.1113.6**. These are
app captures with fictional chat, music, phone and metric data.
Search uses installed Windows application icons. No AI provider request is
made. The README uses the hash-based filenames below to refresh image caches.
These documentation images are not bundled in Lumo.

The opening README image, `banner.png`, is a desktop preview,
not a full-screen app capture. It places the original `music-f8c50633d075.png`
at native size against the top edge of an illustrated Windows 11 desktop.
The desktop background was generated with the built-in imagegen tool; the
Lumo panel is composited from the native capture so its controls and text
are unchanged. Prompt: a clean Windows 11 desktop with blue Bloom-style
wallpaper, a bottom taskbar, no open windows and space at the top for Lumo.
The banner keeps a stable filename so older README previews also resolve it.

Focus and System have transparent desktop backgrounds as of 14 November 2026.
Their original self-test opacity masks replace the composited desktop gradient;
opaque app pixels, canvas sizes and framing are preserved. The original captures
remain available as `focus-a41cffafcbd0.png` and `system-a35e9bd90dcd.png`.

| Image | Capture source | Theme / window |
| --- | --- | --- |
| `music-f8c50633d075.png` | `music_player_controls_fit_with_and_without_lyrics_at_supported_sizes` | Classic |
| `search-c92134758acb.png` | `search_opens_as_a_bar_and_recent_icons_then_expands_only_for_text` | Pill |
| `mini-player-a2576a6eba0d.png` | `lyrics_player_renders_resizes_controls_scrolls_and_follows_the_current_track` | Mini window |
| `compact-performance-5c55c5f1b14e.png` | `layouts_render_in_both_themes_and_scroll_has_three_views` | Pill |
| `compact-network-a84605893881.png` | `layouts_render_in_both_themes_and_scroll_has_three_views` | Pill |
| `ai-607aea43711c.png` | `ai_workspace_preserves_handoffs_resume_pin_and_pauses_while_away` | Pill |
| `phone-076dea3a298b.png` | `search_scroll_and_local_phone_drop_layouts` | Pill |
| `system-e458c29a1838.png` | `self-test: page-system`, restored native opacity | Classic |
| `focus-1f6cacf75a00.png` | `self-test: page-focus`, restored native opacity | Classic |
| `pill-9f103a9a843f.png` | `music_player_controls_fit_with_and_without_lyrics_at_supported_sizes` | Pill |
| `section-tabs-97789f0ed028.png` | `section_tabs_restore_icons_and_active_name_with_wheel_navigation` | Pill |
| `search-single-6045b9e9789c.png` | `inline_search_renders_icons_keyboard_filters_and_collapses_in_both_themes` | Pill |
| `ai-attachment-menu-3dba1b9c5587.png` | `attachment_background_press_dismisses_without_sending_and_keeps_composer_focus` | Mini window |

The native tests above are ignored desktop checks. Set `LUMO_AI_QA`,
`LUMO_SEARCH_QA`, `LUMO_SEARCH_REWORK_QA`, `LUMO_LYRICS_QA`,
`LUMO_COMPACT_QA` and `LUMO_PHONE_QA` to the capture directory, then run
each named test in a separate process with `--ignored --test-threads=1`.
Focus and System use `lumo.exe --selftest <output>` with
`LUMO_SELFTEST_DPI=144` and `LUMO_SELFTEST_THEME=classic`.
Copy new PNGs unchanged and use the first 12 SHA-256 digits in each name.
When a fresh capture has identical pixels, keep the existing optimized PNG
and its original capture name.
The current manifest above takes precedence over older capture references below.

## Capture workflows and previous screenshots

Current compact captures (unmodified offline native fixtures):

- `compact-performance-3af96e748bdd.png` and `compact-network-5a04a350b9e9.png`
  come from `app::closed_checks::layouts_render_in_both_themes_and_scroll_has_three_views`
  with `LUMO_COMPACT_QA`. Values are fictional and checked in Classic/Pill at
  100–200% scaling. The old AI usage and FPS/temperature compact captures are
  historical and no longer appear in the README.
- `ai-40487f69a121.png` and `ai-mini-bfd155a9f064.png` show the compact chat
  type scale and fitted user bubbles from the native handoff fixture.
- `ai-markdown-329cef159b83.png` and `ai-attachments-f99bc46b767d.png` come
  from the native AI-window Markdown/composer checks with `LUMO_AI_QA`.
  These captures use local fictional content and no provider request.
- `clock-settings-ad012b83baf1.png` comes from
  `settings_window::tests::settings_window_renders_and_reopens` with
  `LUMO_SETTINGS_QA`, showing Appearance's font, color and gesture controls.

`lumo/` contains the supplied Windows Dynamic Island logo, including its
original ICO, PNG and SVG files and monochrome variants.

`python tools/make_brand.py` copies the supplied ICO to
`crates/lumo/res/lumo.ico` and generates the installer images from that logo.
It also updates `lumo-logo.svg`, `lumo-logo-512.png` and `social-preview.png`.

The character theme is removed. Any historical artwork retained under `art/`
is unused source material and is not included in the application or installer.

`readme/lumo/` contains unmodified screenshots from Lumo's own graphical
self-test, rendered in Classic and Pill with demo data.
The README uses these current Windows captures rather than historical
upstream artwork. They are documentation assets, not bundled app resources.

To refresh them, run `lumo.exe --selftest <output>` with
`LUMO_SELFTEST_DPI=144` and `LUMO_SELFTEST_THEME=classic` or `pill`, then copy
the captures below without changing their contents. Use the scene suffix to
find each capture; its numeric prefix can change when scenes are removed.

Image filenames include the first 12 characters of their SHA-256 hash. Update
the README image paths when refreshing screenshots so previews request the
new images instead of reusing older cached assets:

| README image | Capture | Theme |
| --- | --- | --- |
| `music-f8c50633d075.png` | `music_player_controls_fit_with_and_without_lyrics_at_supported_sizes` | Classic |
| `ai-40487f69a121.png` | `ai_workspace_preserves_handoffs_resume_pin_and_pauses_while_away` | Pill |
| `phone-076dea3a298b.png` | `search_scroll_and_local_phone_drop_layouts`, Phone Drop IP scene | Pill |
| `system-c0ec9b9543f4.png` | `*-page-system.png` | Classic |
| `focus-922e0d5da670.png` | `*-page-focus.png` | Classic |
| `pill-9f103a9a843f.png` | `music_player_controls_fit_with_and_without_lyrics_at_supported_sizes` | Pill |

Search uses unmodified native launcher-check captures in Pill, with icons
from the Windows app catalog:

- `search-2fc837ec498f.png`: frequent results.
- `search-single-6045b9e9789c.png`: one matching result.
- `search-empty-dbb7ccf50536.png`: the compact web-search action.
- `search-suggestions-2662ff4cf84f.png`: opt-in suggestions with injected demo results.

Run the ignored
`app::tests::inline_search_renders_icons_keyboard_filters_and_collapses_in_both_themes`
test with `LUMO_SEARCH_QA` set to an output directory to refresh them.

The browser preferences capture `browser-preferences-b1370d7150ce.png` comes from
`settings_window::tests::search_preferences_configure_gestures_sources_and_explicit_actions`
with the same output variable. It shows a customized Wide layout; Tall
is the default, pages stay at normal size, and Google suggestions default to off.
Older browser screenshots are retained as historical package references.

The mini player preview `mini-player-f7c19e5dd0f8.png` is the unmodified
430x640-DIP capture from
`lyrics_window::tests::lyrics_player_renders_resizes_controls_scrolls_and_follows_the_current_track`.
Run it with `LUMO_LYRICS_QA` set to an output directory.

`search-visit-b8f75856755c.png` comes from the native launcher check and shows
a bare domain opening directly through Visit.

`search-sources-1b1967aab3b0.png` shows customized source toggles from the native
Settings check. Windows services default to off; VS Code commands default to on.
The retired browser-content and custom-action controls are absent.
`search-service-3aeea0b85788.png` uses a fictional service in the native launcher
check; the test validates the Stop command without executing it.

`native-context-menu-e5493623e2f2.png` is the unmodified native HMENU capture from
`native_menu::tests::native_menu_renders_and_dismisses_without_invoking`.
It previews app actions without invoking a command or changing the clipboard.

AI workspace captures (unmodified native fixtures, no provider query):

- `ai-e43f56934b85.png`: standalone workspace in Pill, full conversation and draft.
- `ai-empty-89965f2e8c4f.png`: empty state, draft-only starters and disabled Send.
- `ai-mini-d2e7b8f28523.png`: mini handoff with a fictional PDF attachment.
- `ai-settings-f2ec13657b49.png`: Settings preferences and provider controls.

Workspace captures come from
`app::tests::ai_workspace_preserves_handoffs_resume_pin_and_pauses_while_away`
with `LUMO_AI_QA` set. Settings uses
`settings_window::tests::settings_window_renders_and_reopens` with
`LUMO_SETTINGS_QA`. Legacy compact AI captures are historical references.

Composer previews and the temporary-chat opening screen come from
`ai_window::tests::composer_previews_temporary_mode_and_right_aligned_model_fit_both_hosts`
with `LUMO_AI_QA` set. The images are unmodified native fixture captures:

- `ai-attachments-467d77cfca65.png`: locally generated sample image and fictional text file.
- `ai-temporary-548e07f3ac4e.png`: the separate temporary-chat opening screen.

`ai-emojis-d015b6465011.png` is the unmodified output of
`ai_window::tests::color_emojis_render_in_plain_text_and_markdown`, using the
installed Windows emoji font in ordinary and styled text. The native check
asserts colored pixels in both rows and also runs with software rendering.

Current native UI fixtures:

- `section-rail-d15359f111ef.png`: `app::tests::vertical_section_rail_scrolls_without_labels_clock_or_settings_icon`, with no media playing.
- `ai-blank-7c124bd648e2.png`: `ai_window::tests::attachment_background_press_dismisses_without_sending_and_keeps_composer_focus`, with an empty composer after dismissal.

`section-tabs-b0e1cb33fb14.png` shows restored icon-and-label navigation from
`app::tests::section_tabs_restore_icons_and_active_name_with_wheel_navigation`.
The earlier section-rail capture is historical.

Expanded Music reference-layout captures (unmodified offline native fixtures):

- `music-02dc5f816f9b.png` and `pill-b4cc877886a2.png`: Classic/Pill, header mini-player button beside Pin, LRC text toggle and right-side lyrics. From `app::tests::lyrics_popup_handoff_visibility_idle_clock_and_return_work_in_both_themes` with `LUMO_LYRICS_QA`.
- `music-wide-a78e94a12b85.png`: full-width player with the lyrics column disabled. From `app::tests::music_player_controls_fit_with_and_without_lyrics_at_supported_sizes`.

The sample track, artwork and lyrics are local test fixtures; no provider request is made. Earlier Music captures remain historical references.

Earlier full-width Music captures:

- `music-355381fc766f.png` and `pill-7a8d26ca5d66.png`: Classic/Pill full-width player with the mini-player control beside Pin, a text LRC toggle and widely spaced transport controls.
- `music-wide-0c340451772d.png`: full-width layout with global lyrics disabled.

The current `music-f8c50633d075.png` and `pill-9f103a9a843f.png` restore inline
lyrics, with the same player controls and fixed header mini-player position.
These are unmodified native fixture captures from the Music layout check above,
with `LUMO_LYRICS_QA` set. Both lyrics-on and lyrics-off layouts are checked at
480/620 DIPs and 100–200% scaling.


The current 14-DIP AI captures and compact metric captures come from the same
offline fixtures. `ai-unavailable-00c9df740e4c.png` shows an invented retired model in
`ai_window::tests::unavailable_models_cannot_select_or_send_and_hackclub_can_hide_them`.
Its disabled footer and availability reason are native UI output. No account
keys or personal conversations appear in these captures. The matching tests
run with GPU and software rendering. Older captures remain historical references.


Current model-picker captures (unmodified offline native fixtures):

- `ai-model-preview-9d3520e11343.png`: model preview.
- `ai-model-details-ddfda8932dd6.png`: model details.
- `ai-model-filters-663e3c18ef6c.png`: model filters.
- `ai-model-list-3d95f46d102a.png`: model list.
- `ai-unavailable-98409e6c4599.png`: unavailable.

Quick preview comes from `ai_window::tests::model_info_hover_previews_capabilities_and_filters_dismiss_locally`; model details, catalog and filters come from `model_picker_selection_filters_details_and_background_dismissal_work`. The unavailable-model fixture uses an invented retired model. All run in GPU and software rendering. Diagonal corner checks exercise AI, browser and lyrics windows at 100–200% scaling; small-window filter captures also cover 360 × 360 DIPs.


`settings-mouse-wheel-c515847a7701.png` is an unmodified native Settings fixture capture from
`settings_window::tests::settings_window_renders_and_reopens`, with the General
page scrolled to Mouse wheel shortcuts. GPU and software tests exercise both
setting states, source-aware event routing, unchanged touchpad gestures and
content/track scrolling. No physical user input is injected.


Current native workspace captures use the reduced 8-DIP top inset. `search-tool-icons-47eaaa65fa19.png` comes from `search_tool_results_use_a_small_code_mark_and_powertoys_fallback`, with invented entries and PowerToys artwork intentionally unavailable. The pipette is the vector fallback; installed binary icon extraction is checked separately by `search_tool_launch_targets_load_transparent_icons_without_execution`. The refreshed native context-menu fixture invokes no command and changes no clipboard content. Both rendering paths were checked.
