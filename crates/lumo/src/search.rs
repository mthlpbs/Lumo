//! Search inside the island. No Lumo commands, filter chips or footer chrome.
use crate::state::AppState;
use lumo_core::search::{Entry, Group, Target, WindowsSetting};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
pub struct Scan {
    pub request: u64,
    pub cancel: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct State {
    pub active: bool,
    pub context_menu: bool,
    pub web_url: Option<String>,
    pub web_error: String,
    pub web_ready: bool,
    pub web_visible: bool,
    pub navigation: crate::search_web::Navigation,
    pub address: String,
    pub address_dirty: bool,
    pub view: Option<View>,
    pub saved_view: Option<View>,
    pub icons: std::collections::HashMap<String, Option<crate::gfx::Image>>,
    pub icon_pending: Option<Scan>,
    pub apps: Vec<Entry>,
    pub loaded_at: Option<f64>,
    pub pending: Option<Scan>,
    pub request: u64,
    pub error: String,
}
pub struct Event {
    pub request: u64,
    pub result: Result<Vec<Entry>, String>,
}
impl State {
    pub fn cancel(&mut self) {
        if let Some(scan) = &self.icon_pending {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(scan) = &self.pending {
            scan.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn apply(&mut self, event: Event, now: f64, enabled: bool) {
        if self
            .pending
            .as_ref()
            .is_none_or(|p| p.request != event.request)
        {
            return;
        }
        let cancelled = self.pending.take().unwrap().cancel.load(Ordering::Relaxed);
        if cancelled || !enabled {
            return;
        }
        match event.result {
            Ok(apps) => {
                self.apps = apps;
                self.loaded_at = Some(now);
                self.error.clear();
            }
            Err(error) => {
                self.error = error;
                self.loaded_at = Some(now);
            }
        }
    }
}

pub struct IconEvent {
    pub request: u64,
    pub images: Vec<(String, Option<crate::gfx::Image>)>,
}

pub const WIDTH: f32 = 620.0;
pub const HEIGHT: f32 = 338.0;
const LIST_TOP: f32 = 72.0;
const ROW: f32 = 50.0;
pub struct View {
    pub query: String,
    entries: Vec<Entry>,
    results: Vec<usize>,
    pub selected: usize,
    scroll: usize,
    dirty: bool,
}
impl View {
    pub fn new(st: &AppState) -> Self {
        Self {
            query: String::new(),
            entries: entries(st),
            results: vec![],
            selected: 0,
            scroll: 0,
            dirty: true,
        }
    }
    pub fn navigate(&mut self, delta: isize) {
        self.selected = (self.selected as isize + delta)
            .clamp(0, self.results.len().saturating_sub(1) as isize)
            as usize;
    }
    fn pending(st: &AppState) -> bool {
        st.search
            .pending
            .as_ref()
            .is_some_and(|scan| !scan.cancel.load(Ordering::Relaxed))
    }
    fn web_action(&self, st: &AppState) -> bool {
        st.settings.search.web && !self.query.trim().is_empty() && !Self::pending(st)
    }
    pub fn height(&self, st: &AppState) -> f32 {
        if !self.results.is_empty() {
            LIST_TOP + ROW * self.results.len().min(5) as f32 + 16.0
        } else if self.web_action(st) {
            204.0
        } else {
            146.0
        }
    }
    pub fn selected(&self) -> Option<Entry> {
        self.results
            .get(self.selected)
            .map(|&i| self.entries[i].clone())
    }
    pub fn result_at(&mut self, id: crate::ui::WidgetId) -> Option<Entry> {
        let pos = self
            .results
            .iter()
            .enumerate()
            .position(|(p, _)| crate::ui::wid_n("search.result", p as u64) == id)?;
        self.selected = pos;
        self.selected()
    }
    fn rank(&mut self, st: &AppState) {
        if self.dirty {
            self.results = if self.query.trim().is_empty() {
                lumo_core::search::frequent(
                    &self.entries,
                    &st.settings.search,
                    &st.store.search_usage,
                )
            } else {
                lumo_core::search::ranked(&self.entries, &self.query, None, &st.settings.search)
            };
            self.selected = self.selected.min(self.results.len().saturating_sub(1));
            self.dirty = false;
        }
    }
}
pub fn refresh(st: &mut AppState) {
    let entries = entries(st);
    if let Some(view) = st.search.view.as_mut() {
        view.entries = entries;
        view.dirty = true;
    }
}
pub fn entries(st: &AppState) -> Vec<Entry> {
    let mut entries = Vec::new();
    for setting in WindowsSetting::ALL {
        entries.push(Entry::new(
            format!("windows-{setting:?}"),
            setting.title().into(),
            String::new(),
            &format!("{} settings", setting.keywords()),
            Group::Settings,
            Target::Windows(setting),
        ));
    }
    entries.extend(st.search.apps.iter().cloned());
    entries
}
pub fn draw(ui: &mut crate::ui::Ui, st: &mut AppState, body: lumo_core::geom::Rect) {
    use crate::{
        commands::Command,
        gfx::{TextStyle, Weight},
        icons::Icon,
        ui::{wid, wid_n},
    };
    use lumo_core::geom::{palette, Color, Rect};
    let Some(mut view) = st.search.view.take() else {
        return;
    };
    let rect = |x, y, w, h| Rect::new(body.x + x, body.y + y, w, h);
    if st.search.web_url.is_some() {
        st.search.view = Some(view);
        draw_browser(ui, st, body);
        return;
    }
    let input = ui.text_input_capsule(
        wid("search.query"),
        rect(24.0, 16.0, body.w - 168.0, 42.0),
        &mut view.query,
        "Search apps and Windows settings",
        128,
        st.accent,
    );
    if input.changed {
        view.dirty = true;
        view.selected = 0;
        view.scroll = 0;
        view.entries = entries(st);
    }
    let close = ui
        .icon_button(
            wid("search.close"),
            rect(body.w - 48.0, 18.0, 32.0, 36.0),
            Icon::CollapseUp,
            palette::TEXT_DIM,
            18.0,
        )
        .clicked;
    let control_tip = draw_search_controls(ui, st, body);
    let mut execute = None;
    let mut missing = vec![];
    {
        view.rank(st);
        let visible = ((body.h - LIST_TOP - 16.0) / ROW).floor().max(1.0) as usize;
        if ui.input.wheel != 0.0 {
            view.navigate(-ui.input.wheel.signum() as isize);
        }
        if view.selected < view.scroll {
            view.scroll = view.selected;
        }
        if view.selected >= view.scroll + visible {
            view.scroll = view.selected + 1 - visible;
        }
        let viewport = rect(
            18.0,
            LIST_TOP,
            body.w - 36.0,
            (body.h - LIST_TOP - 16.0).max(0.0),
        );
        ui.c.clip(viewport);
        let mut web_fallback = None;
        if view.results.is_empty() {
            let pending = View::pending(st);
            let caption = if pending {
                "Finding your apps…"
            } else if st.settings.search.apps && !st.search.error.is_empty() {
                "Apps are unavailable. Reopen Search to try again."
            } else if st.power.saver && st.settings.search.apps && st.search.loaded_at.is_none() {
                "App search is paused in battery saver."
            } else if !st.settings.search.apps && !st.settings.search.settings {
                "Local search is turned off."
            } else if view.query.trim().is_empty() {
                "Type to find an app or Windows setting"
            } else {
                "No apps or settings found"
            };
            ui.c.text(
                caption,
                rect(32.0, 78.0, body.w - 64.0, 24.0),
                &TextStyle::body().color(palette::TEXT_DIM),
            );
            if view.web_action(st) {
                if let Some(url) = st.settings.search.engine.url(&view.query) {
                    let target = Target::Web {
                        query: view.query.clone(),
                        url,
                    };
                    let r = rect(24.0, 112.0, body.w - 48.0, 68.0);
                    let response = ui.button(wid("search.web"), r);
                    ui.c.fill_rrect(
                        r,
                        18.0,
                        if response.hovered {
                            st.accent.mul_alpha(0.12)
                        } else {
                            Color::white(0.055)
                        },
                    );
                    ui.c.stroke_rrect(
                        r.inset(0.5, 0.5),
                        18.0,
                        st.accent
                            .mul_alpha(if response.hovered { 0.65 } else { 0.30 }),
                        1.0,
                    );
                    let mark = Rect::new(r.x + 14.0, r.y + 17.0, 34.0, 34.0);
                    ui.c.fill_rrect(mark, 17.0, st.accent.mul_alpha(0.11));
                    ui.c.icon(Icon::Globe, mark.inset(7.0, 7.0), st.accent);
                    ui.c.text(
                        &format!("Search {}", st.settings.search.engine.name()),
                        Rect::new(r.x + 62.0, r.y + 10.0, r.w - 112.0, 24.0),
                        &TextStyle::body().weight(Weight::Semibold),
                    );
                    ui.c.text(
                        view.query.trim(),
                        Rect::new(r.x + 62.0, r.y + 35.0, r.w - 112.0, 20.0),
                        &TextStyle::footnote().color(palette::TEXT_DIM),
                    );
                    ui.c.icon(
                        Icon::ArrowRight,
                        Rect::new(r.right() - 36.0, r.y + 25.0, 18.0, 18.0),
                        st.accent,
                    );
                    if response.clicked {
                        execute = Some(target.clone());
                    }
                    web_fallback = Some(target);
                }
            } else if !pending && !view.query.trim().is_empty() {
                ui.c.text(
                    "Try another app or setting name.",
                    rect(32.0, 107.0, body.w - 64.0, 23.0),
                    &TextStyle::footnote().color(palette::TEXT_FAINT),
                );
            }
        }
        for (row, position) in (view.scroll..view.results.len()).take(visible).enumerate() {
            let entry = &view.entries[view.results[position]];
            let r = rect(20.0, LIST_TOP + row as f32 * ROW, body.w - 40.0, ROW - 4.0);
            let response = ui.button(wid_n("search.result", position as u64), r);
            if position == view.selected || response.hovered {
                ui.c.fill_rrect(
                    r,
                    14.0,
                    Color::white(if position == view.selected {
                        0.105
                    } else {
                        0.055
                    }),
                );
            }
            let badge = Rect::new(r.x + 12.0, r.y + 7.0, 32.0, 32.0);
            if let Some(Some(image)) = st.search.icons.get(&entry.id) {
                ui.c.image(image, badge, 0.0);
            } else {
                let icon = match entry.target {
                    Target::App(_) => Icon::App,
                    Target::Windows(_) => Icon::Gear,
                    _ => Icon::Search,
                };
                ui.c.icon(icon, badge.inset(4.0, 4.0), palette::TEXT_DIM);
                if st.search.active
                    && st.search.icon_pending.is_none()
                    && !st.power.saver
                    && !st.search.icons.contains_key(&entry.id)
                    && matches!(entry.target, Target::App(_))
                {
                    missing.push((entry.id.clone(), entry.target.clone()));
                }
            }
            ui.c.text(
                &entry.title,
                Rect::new(r.x + 58.0, r.y, r.w - 74.0, r.h),
                &TextStyle::body().weight(Weight::Semibold),
            );
            if response.clicked && !input.changed {
                execute = Some(entry.target.clone());
            }
        }
        if view.results.len() > visible {
            let h = (viewport.h * visible as f32 / view.results.len() as f32).max(18.0);
            let y = viewport.y
                + (viewport.h - h) * view.scroll as f32 / (view.results.len() - visible) as f32;
            ui.c.fill_rrect(
                Rect::new(body.right() - 12.0, y, 2.0, h),
                1.0,
                Color::white(0.22),
            );
        }
        ui.c.unclip();
        if input.submitted {
            execute = view.selected().map(|e| e.target).or(web_fallback);
        }
    }
    if st.search.active {
        ui.input.focused = Some(wid("search.query"));
        if close || input.cancelled {
            st.cmd(Command::Collapse);
        } else if let Some(target) = execute {
            st.cmd(Command::ExecuteSearch(target));
        }
        if !missing.is_empty() {
            st.cmd(Command::SearchIcons(missing));
        }
    }
    draw_toolbar_tip(ui, body, control_tip);
    st.search.view = Some(view);
}

/// One geometry for drawing, native clipping and pointer input, at every DPI.
pub fn web_rect(body: lumo_core::geom::Rect) -> lumo_core::geom::Rect {
    lumo_core::geom::Rect::new(body.x + 16.0, body.y + 72.0, body.w - 32.0, body.h - 88.0)
}
pub fn address_target(value: &str, engine: lumo_core::search::Engine) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    if value.starts_with("https://") || value.starts_with("http://") {
        let authority = value.split_once("://")?.1.split('/').next()?;
        return (!authority.is_empty() && !authority.contains(char::is_whitespace))
            .then(|| value.to_owned());
    }
    // Reject active/non-web protocols. Bare hostnames are HTTPS; everything
    // else is an explicit search, never a request on each keystroke.
    if value.contains("://")
        || value.starts_with("javascript:")
        || value.starts_with("data:")
        || value.starts_with("file:")
    {
        return None;
    }
    if !value.contains(char::is_whitespace)
        && value
            .split('/')
            .next()
            .is_some_and(|h| h.contains('.') || h == "localhost")
    {
        Some(format!("https://{value}"))
    } else {
        engine.url(value)
    }
}
fn draw_browser(ui: &mut crate::ui::Ui, st: &mut AppState, body: lumo_core::geom::Rect) {
    use crate::{
        commands::Command,
        gfx::{TextStyle, Weight},
        icons::Icon,
        search_web::Action,
        ui::wid,
    };
    use lumo_core::geom::{palette, Color, Rect};
    let rect = |x, y, w, h| Rect::new(body.x + x, body.y + y, w, h);
    let mut tip = draw_search_controls(ui, st, body);
    if ui
        .icon_button(
            wid("search.back"),
            rect(16.0, 18.0, 36.0, 36.0),
            Icon::ChevronLeft,
            palette::TEXT,
            18.0,
        )
        .clicked
    {
        st.cmd(if st.search.navigation.back {
            Command::SearchWeb(Action::Back)
        } else {
            Command::CloseSearchWeb
        });
    }
    if ui.hovered(rect(16.0, 18.0, 36.0, 36.0)) {
        tip = Some(if st.search.navigation.back {
            "Back (Alt+Left)"
        } else {
            "Back to search"
        });
    }
    let forward = rect(52.0, 18.0, 36.0, 36.0);
    if st.search.navigation.forward {
        let resp = ui.icon_button(
            wid("search.forward"),
            forward,
            Icon::ChevronRight,
            palette::TEXT,
            18.0,
        );
        if resp.clicked {
            st.cmd(Command::SearchWeb(Action::Forward));
        }
        if resp.hovered {
            tip = Some("Forward (Alt+Right)");
        }
    } else {
        ui.c.icon(
            Icon::ChevronRight,
            forward.inset(9.0, 9.0),
            palette::TEXT_FAINT.mul_alpha(0.45),
        );
    }
    let bar = rect(100.0, 16.0, body.w - 380.0, 40.0);
    let input = ui.address_input(wid("search.query"), bar, &mut st.search.address, st.accent);
    if input.changed {
        st.search.address_dirty = true;
    }
    if input.submitted {
        if let Some(url) = address_target(&st.search.address, st.settings.search.engine) {
            st.search.address_dirty = false;
            st.cmd(Command::NavigateSearchWeb(url));
        }
    }
    if st.search.navigation.loading {
        // Event milestones only. No extra animation or frame clock while waiting.
        ui.c.fill_rrect(
            Rect::new(
                bar.x + 12.0,
                bar.bottom() - 3.0,
                (bar.w - 24.0) * st.search.navigation.progress.clamp(0.05, 1.0),
                2.0,
            ),
            1.0,
            st.accent,
        );
    }
    let reload = ui.icon_button(
        wid("search.reload"),
        Rect::new(bar.right() - 35.0, bar.y + 3.0, 32.0, 34.0),
        if st.search.navigation.loading {
            Icon::Close
        } else {
            Icon::Refresh
        },
        palette::TEXT_DIM,
        17.0,
    );
    if reload.clicked {
        st.cmd(Command::SearchWeb(if st.search.navigation.loading {
            Action::Stop
        } else {
            Action::Reload
        }));
    }
    if reload.hovered {
        tip = Some(if st.search.navigation.loading {
            "Stop loading"
        } else {
            "Reload (Ctrl+R)"
        });
    }
    let external = ui.pill_button(
        wid("search.external"),
        rect(body.w - 268.0, 19.0, 128.0, 34.0),
        "Open in browser",
        Some(Icon::ExternalLink),
        Color::white(0.07),
        palette::TEXT,
    );
    let current = st.search.navigation.url.clone();
    if external.clicked && (current.starts_with("https://") || current.starts_with("http://")) {
        st.cmd(Command::OpenUrl(current));
    }
    if external.hovered {
        tip = Some("Continue in your default browser");
    }
    let close = ui.icon_button(
        wid("search.close"),
        rect(body.w - 48.0, 18.0, 32.0, 36.0),
        Icon::CollapseUp,
        palette::TEXT_DIM,
        18.0,
    );
    if close.clicked || input.cancelled {
        st.cmd(Command::Collapse);
    }
    if close.hovered {
        tip = Some("Close search (Esc)");
    }
    let page = web_rect(body);
    ui.c.fill_rrect(page, 18.0, Color::rgba(0.075, 0.075, 0.08, 1.0));
    if st.search.web_visible {
        ui.c.web_aperture(page, 18.0);
    }
    ui.c.stroke_rrect(page.inset(0.5, 0.5), 18.0, Color::white(0.10), 1.0);
    if !st.search.web_error.is_empty() {
        let cx = page.center().x;
        let cy = page.center().y;
        ui.c.icon(
            Icon::Globe,
            Rect::new(cx - 16.0, cy - 78.0, 32.0, 32.0),
            palette::TEXT_DIM,
        );
        ui.c.text(
            "Unable to open this page",
            Rect::new(page.x + 24.0, cy - 30.0, page.w - 48.0, 30.0),
            &TextStyle::body().weight(Weight::Semibold).center(),
        );
        ui.c.text(
            &st.search.web_error,
            Rect::new(page.x + 40.0, cy + 2.0, page.w - 80.0, 50.0),
            &TextStyle::footnote()
                .color(palette::TEXT_DIM)
                .center()
                .wrap(),
        );
        if ui
            .pill_button(
                wid("search.retry"),
                Rect::new(cx - 52.0, cy + 64.0, 104.0, 34.0),
                "Try again",
                Some(Icon::Refresh),
                st.accent.mul_alpha(0.18),
                st.accent,
            )
            .clicked
        {
            st.cmd(Command::SearchWeb(Action::Reload));
        }
    } else if !st.search.web_ready {
        ui.c.text(
            "Opening page…",
            page.inset(24.0, 24.0),
            &TextStyle::body().color(palette::TEXT_DIM).center(),
        );
    }
    draw_toolbar_tip(ui, body, tip);
}
fn draw_search_controls(
    ui: &mut crate::ui::Ui,
    st: &mut AppState,
    body: lumo_core::geom::Rect,
) -> Option<&'static str> {
    use crate::{commands::Command, icons::Icon, ui::wid};
    use lumo_core::geom::{palette, Rect};
    let mut tip = None;
    for (id, x, on, icon, command, label) in [
        (
            "search.keep_open",
            body.w - 128.0,
            st.settings.search.keep_open,
            if st.settings.search.keep_open {
                Icon::PinFill
            } else {
                Icon::Pin
            },
            Command::ToggleSearchKeepOpen,
            if st.settings.search.keep_open {
                "Keep Search open: on — click to turn off"
            } else {
                "Keep Search open when switching apps"
            },
        ),
        (
            "search.resume",
            body.w - 88.0,
            st.settings.search.resume_page,
            Icon::ResumePage,
            Command::ToggleSearchResume,
            if st.settings.search.resume_page {
                "Resume page: on — click to turn off"
            } else {
                "Resume this page when Search reopens"
            },
        ),
    ] {
        let r = Rect::new(body.x + x, body.y + 18.0, 36.0, 36.0);
        if on {
            ui.c.fill_rrect(r, 11.0, st.accent.mul_alpha(0.16));
            ui.c.stroke_rrect(r.inset(0.5, 0.5), 11.0, st.accent.mul_alpha(0.4), 1.0);
        }
        let response = ui.icon_button(
            wid(id),
            r,
            icon,
            if on { st.accent } else { palette::TEXT_DIM },
            18.0,
        );
        if response.clicked {
            st.cmd(command);
        }
        if response.hovered {
            tip = Some(label);
        }
    }
    tip
}
fn draw_toolbar_tip(ui: &mut crate::ui::Ui, body: lumo_core::geom::Rect, tip: Option<&str>) {
    use crate::gfx::TextStyle;
    use lumo_core::geom::{Color, Rect};
    // The visual surface keeps toolbar hints above the native webpage.
    if let Some(tip) = tip {
        let style = TextStyle::footnote();
        let w = (ui.c.measure(tip, &style, body.w - 48.0).0 + 24.0).min(body.w - 48.0);
        let x = ui
            .input
            .mouse
            .map_or(body.center().x, |p| p.x)
            .clamp(body.x + 24.0 + w / 2.0, body.right() - 24.0 - w / 2.0)
            - w / 2.0;
        let r = Rect::new(x, body.y + 62.0, w, 28.0);
        ui.c.fill_rrect(r, 8.0, Color::rgba(0.16, 0.16, 0.17, 1.0));
        ui.c.stroke_rrect(r, 8.0, Color::white(0.12), 1.0);
        ui.c.text(tip, r, &style.center());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn address_navigation_is_explicit_and_web_only() {
        use lumo_core::search::Engine;
        assert_eq!(
            address_target(" example.com/path ", Engine::Google).as_deref(),
            Some("https://example.com/path")
        );
        assert_eq!(
            address_target("http://localhost:1234/one", Engine::Google).as_deref(),
            Some("http://localhost:1234/one")
        );
        assert_eq!(
            address_target("al nassr", Engine::Google),
            Engine::Google.url("al nassr")
        );
        for bad in [
            "",
            "javascript:alert(1)",
            "file:///C:/test",
            "data:text/html,test",
            "https://",
            "https://bad host",
            "abc\nxyz",
        ] {
            assert!(address_target(bad, Engine::Google).is_none(), "{bad}");
        }
    }
    #[test]
    fn only_apps_and_windows_settings_are_built() {
        let st = AppState::new(Default::default(), Default::default());
        let items = entries(&st);
        assert!(items.iter().all(|e| matches!(e.target, Target::Windows(_))));
        assert!(items.iter().any(|e| e.title == "Wi-Fi"));
    }
    #[test]
    fn retired_file_history_is_removed_and_empty_states_are_compact() {
        let mut store = lumo_core::store::Store::default();
        store.search_usage.insert(
            "file:C:\\Old\\item.txt".into(),
            lumo_core::search::Usage { count: 10, last: 1 },
        );
        store.search_usage.insert(
            "app-id".into(),
            lumo_core::search::Usage { count: 2, last: 1 },
        );
        let mut st = AppState::new(Default::default(), store);
        assert!(!st.store.search_usage.contains_key("file:C:\\Old\\item.txt"));
        assert!(st.store.search_usage.contains_key("app-id"));
        let mut view = View::new(&st);
        view.query = "missing-987".into();
        view.rank(&st);
        assert_eq!(view.height(&st), 146.0);
        st.settings.search.web = true;
        assert_eq!(view.height(&st), 204.0);
    }
    #[test]
    fn stale_or_cancelled_discovery_cannot_populate_cache() {
        let mut state = State::default();
        let cancel = Arc::new(AtomicBool::new(false));
        state.pending = Some(Scan {
            request: 2,
            cancel: cancel.clone(),
        });
        state.apply(
            Event {
                request: 1,
                result: Ok(vec![]),
            },
            0.0,
            true,
        );
        assert!(state.pending.is_some());
        cancel.store(true, Ordering::Relaxed);
        state.apply(
            Event {
                request: 2,
                result: Ok(vec![]),
            },
            0.0,
            true,
        );
        assert!(state.loaded_at.is_none());
    }
}
