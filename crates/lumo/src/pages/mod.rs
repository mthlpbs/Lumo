//! Expanded-island pages. Each page module exposes:
//!
//! ```ignore
//! pub fn height(st: &AppState) -> f32;                        // content height (DIPs)
//! pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect);    // draw + handle input
//! ```
//!
//! Pages mutate `AppState` directly for local state and push
//! [`crate::commands::Command`]s for side effects (media control, opening
//! files, saving, …).

pub mod ai;
pub mod ai_markdown;
pub mod ai_models;
pub mod clipboard;
pub mod focus;
pub mod media;
pub mod addons;
pub mod script;
pub mod settings;
pub mod shelf;
pub mod system;
pub mod tasks;

use crate::commands::Command;
use crate::gfx::{TextStyle, Weight};
use crate::icons::Icon;
use crate::state::AppState;
use crate::ui::{wid, wid_n, Input, Ui};
use lumo_core::geom::{lerp, palette, Color, Point, Rect};

/// Height of the compact toolbar band above the page content. The
/// header row itself is a fixed 36 DIPs ([`header_rect`]); the rest is the
/// 12 DIP gap between the tab pills and the first line of content.
pub const HEADER_H: f32 = 50.0;
/// Height of the header row (the 28 DIP tab pills sit centred in it).
const HEADER_ROW_H: f32 = 36.0;
/// Horizontal padding inside the expanded island, also used below the content.
pub const PAD: f32 = 18.0;

/// The header row inside the expanded island `body`. Fixed size, so changing
/// [`HEADER_H`] only moves the content, never the tabs.
pub fn header_rect(body: Rect) -> Rect {
    Rect::new(body.x + PAD - 6.0, body.y + 6.0, body.w - 2.0 * PAD + 12.0, HEADER_ROW_H)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageKind {
    Ai,
    Media,
    Focus,
    Shelf,
    Tasks,
    /// Only while Clipboard history is on (an add-on since 0.2.7, [`tabs`]).
    Clipboard,
    System,
    /// The teleprompter: a script read under the webcam.
    Script,
    /// Optional features, with switches to show or hide their pages.
    Addons,
    /// Opened through Settings in the tray menu; never in the section rail.
    Settings,
}

impl PageKind {
    pub const ALL: [PageKind; 10] = [
        PageKind::Media,
        PageKind::Focus,
        PageKind::Shelf,
        PageKind::Tasks,
        PageKind::Clipboard,
        PageKind::System,
        PageKind::Script,
        PageKind::Ai,
        PageKind::Addons,
        PageKind::Settings,
    ];

    pub fn key(self) -> &'static str {
        match self {
            PageKind::Ai => "ai",
            PageKind::Media => "media",
            PageKind::Focus => "focus",
            PageKind::Shelf => "shelf",
            PageKind::Tasks => "tasks",
            PageKind::Clipboard => "clipboard",
            PageKind::System => "system",
            PageKind::Script => "script",
            PageKind::Addons => "addons",
            PageKind::Settings => "settings",
        }
    }

    pub fn from_key(s: &str) -> PageKind {
        let s = s.trim();
        Self::ALL.into_iter().filter(|p| p.tabbed()).find(|p| p.key().eq_ignore_ascii_case(s)).unwrap_or(PageKind::Media)
    }

    pub fn title(self) -> &'static str {
        match self {
            PageKind::Ai => "Ask AI",
            PageKind::Media => "Music",
            PageKind::Focus => "Focus",
            PageKind::Shelf => "Shelf",
            PageKind::Tasks => "Tasks",
            PageKind::Clipboard => "Clipboard",
            PageKind::System => "System",
            PageKind::Script => "Script",
            PageKind::Addons => "Add-ons",
            PageKind::Settings => "Settings",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            PageKind::Ai => Icon::Sparkles,
            PageKind::Media => Icon::Music,
            PageKind::Focus => Icon::Timer,
            PageKind::Shelf => Icon::Tray,
            PageKind::Tasks => Icon::ListChecks,
            PageKind::Clipboard => Icon::Clipboard,
            PageKind::System => Icon::Gauge,
            PageKind::Script => Icon::Script,
            PageKind::Addons => Icon::Plus,
            PageKind::Settings => Icon::Gear,
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// Position in the tab row (for slide direction). Settings comes last:
    /// its gear sits right of the tabs.
    pub fn tab_index(self) -> usize {
        self.index()
    }

    /// Has a tab in the header's row (Settings has the gear instead).
    pub fn tabbed(self) -> bool {
        !matches!(self,PageKind::Settings|PageKind::Ai)
    }
}

pub fn tabs(st: &AppState) -> Vec<PageKind> {
    PageKind::ALL
        .into_iter()
        .filter(|p| match p {
            PageKind::Ai => false,
            PageKind::Script => st.script_on(),
            PageKind::Clipboard => st.settings.clipboard_history,
            p => p.tabbed(),
        })
        .collect()
}

/// The adjacent tab, stopping at either end so overscroll never jumps across.
pub fn step_tab(p: PageKind, st: &AppState, forward: bool) -> PageKind {
    let tabs = tabs(st);
    let n = tabs.len();
    let i = tabs.iter().position(|t| *t == p).unwrap_or(0);
    tabs[if forward { (i + 1).min(n - 1) } else { i.saturating_sub(1) }]
}

pub fn content_height(p: PageKind, st: &AppState) -> f32 {
    match p {
        PageKind::Ai => ai::height(st),
        PageKind::Media => media::height(st),
        PageKind::Focus => focus::height(st),
        PageKind::Shelf => shelf::height(st),
        PageKind::Tasks => tasks::height(st),
        PageKind::Clipboard => clipboard::height(st),
        PageKind::System => system::height(st),
        PageKind::Script => script::height(st),
        PageKind::Addons => addons::height(st),
        PageKind::Settings => settings::height(st),
    }
}

/// The next time (mono seconds) the expanded `page` changes on its own and
/// needs a frame without input, if any (e.g. an Undo pill expiring). Meant
/// for the app's wake timer; until it asks, such changes show on the next
/// repaint.
pub fn next_wake(page: PageKind, st: &AppState) -> Option<f64> {
    let wake = match page {
        PageKind::Ai => ai::next_wake(st),
        PageKind::Clipboard => clipboard::next_wake(st),
        PageKind::Tasks => tasks::next_wake(st),
        PageKind::Shelf => shelf::next_wake(st),
        PageKind::Script => script::next_wake(st),
        _ => None,
    };
    wake
}

/// A page switch in progress: the page we came from, 0..1 progress, and the
/// slide direction (+1 = moving to a tab on the right).
#[derive(Clone, Copy, Debug)]
pub struct PageTransition {
    pub from: PageKind,
    pub t: f32,
    pub dir: f32,
}

/// How far (DIPs) pages slide during a switch.
const SLIDE: f32 = 20.0;
/// The header's selection pill and labels settle by this much of the switch.

/// Draw the header and the active page inside the expanded island `body`.
/// During a page switch the old page fades out sliding away while the new
/// one fades in; the old page is drawn without input (it can't be clicked).
/// `body` is the island at its final size, so nothing reflows while it opens.
pub fn draw_expanded(ui: &mut Ui, st: &mut AppState, body: Rect, page: PageKind, transition: Option<PageTransition>) {
    if st.search.view.is_some() { crate::search::draw(ui, st, body); return; }
    if page==PageKind::Ai {ai::draw(ui,st,ai::workspace_area(body));return;}
    if crate::pill::expanded::draw(ui, st, body, page, transition) {
        return;
    }
    let tr = transition.filter(|tr| tr.t < 1.0 && tr.from != page);
    let header = header_rect(body);
    let tip = draw_header(ui, st, header, page, tr);
    draw_pages(ui, st, body, header, page, tr);
    // Hover labels sit on top of the page, outside the content clip.
    if let Some(tip) = tip {
        draw_tip(ui, header, &tip);
    }
}

/// The page content's rect inside the expanded island `body`: below the
/// header band, `PAD` in from the sides and the bottom.
pub(crate) fn content_area(body: Rect) -> Rect {
    Rect::new(body.x + PAD, body.y + HEADER_H, body.w - 2.0 * PAD, (body.h - HEADER_H - PAD).max(0.0))
}

/// The active page in [`content_area`], clipped to the island below the
/// `header` row. During the page switch `tr` (one in progress, as
/// [`draw_expanded`] filters it) the old page fades out sliding away while
/// the new one fades in. Every theme's frame draws its pages with this, so
/// a switch moves the same in all of them.
pub(crate) fn draw_pages(ui: &mut Ui, st: &mut AppState, body: Rect, header: Rect, page: PageKind, tr: Option<PageTransition>) {
    let area = content_area(body);
    // Content may reach a little above `area` (text boxes whose cap height,
    // not their line box, sits on the content top) but never into the tabs.
    let clip_top = header.bottom() + 2.0;
    ui.c.clip(Rect::new(body.x, clip_top, body.w, (body.bottom() - clip_top).max(0.0)));
    match tr {
        Some(tr) => {
            // Start revealing the destination immediately, with the header
            // and content settling together. Complementary fades keep the
            // combined opacity constant instead of dipping between pages.
            let in_a = ease_out_cubic(tr.t);
            let out_a = 1.0 - in_a;
            let snap_was = ui.c.set_text_snap(false);
            let base = ui.c.alpha();
            // Restore (never reset) the transform: the app may be scaling the
            // whole island content while it opens.
            let m0 = ui.c.save_transform();
            if out_a > 0.0 {
                // Outgoing page: ghost UI (no input, throwaway hit list).
                let mut ghost_input = Input::default();
                let mut ghost_hits = Vec::new();
                let now = ui.now;
                let mut ghost = Ui::new(&mut *ui.c, &mut ghost_input, &mut ghost_hits, now);
                ghost.c.set_alpha(base * out_a);
                ghost.c.translate(-tr.dir * SLIDE * (1.0 - out_a), 0.0);
                draw_page(&mut ghost, st, area, tr.from);
                ghost.c.restore_transform(m0);
            }
            // The new page always draws (even fully transparent) so its hit
            // regions and animation state stay continuous.
            ui.c.set_alpha(base * in_a);
            ui.c.translate(tr.dir * SLIDE * (1.0 - in_a), 0.0);
            draw_page(ui, st, area, page);
            ui.c.restore_transform(m0);
            ui.c.set_alpha(base);
            ui.c.set_text_snap(snap_was);
            ui.request_anim();
        }
        None => draw_page(ui, st, area, page),
    }
    ui.c.unclip();
}

pub(crate) fn draw_page(ui: &mut Ui, st: &mut AppState, area: Rect, page: PageKind) {
    match page {
        PageKind::Ai => ai::draw(ui, st, area),
        PageKind::Media => media::draw(ui, st, area),
        PageKind::Focus => focus::draw(ui, st, area),
        PageKind::Shelf => shelf::draw(ui, st, area),
        PageKind::Tasks => tasks::draw(ui, st, area),
        PageKind::Clipboard => clipboard::draw(ui, st, area),
        PageKind::System => system::draw(ui, st, area),
        PageKind::Script => script::draw(ui, st, area),
        PageKind::Addons => addons::draw(ui, st, area),
        PageKind::Settings => settings::draw(ui, st, area),
    }
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Round to the device-pixel grid.
pub(crate) fn snap(v: f32, scale: f32) -> f32 {
    (v * scale).round() / scale
}

// ---------------------------------------------------------------- header

pub(crate) const TIP_H: f32 = 22.0;
pub(crate) const TIP_PAD: f32 = 8.0;
const TIP_RADIUS: f32 = 8.0;
/// White 0.16 over the black island, made opaque so page text can't show through.
pub(crate) const TIP_FILL: Color = Color::rgba(0.16, 0.16, 0.16, 1.0);
pub const GEAR_ID: &str = "header.gear";

/// A hover label waiting to be painted above the page.
pub(crate) struct Tip {
    pub(crate) anchor: Rect,
    pub(crate) text: std::borrow::Cow<'static, str>,
    pub(crate) alpha: f32,
}

const TAB_H:f32=28.0;
const TAB_W:f32=30.0;
const TAB_GAP:f32=2.0;
const TAB_ICON:f32=16.0;
const TAB_LABEL_X:f32=28.0;
const TAB_LABEL_END:f32=12.0;
const TABS_END:f32=1.0;
fn tab_layout(ui: &mut Ui, x0: f32, tabs: &[PageKind], active: PageKind, label_st: &TextStyle, scale: f32) -> Vec<(f32, f32)> {
    let mut x = x0;
    tabs.iter()
        .map(|&p| {
            let w = if p == active { snap(TAB_LABEL_X + ui.c.measure(p.title(), label_st, 200.0).0 + TAB_LABEL_END, scale) } else { TAB_W };
            let slot = (snap(x, scale), w);
            x += w + TAB_GAP;
            slot
        })
        .collect()
}

pub(crate) fn draw_header(ui: &mut Ui, st: &mut AppState, r: Rect, page: PageKind, tr: Option<PageTransition>) -> Option<Tip> {
    let scale = ui.c.scale().max(0.5);
    let cy = r.center().y;
    let label_st = TextStyle::body().weight(Weight::Semibold);
    let from = tr.map_or(page, |t| t.from);
    let e = tr.map_or(1.0, |t| ease_out_cubic(t.t / TABS_END));
    let row = self::tabs(st);
    let lay_from = tab_layout(ui, r.x, &row, from, &label_st, scale);
    let lay_to = if from == page { lay_from.clone() } else { tab_layout(ui, r.x, &row, page, &label_st, scale) };
    let tabs: Vec<Rect> =
        (0..row.len()).map(|i| Rect::new(lerp(lay_from[i].0, lay_to[i].0, e), cy - TAB_H / 2.0, lerp(lay_from[i].1, lay_to[i].1, e), TAB_H)).collect();
    // A page outside the row counts as the first tab.
    let place = |p: PageKind| row.iter().position(|t| *t == p).unwrap_or(0);
    let (fi, pi) = (place(from), place(page));
    let mut hint:Option<Tip>=None;
    // How much the selection sits on the gear (Settings) rather than on a tab.
    let on_gear = |p: PageKind| if p.tabbed() { 0.0 } else { 1.0 };
    let gear_k = lerp(on_gear(from), on_gear(page), e);

    // Selection pill, gliding from the old tab to the new one. To or from
    // Settings it stays on its tab and fades while the gear takes it.
    let (a, b) = (if from.tabbed() { fi } else { pi }, if page.tabbed() { pi } else { fi });
    let pill = Rect::new(lerp(lay_from[a].0, lay_to[b].0, e), cy - TAB_H / 2.0, lerp(lay_from[a].1, lay_to[b].1, e), TAB_H);
    if gear_k < 1.0 {
        ui.c.with_alpha(1.0 - gear_k, |c| c.fill_rrect(pill, TAB_H / 2.0, Color::white(0.14)));
    }

    for (i, &p) in row.iter().enumerate() {
        let tr = tabs[i];
        let resp = ui.button(wid_n("tab", i as u64), tr);
        let active = p == page;
        if resp.hovered && !active {
            ui.c.fill_rrect(tr, TAB_H / 2.0, palette::SURFACE);
        }
        if resp.hovered {hint=Some(Tip{anchor:tr,text:format!("Open {}",p.title()).into(),alpha:1.0});}
        let k = if active { e } else if from.tabbed() && i == fi { 1.0 - e } else { 0.0 };
        let ic = if resp.hovered { palette::TEXT } else { Color::lerp(palette::TEXT_FAINT, palette::TEXT, k) };
        let ix = snap(tr.x + (TAB_W - TAB_ICON) / 2.0, scale);
        ui.c.icon(p.icon(), Rect::new(ix, snap(cy - TAB_ICON / 2.0, scale), TAB_ICON, TAB_ICON), ic);
        if resp.clicked && !active {
            st.cmd(Command::SetPage(p));
        }
    }
    if from.tabbed() && (fi != pi || !page.tabbed()) {
        draw_tab_label(ui, from, tabs[fi], &label_st, 1.0 - e);
    }
    if page.tabbed() {
        draw_tab_label(ui, page, tabs[pi], &label_st, e);
    }

    st.ui.tab_hover=None;
    let pin=Rect::new(r.right()-34.0,r.center().y-14.0,28.0,28.0);
    let response=ui.icon_button(wid("pin"),pin,if st.pinned{Icon::PinFill}else{Icon::Pin},if st.pinned{st.accent}else{palette::TEXT_FAINT},15.0);
    if response.hovered{hint=Some(Tip{anchor:pin,text:if st.pinned{"Unpin island"}else{"Pin island to keep it open"}.into(),alpha:1.0});}
    if response.clicked{st.cmd(Command::SetPinned(!st.pinned));}
    // Reserve this slot across transient media/lyric updates. Only availability
    // changes; song changes must never move the adjacent toolbar controls.
    let mini=page==PageKind::Media&&(st.settings.lyrics_popup_button||st.lyrics_popup_open);
    if mini{
        let button=Rect::new(pin.x-30.0,pin.y,28.0,28.0);
        let enabled=st.lyrics_popup_open||crate::lyrics_window::available(st);
        if media::popup_button(ui,button,st.lyrics_popup_open,enabled){st.cmd(if st.lyrics_popup_open{Command::CloseLyricsWindow}else{Command::OpenLyricsWindow});}
        if ui.hovered(button){let text=if st.lyrics_popup_open{"Return to island"}else if enabled{"Open mini player"}else if !st.settings.lyrics{"Enable synced lyrics to use mini player"}else{"Waiting for synced lyrics"};hint=Some(Tip{anchor:button,text:text.into(),alpha:1.0});}
    }
    if let Some(app)=st.foreground_app.as_ref().filter(|app|app.maximized_here){
        let keep=st.settings.keeps_app_visible(&app.executable);let button=Rect::new(pin.x-if mini{60.0}else{30.0},pin.y,28.0,28.0);
        let response=ui.icon_button(wid("header.keep-visible"),button,if keep{Icon::KeepVisible}else{Icon::App},if keep{st.accent}else{palette::TEXT_FAINT},16.0);
        if response.hovered{hint=Some(Tip{anchor:button,text:if keep{"Stop keeping visible over this app"}else{"Stay visible over this app"}.into(),alpha:1.0});}
        if response.clicked{st.settings.keep_app_visible(app.executable.clone(),!keep);st.cmd(Command::SaveSettings);}
    }
    if page==PageKind::Media {None}else{hint}
}

fn draw_tab_label(ui: &mut Ui, p: PageKind, tab: Rect, st: &TextStyle, alpha: f32) {
    if alpha <= 0.01 {
        return;
    }
    let lx = tab.x + TAB_LABEL_X;
    let avail = tab.right() - TAB_LABEL_END + 2.0 - lx;
    if avail <= 0.5 {
        return;
    }
    let lw = ui.c.measure(p.title(), st, 200.0).0;
    let cut = avail < lw + 1.0;
    if cut {
        ui.c.clip(Rect::new(lx, tab.y, avail, tab.h));
    }
    ui.c.with_alpha(alpha, |c| c.text(p.title(), Rect::new(lx, tab.y, lw + 2.0, tab.h), st));
    if cut {
        ui.c.unclip();
    }
}

fn draw_tip(ui: &mut Ui, header: Rect, tip: &Tip) {
    let scale = ui.c.scale().max(0.5);
    let st = TextStyle::footnote().center();
    let tw = ui.c.measure(&tip.text, &st, 1000.0).0;
    let w = snap((tw + 2.0 * TIP_PAD).ceil().min(header.w), scale);
    let x = (tip.anchor.center().x - w / 2.0).clamp(header.x, (header.right() - w).max(header.x));
    let r = Rect::new(snap(x, scale), snap(header.bottom() + 4.0, scale), w, TIP_H);
    ui.c.with_alpha(tip.alpha, |c| {
        c.fill_rrect(r, TIP_RADIUS, TIP_FILL);
        c.text(&tip.text, r, &st);
    });
}

/// Shared helper: a centred, dim placeholder message with an icon.
pub fn empty_state(ui: &mut Ui, area: Rect, icon: Icon, title: &str, hint: &str) {
    let c = area.center();
    ui.c.icon(icon, Rect::from_center(Point::new(c.x, c.y - 22.0), 26.0, 26.0), Color::white(0.35));
    ui.c.text(title, Rect::new(area.x, c.y - 4.0, area.w, 20.0), &TextStyle::headline().center());
    ui.c.text(hint, Rect::new(area.x, c.y + 16.0, area.w, 18.0), &TextStyle::footnote().color(palette::TEXT_DIM).center());
}

#[cfg(test)]
mod tests {
    use super::*;

    use lumo_core::settings::Settings;
    use lumo_core::store::Store;

    fn state() -> AppState {
        AppState::new(Settings::default(), Store::default())
    }

    #[test]
    fn settings_is_reached_from_the_tray_not_the_section_rail() {
        let st = state();
        assert!(!tabs(&st).contains(&PageKind::Settings));
        // Stepping through the tabs never lands on Settings.
        let mut p = PageKind::Media;
        for _ in 0..2 * PageKind::ALL.len() {
            p = step_tab(p, &st, true);
            assert!(p.tabbed(), "stepped onto {p:?}");
        }
    }

    #[test]
    fn wheel_navigation_stops_at_the_edges_and_visits_each_adjacent_tab() {
        let st = state();
        let row = tabs(&st);
        let first = row[0];
        let last = row[row.len() - 1];
        assert_eq!(step_tab(first, &st, false), first);
        assert_eq!(step_tab(last, &st, true), last);
        for pair in row.windows(2) {
            assert_eq!(step_tab(pair[0], &st, true), pair[1]);
            assert_eq!(step_tab(pair[1], &st, false), pair[0]);
        }
    }

    #[test]
    fn removed_pages_in_old_preferences_fall_back_to_music() {
        for old in ["health", "money", "water", "prayer", "settings"] {
            assert_eq!(PageKind::from_key(old), PageKind::Media);
            let settings = Settings { start_page: old.into(), ..Settings::default() };
            assert_eq!(AppState::new(settings, Store::default()).page, PageKind::Media);
            let settings = Settings { remember_page: true, ..Settings::default() };
            let store = Store { last_page: old.into(), ..Store::default() };
            assert_eq!(AppState::new(settings, store).page, PageKind::Media);
        }
    }

    #[test]
    fn retired_embedded_settings_is_not_restored_as_an_island_page() {
        assert_eq!(PageKind::from_key("settings"), PageKind::Media);
        assert!(PageKind::ALL.iter().all(|p| p.tab_index() <= PageKind::Settings.tab_index()));
    }


    #[test]
    fn clipboard_has_a_tab_only_while_its_add_on_is_on() {
        let mut st = state();
        assert!(!tabs(&st).contains(&PageKind::Clipboard), "off by default since 0.2.7");
        st.settings.set_addon(lumo_core::addons::CLIPBOARD_HISTORY.id, true);
        assert!(tabs(&st).contains(&PageKind::Clipboard));
    }

    #[test]
    fn ai_is_a_gesture_surface_and_never_a_tab_or_automatic_start_page() {
        let mut settings = Settings { start_page: "ai".into(), ..Settings::default() };
        let store = Store { last_page: "ai".into(), ..Store::default() };
        let st = AppState::new(settings.clone(), store.clone());
        assert!(!tabs(&st).contains(&PageKind::Ai));
        assert_eq!(st.page, PageKind::Media);
        settings.set_addon("ai", true);
        let st = AppState::new(settings, store);
        assert!(!tabs(&st).contains(&PageKind::Ai));
        assert_eq!(st.page, PageKind::Media);
    }

    #[test]
    fn script_has_a_tab_only_while_its_add_on_is_on() {
        let mut st = state();
        assert!(!tabs(&st).contains(&PageKind::Script), "off for everyone until switched on");
        st.addons.insert("script", lumo_core::addons::Status::Ready);
        assert!(tabs(&st).contains(&PageKind::Script));
        assert_eq!(tabs(&st).last(), Some(&PageKind::Addons));
    }

    #[test]
    fn an_island_last_on_the_script_page_opens_on_the_first_tab_once_it_is_off() {
        let store = Store { last_page: "script".into(), ..Store::default() };
        let mut settings = Settings { remember_page: true, ..Settings::default() };
        assert_eq!(AppState::new(settings.clone(), store.clone()).page, PageKind::ALL[0]);
        settings.set_addon("script", true);
        assert_eq!(AppState::new(settings, store).page, PageKind::Script);
    }

    #[test]
    fn settings_never_makes_the_window_taller() {
        // The window fits the tallest page; System is always 256 DIPs.
        let mut st = state();
        assert!(content_height(PageKind::Settings, &st) <= content_height(PageKind::System, &st));
        // Nor its new version's row, nor the Update sheet in its place.
        let rel = lumo_core::update::Release {
            version: "9.9.9".into(),
            date: String::new(),
            headline: "Something new".into(),
            notes: String::new(),
            windows: None,
        };
        st.update.offer = Some(rel);
        let row = content_height(PageKind::Settings, &st);
        st.update.sheet = true;
        let sheet = content_height(PageKind::Settings, &st);
        let window = content_height(PageKind::System, &st);
        assert!(row <= window && sheet <= window, "{row} {sheet}");
    }
}
