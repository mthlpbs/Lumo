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
    pub detached: bool,
    pub navigation: crate::search_web::Navigation,
    pub address: String,
    pub address_dirty: bool,
    pub address_editing:bool,
    pub address_suggestions:Vec<String>,
    pub address_selected:Option<usize>,
    pub view: Option<View>,
    pub saved_view: Option<View>,
    pub icons: std::collections::HashMap<String, Option<crate::gfx::Image>>,
    pub icon_pending: Option<Scan>,
    pub apps: Vec<Entry>,
    pub integrations: Vec<Entry>,
    pub integrations_pending: Option<Scan>,
    pub integrations_request:u64,
    pub integrations_warning:String,
    pub action_pending:bool,
    pub notice:String,
    pub loaded_at: Option<f64>,
    pub pending: Option<Scan>,
    pub request: u64,
    pub error: String,
    pub suggestions: Suggestions,
    pub browser_transition: Option<f64>,
    pub transition_query: String,
}
#[derive(Default)]
pub struct Suggestions {
    pub address:bool,
    pub query: String,
    pub due: Option<f64>,
    pub serial: u64,
    pub pending: Option<(u64,Arc<crate::services::http::Cancel>)>,
}
impl Suggestions {
    pub fn cancel(&mut self) {
        if let Some((_,cancel))=self.pending.take(){cancel.cancel();}
        self.due=None;self.query.clear();self.serial=self.serial.wrapping_add(1);
    }
}
pub struct SuggestionsEvent {pub request:u64,pub query:String,pub items:Vec<String>}
pub struct Event {
    pub request: u64,
    pub result: Result<Vec<Entry>, String>,
}
impl State {
    pub fn embedded(&self) -> bool { self.web_url.is_some() && !self.detached }
    pub fn cancel(&mut self) {
        self.suggestions.cancel();
        self.address_suggestions.clear();self.address_selected=None;self.address_editing=false;
        if let Some(view)=self.view.as_mut(){view.suggestions.clear();}
        if let Some(scan) = &self.icon_pending {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(scan) = &self.pending {
            scan.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(scan)=&self.integrations_pending{scan.cancel.store(true,Ordering::Relaxed);}
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

pub const WIDTH: f32 = 560.0;
const LIST_TOP: f32 = 68.0;
const ROW: f32 = 40.0;
const MAX_ROWS: usize = 5;
pub const HEIGHT: f32 = LIST_TOP + ROW * MAX_ROWS as f32 + 16.0;
pub fn service_action_id(name:&str,start:bool)->crate::ui::WidgetId{
    crate::ui::wid_n(if start{"search.service.start"}else{"search.service.stop"},crate::ui::wid(name))
}
fn result_id(entry:&Entry,position:usize)->crate::ui::WidgetId{
    if matches!(entry.target,Target::AppCommand{..}){crate::ui::wid_n("search.code-result",crate::ui::wid(&entry.id))}
    else{crate::ui::wid_n("search.result",position as u64)}
}
pub struct View {
    pub query: String,
    entries: Vec<Entry>,
    results: Vec<usize>,
    pub selected: usize,
    scroll: usize,
    dirty: bool,
    pub suggestions: Vec<String>,
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
            suggestions: vec![],
        }
    }
    pub fn navigate(&mut self, delta: isize) {
        let count=if self.results.is_empty() && !self.suggestions.is_empty(){self.suggestions.len()+1}else{self.results.len()};
        self.selected = (self.selected as isize + delta)
            .clamp(0, count.saturating_sub(1) as isize)
            as usize;
    }
    fn pending(st: &AppState) -> bool {
        st.search.integrations_pending.as_ref().is_some_and(|p|!p.cancel.load(Ordering::Relaxed)) || st.search
            .pending
            .as_ref()
            .is_some_and(|scan| !scan.cancel.load(Ordering::Relaxed))
    }
    fn web_action(&self, st: &AppState) -> bool {
        st.settings.search.web && !self.query.trim().is_empty() && !Self::pending(st)
    }
    pub fn height(&self, st: &AppState) -> f32 {
        let height=if !self.results.is_empty() {
            LIST_TOP + ROW * self.results.len().min(MAX_ROWS) as f32 + 16.0
        } else if self.web_action(st) {
            if self.suggestions.is_empty(){204.0}else{232.0+ROW*self.suggestions.len() as f32}
        } else {
            146.0
        };height+if st.search.notice.is_empty(){0.0}else{32.0}
    }
    pub fn selected(&self) -> Option<Entry> {
        if self.results.is_empty() && self.selected>0 {
            let query=self.suggestions.get(self.selected-1)?;
            return Some(Entry::new("google-suggestion".into(),query.clone(),String::new(),"",Group::Settings,
                Target::Web{query:query.clone(),url:lumo_core::search::Engine::Google.url(query)?}));
        }
        self.results
            .get(self.selected)
            .map(|&i| self.entries[i].clone())
    }
    pub fn result_at(&mut self, id: crate::ui::WidgetId) -> Option<Entry> {
        let pos = self
            .results
            .iter()
            .enumerate()
            .position(|(p, i)| result_id(&self.entries[*i],p) == id)?;
        self.selected = pos;
        self.selected()
    }
    fn rank(&mut self, st: &AppState) {
        // Recheck cached Code windows only when Search is already rendering;
        // no lifecycle watcher or idle poll is needed.
        let old=self.entries.len();
        self.entries.retain(|e|crate::services::search_integrations::target_available(&e.target));
        if self.entries.len()!=old{self.dirty=true;}
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
            if !self.results.is_empty(){self.suggestions.clear();}
            self.dirty = false;
        }
    }
    pub fn can_suggest(&self,st:&AppState)->bool {
        st.search.active && !st.search.embedded() && st.settings.search.web && st.settings.search.google_suggestions
            && self.results.is_empty() && !Self::pending(st) && self.query.trim().chars().count()>=2 && direct_address(&self.query).is_none()
    }
}
pub fn apply_suggestions(st:&mut AppState,event:SuggestionsEvent){
    let Some((id,cancel))=st.search.suggestions.pending.as_ref()else{return;};
    if *id!=event.request || cancel.cancelled(){return;}
    st.search.suggestions.pending=None;
    if st.search.suggestions.address{
        if address_suggestion_query(st).as_deref()==Some(event.query.as_str()) && !st.power.saver{
            st.search.address_suggestions=event.items.into_iter().take(lumo_core::search::MAX_SUGGESTIONS).collect();st.search.address_selected=None;
        }else{st.search.address_suggestions.clear();st.search.address_selected=None;}
        return;
    }
    let Some(mut view)=st.search.view.take()else{return;};
    view.rank(st);
    if view.can_suggest(st) && view.query.trim()==event.query && !st.power.saver {
        view.suggestions=event.items.into_iter().take(lumo_core::search::MAX_SUGGESTIONS).collect();view.selected=0;view.scroll=0;
    }else{view.suggestions.clear();view.selected=0;view.scroll=0;}
    st.search.view=Some(view);
}
pub fn suggestion_query(st:&mut AppState)->Option<String>{
    if st.search.address_editing && (st.search.embedded()||st.search.detached){return address_suggestion_query(st);}
    let mut view=st.search.view.take()?;view.rank(st);
    let query=view.can_suggest(st).then(||view.query.trim().to_owned());st.search.view=Some(view);query
}
fn address_suggestion_query(st:&AppState)->Option<String>{
    let query=st.search.address.trim();
    (st.settings.search.web && st.settings.search.google_suggestions && st.search.address_editing && st.search.address_dirty
        && query.chars().count()>=2 && direct_address(query).is_none()).then(||query.to_owned())
}
pub fn refresh(st: &mut AppState) {
    let entries = entries(st);
    if let Some(view) = st.search.view.as_mut() {
        view.entries = entries.clone();
        view.dirty = true;
    }
    if let Some(view)=st.search.saved_view.as_mut(){view.entries=entries;view.dirty=true;}
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
    entries.extend(st.search.integrations.iter().filter(|e|e.enabled(&st.settings.search) && crate::services::search_integrations::target_available(&e.target)).cloned());

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
    let selected_code_closed=view.results.get(view.selected).and_then(|i|view.entries.get(*i))
        .is_some_and(|e|!crate::services::search_integrations::target_available(&e.target));
    let rect = |x, y, w, h| Rect::new(body.x + x, body.y + y, w, h);
    if st.search.embedded() {
        st.search.view = Some(view);
        let alpha=ui.c.alpha();
        if let Some(start)=st.search.browser_transition {
            let t=((st.now-start)/0.22).clamp(0.0,1.0) as f32;let eased=t*t*(3.0-2.0*t);
            if t<1.0 && !st.power.saver {
                // A finite crossfade shares the island's height spring. The
                // native webpage stays hidden until that outline has settled.
                let r=rect(24.0,16.0,body.w-48.0,40.0);
                ui.c.set_alpha(alpha*(1.0-eased));ui.c.fill_rrect(r,20.0,Color::white(0.10));
                ui.c.icon(Icon::Search,Rect::new(r.x+15.0,r.y+12.0,16.0,16.0),palette::TEXT_DIM);
                ui.c.text(&st.search.transition_query,Rect::new(r.x+43.0,r.y,r.w-58.0,r.h),&TextStyle::body());
                ui.c.set_alpha(alpha*eased);ui.request_anim();
            }else{st.search.browser_transition=None;}
        }
        draw_browser(ui, st, body);
        ui.c.set_alpha(alpha);
        return;
    }
    let input = ui.text_input_capsule(
        wid("search.query"),
        rect(24.0, 16.0, body.w - 48.0, 40.0),
        &mut view.query,
        "Search apps, settings and actions",
        128,
        st.accent,
    );
    if input.changed {
        if !st.search.action_pending{st.search.notice.clear();}
        view.dirty = true;
        view.selected = 0;
        view.scroll = 0;
        view.entries = entries(st);
        view.suggestions.clear();
    }
    let control_tip=None;
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
            (body.h - LIST_TOP - 16.0-if st.search.notice.is_empty(){0.0}else{32.0}).max(0.0),
        );
        ui.c.clip(viewport);
        let mut web_fallback = None;
        if view.results.is_empty() {
            let pending = View::pending(st);
            let caption = if pending {
                "Loading search sources…"
            } else if st.settings.search.apps && !st.search.error.is_empty() {
                "Apps are unavailable. Reopen Search to try again."
            } else if st.power.saver && st.settings.search.apps && st.search.loaded_at.is_none() {
                "App search is paused in battery saver."
            } else if !st.search.integrations_warning.is_empty() {
                &st.search.integrations_warning
            } else if !st.settings.search.apps && !st.settings.search.settings && !crate::services::search_integrations::enabled(&st.settings.search) {
                "Local search is turned off."
            } else if view.query.trim().is_empty() {
                "Type to find an app or Windows setting"
            } else {
                "No matching results"
            };
            ui.c.text(
                caption,
                rect(32.0, 78.0, body.w - 64.0, 24.0),
                &TextStyle::body().color(palette::TEXT_DIM),
            );
            if view.web_action(st) {
                if let Some(entry) = web_entry(&view.query,st.settings.search.engine) {
                    let target = entry.target;
                    let r = rect(24.0, 112.0, body.w - 48.0, 68.0);
                    let response = ui.button(wid("search.web"), r);
                    ui.c.fill_rrect(
                        r,
                        18.0,
                        if response.hovered || (!view.suggestions.is_empty() && view.selected==0) {
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
                        &entry.title,
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
                if !view.suggestions.is_empty() {
                    ui.c.text("Google suggestions",rect(32.0,190.0,body.w-64.0,22.0),&TextStyle::footnote().color(palette::TEXT_DIM));
                    let count=((body.h-232.0)/ROW).floor().max(0.0) as usize;
                    if view.selected==0{view.scroll=0;}else if count>0 {
                        let selected=view.selected-1;
                        if selected<view.scroll{view.scroll=selected;}
                        if selected>=view.scroll+count{view.scroll=selected+1-count;}
                    }
                    for (row,i) in (view.scroll..view.suggestions.len()).take(count).enumerate(){
                        let r=rect(24.0,216.0+ROW*row as f32,body.w-48.0,38.0);
                        let response=ui.button(wid_n("search.suggestion",i as u64),r);let selected=view.selected==i+1;
                        if selected || response.hovered{ui.c.fill_rrect(r,10.0,Color::white(if selected{0.08}else{0.04}));}
                        if selected{ui.c.fill_rrect(Rect::new(r.x+4.0,r.center().y-8.0,2.0,16.0),1.0,st.accent);}
                        ui.c.icon(Icon::Search,Rect::new(r.x+14.0,r.center().y-9.0,18.0,18.0),palette::TEXT_DIM);
                        let text=&view.suggestions[i];
                        ui.c.text(text,Rect::new(r.x+44.0,r.y,r.w-58.0,r.h),&TextStyle::body());
                        if response.clicked && !input.changed {
                            view.selected=i+1;
                            execute=Some(Target::Web{query:text.clone(),url:lumo_core::search::Engine::Google.url(text).unwrap()});
                        }
                    }
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
            let r = rect(20.0, LIST_TOP + row as f32 * ROW, body.w - 40.0, ROW - 2.0);
            let response = ui.button(result_id(entry,position), r);
            let selected = position == view.selected;
            if selected || response.hovered || response.pressed {
                ui.c.fill_rrect(
                    r,
                    10.0,
                    Color::white(if response.pressed {
                        0.12
                    } else if selected && response.hovered {
                        0.09
                    } else if selected {
                        0.065
                    } else {
                        0.035
                    }),
                );
            }
            if selected {
                ui.c.fill_rrect(Rect::new(r.x + 4.0, r.center().y - 8.0, 2.0, 16.0), 1.0, st.accent);
            }
            let badge = Rect::new(r.x + 14.0, r.center().y - 12.0, 24.0, 24.0);
            if let Some(Some(image)) = st.search.icons.get(&entry.id) {
                ui.c.image(image, badge, 0.0);
            } else {
                let icon = match entry.target {
                    Target::App(_) => Icon::App,
                    Target::Windows(_) => Icon::Gear,
                    Target::Service(_) => Icon::Gear,
                    Target::PowerToy{..} => Icon::App,
                    Target::Launch{..}|Target::AppCommand{..}=>Icon::App,
                    _ => Icon::Search,
                };
                ui.c.icon(icon, badge.inset(2.0, 2.0), palette::TEXT_DIM);
                if st.search.active
                    && st.search.icon_pending.is_none()
                    && !st.power.saver
                    && !st.search.icons.contains_key(&entry.id)
                    && matches!(entry.target, Target::App(_))
                {
                    missing.push((entry.id.clone(), entry.target.clone()));
                }
            }
            let is_service=matches!(entry.target,Target::Service(_));
            let title_width=r.w-if is_service{138.0}else{66.0};
            let detailed=is_service || matches!(entry.group,Group::PowerToys|Group::VSCode);
            ui.c.text(
                &entry.title,
                Rect::new(r.x + 50.0, r.y, title_width,if detailed{22.0}else{r.h}),
                &TextStyle::body().weight(Weight::Medium),
            );
            if detailed{ui.c.text(&entry.subtitle,Rect::new(r.x+50.0,r.y+21.0,title_width,16.0),&TextStyle::footnote().color(palette::TEXT_DIM));}
            let mut service_clicked=false;
            if let Target::Service(service)=&entry.target{
                let action=if service.can_start(){Some((true,"Start"))}else if service.can_stop(){Some((false,"Stop"))}else{None};
                if let Some((start,label))=action{let at=Rect::new(r.right()-76.0,r.y+5.0,68.0,28.0);
                    // Both identity and operation stay stable across async
                    // ranking/state refreshes between mouse-down and mouse-up.
                    if !st.search.action_pending && ui.pill(service_action_id(&service.name,start),at,label,None,crate::ui::PillStyle::Utility).clicked{st.cmd(Command::SearchService{name:service.name.clone(),start});service_clicked=true;}
                }
            }
            if response.clicked && !input.changed && !service_clicked {
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
        if input.submitted && (!selected_code_closed || input.changed) {
            execute = if ui.input.ctrl && st.settings.search.web{Some(browser_target(&view.query,st.settings.search.engine))}else{view.selected().map(|e| e.target).or(web_fallback)};
        }
    }
    if st.search.active {
        ui.input.focused = Some(wid("search.query"));
        if input.cancelled {
            st.cmd(Command::Collapse);
        } else if let Some(target) = execute {
            st.cmd(Command::ExecuteSearch(target));
        }
        if !missing.is_empty() {
            st.cmd(Command::SearchIcons(missing));
        }
    }
    draw_toolbar_tip(ui, body, control_tip);
    if !st.search.notice.is_empty(){ui.c.text(&st.search.notice,rect(24.0,body.h-30.0,body.w-48.0,24.0),&TextStyle::footnote().color(palette::TEXT_DIM));}
    st.search.view = Some(view);
}

/// The browser button is an explicit navigation, even when there is no query.
/// Local search never starts the web runtime or contacts an engine on its own.
fn browser_target(query: &str, engine: lumo_core::search::Engine) -> Target {
    use lumo_core::search::Engine;
    let url = address_target(query,engine).unwrap_or_else(|| {
        match engine {
            Engine::Bing => "https://www.bing.com/",
            Engine::Google => "https://www.google.com/",
            Engine::DuckDuckGo => "https://duckduckgo.com/",
            Engine::Brave => "https://search.brave.com/",
        }
        .into()
    });
    Target::Web {
        query: query.trim().into(),
        url,
    }
}

/// One geometry for drawing, native clipping and pointer input, at every DPI.
pub fn web_rect(body: lumo_core::geom::Rect) -> lumo_core::geom::Rect {
    let top=76.0;
    lumo_core::geom::Rect::new(body.x + 16.0, body.y + top, body.w - 32.0, (body.h - top - 16.0).max(1.0))
}
pub struct BrowserToolbar {
    pub bar:lumo_core::geom::Rect,
    pub address:lumo_core::geom::Rect,
    pub back:lumo_core::geom::Rect,
    pub forward:lumo_core::geom::Rect,
    pub reload:lumo_core::geom::Rect,
    pub external:lumo_core::geom::Rect,
    pub mini:lumo_core::geom::Rect,
    pub pin:lumo_core::geom::Rect,
    pub resume:lumo_core::geom::Rect,
    pub more:Option<lumo_core::geom::Rect>,
    pub close:Option<lumo_core::geom::Rect>,
}
pub fn browser_toolbar(body:lumo_core::geom::Rect,detached:bool)->BrowserToolbar {
    use lumo_core::geom::Rect;
    let bar=Rect::new(body.x+16.0,body.y+16.0,body.w-32.0,44.0);
    let at=|i:f32|Rect::new(bar.right()-32.0-i*28.0,bar.y+8.0,28.0,28.0);
    let full=body.w>=440.0;let end=if detached{1.0}else{0.0};let zero=Rect::new(bar.x,bar.y,0.0,0.0);
    let right=if full{(5.0+end)*28.0}else{(3.0+end)*28.0};
    let left=if full{56.0}else{28.0};
    BrowserToolbar{bar,address:Rect::new(bar.x+4.0+left,bar.y,bar.w-8.0-left-right,bar.h),
        back:Rect::new(bar.x+4.0,bar.y+8.0,28.0,28.0),
        forward:if full{Rect::new(bar.x+32.0,bar.y+8.0,28.0,28.0)}else{zero},
        reload:at(end),mini:at(end+1.0),external:if full{at(end+2.0)}else{zero},
        pin:if full{at(end+3.0)}else{zero},resume:if full{at(end+4.0)}else{zero},
        more:(!full).then(||at(end+2.0)),close:detached.then(||at(0.0))}
}
pub fn address_dropdown(st:&AppState,body:lumo_core::geom::Rect)->Option<lumo_core::geom::Rect>{
    if !st.search.address_editing || st.search.address_suggestions.is_empty(){return None;}
    let bar=browser_toolbar(body,st.search.detached).bar;
    let count=st.search.address_suggestions.len().min(((body.bottom()-16.0-bar.bottom()-18.0)/36.0).floor().max(0.0) as usize);
    (count>0).then(||lumo_core::geom::Rect::new(bar.x,bar.bottom()+6.0,bar.w,12.0+36.0*count as f32))
}
pub fn address_cover(st:&AppState,body:lumo_core::geom::Rect,scale:f32)->i32{
    address_dropdown(st,body).map_or(0,|r|((r.bottom()+2.0-web_rect(body).y).max(0.0)*scale).ceil() as i32)
}
pub fn snapped_web_rect(body: lumo_core::geom::Rect, scale: f32) -> lumo_core::geom::Rect {
    let r = web_rect(body);
    let x = (r.x * scale).round() / scale;
    let y = (r.y * scale).round() / scale;
    lumo_core::geom::Rect::new(x, y, (r.right() * scale).round() / scale - x, (r.bottom() * scale).round() / scale - y)
}
/// Classify website input once for the launcher, context menu and omnibox.
/// No networking or provider lookup is needed to recognize a bare domain.
pub fn direct_address(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(|c|c.is_control() || c.is_whitespace()) {
        return None;
    }
    let explicit=value.get(..8).is_some_and(|s|s.eq_ignore_ascii_case("https://"))
        || value.get(..7).is_some_and(|s|s.eq_ignore_ascii_case("http://"));
    let rest=if explicit{value.split_once("://")?.1}else{value};
    let authority=rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('\\'){return None;}
    // Explicit HTTP URLs can also contain IPv6 hosts or credentials; leave
    // their full URL parsing to WebView2 instead of treating them as queries.
    if explicit {
        let (scheme,_)=value.split_once("://")?;
        return Some(format!("{}://{rest}",scheme.to_ascii_lowercase()));
    }
    if authority.contains('@'){return None;}
    let host=if let Some((host,port))=authority.rsplit_once(':') {
        port.parse::<u16>().ok()?;host
    }else{authority};
    let valid_label=|label:&str|!label.is_empty() && !label.starts_with('-') && !label.ends_with('-')
        && label.chars().all(|c|c.is_alphanumeric() || c=='-');
    let valid_host=host.split('.').all(valid_label);
    let is_domain=host.split_once('.').is_some_and(|(_,tail)|!tail.is_empty());
    if !valid_host || (!is_domain && !host.eq_ignore_ascii_case("localhost")){return None;}
    Some(format!("https://{value}"))
}
pub fn address_target(value: &str, engine: lumo_core::search::Engine) -> Option<String> {
    if let Some(url)=direct_address(value){return Some(url);}
    let value=value.trim();let lower=value.to_ascii_lowercase();
    if value.is_empty() || value.chars().any(char::is_control) || value.contains("://")
        || lower.starts_with("javascript:") || lower.starts_with("data:") || lower.starts_with("file:")
    {
        return None;
    }
    engine.url(value)
}
pub fn web_entry(query:&str,engine:lumo_core::search::Engine)->Option<Entry>{
    let title=if direct_address(query).is_some(){"Visit".to_owned()}else{format!("Search {}",engine.name())};
    Some(Entry::new("web".into(),title,String::new(),"",Group::Settings,
        Target::Web{query:query.trim().into(),url:address_target(query,engine)?}))
}
pub(crate) fn draw_browser(ui: &mut crate::ui::Ui, st: &mut AppState, body: lumo_core::geom::Rect) {
    use crate::{
        commands::Command,
        gfx::{TextStyle, Weight},
        icons::Icon,
        search_web::Action,
        ui::wid,
    };
    use lumo_core::geom::{palette, Color, Rect};
    let toolbar=browser_toolbar(body,st.search.detached);
    let editing=ui.input.focused==Some(wid("search.query"));st.search.address_editing=editing;
    let progress=st.search.navigation.loading.then_some(st.search.navigation.progress.clamp(0.05,1.0));
    let mut tip=None;
    if editing {
        for key in &ui.input.keys{match key{
            crate::ui::KeyInput::Down if !st.search.address_suggestions.is_empty()=>st.search.address_selected=Some(st.search.address_selected.map_or(0,|i|(i+1).min(st.search.address_suggestions.len()-1))),
            crate::ui::KeyInput::Up=>st.search.address_selected=st.search.address_selected.and_then(|i|i.checked_sub(1)),_=>{}
        }}
        let input=ui.address_input(wid("search.query"),toolbar.bar,&mut st.search.address,st.accent,progress);
        if input.changed{st.search.address_dirty=true;st.search.address_suggestions.clear();st.search.address_selected=None;}
        if input.submitted{
            let target=st.search.address_selected.and_then(|i|st.search.address_suggestions.get(i)).and_then(|q|st.settings.search.engine.url(q))
                .or_else(||address_target(&st.search.address,st.settings.search.engine));
            if let Some(url)=target{st.search.address_dirty=false;st.search.address_editing=false;st.search.address_suggestions.clear();ui.input.focused=None;st.cmd(Command::NavigateSearchWeb(url));}
        }
        if input.cancelled{ui.input.focused=None;st.search.address_editing=false;st.search.address_dirty=false;st.search.address=st.search.navigation.url.clone();st.search.address_suggestions.clear();st.search.suggestions.cancel();}
    }else{
        ui.c.fill_rrect(toolbar.bar,12.0,Color::white(0.055));
        if let Some(progress)=progress{
            ui.c.push_outline_mask(&crate::pill::shape::segments(toolbar.bar,12.0));
            ui.c.fill_rect(toolbar.bar,st.accent.mul_alpha(0.035));
            let width=toolbar.bar.w*progress;ui.c.fill_rect(Rect::new(toolbar.bar.x,toolbar.bar.y,width,toolbar.bar.h),st.accent.mul_alpha(0.14));
            ui.c.fill_rect(Rect::new(toolbar.bar.x,toolbar.bar.bottom()-2.0,width,2.0),st.accent);ui.c.pop_shape_mask();
        }
        ui.c.stroke_rrect(toolbar.bar.inset(0.5,0.5),12.0,Color::white(0.09),1.0);
        let response=ui.button(wid("search.query"),toolbar.address);
        let title=if st.search.navigation.title.trim().is_empty(){crate::search_web::display_address(&st.search.navigation.url)}else{st.search.navigation.title.clone()};
        ui.c.text(&title,toolbar.address.inset(8.0,0.0),&TextStyle::body().center());
        if response.clicked{st.cmd(Command::SearchWeb(Action::Address));}
        tip=draw_browser_controls(ui,st,&toolbar);
        for (id,r,icon,enabled,action,label) in [
            ("search.back",toolbar.back,Icon::ChevronLeft,true,if st.search.navigation.back{Command::SearchWeb(Action::Back)}else{Command::CloseSearchWeb},"Back (Alt+Left)"),
            ("search.forward",toolbar.forward,Icon::ChevronRight,st.search.navigation.forward,Command::SearchWeb(Action::Forward),"Forward (Alt+Right)"),
            ("search.reload",toolbar.reload,if st.search.navigation.loading{Icon::Close}else{Icon::Refresh},true,Command::SearchWeb(if st.search.navigation.loading{Action::Stop}else{Action::Reload}),if st.search.navigation.loading{"Stop loading"}else{"Reload (Ctrl+R)"}),
            ("search.external",toolbar.external,Icon::ExternalLink,true,Command::OpenUrl(st.search.navigation.url.clone()),"Continue in default browser"),
            ("search.mini",toolbar.mini,if st.search.detached{Icon::ExitMiniPlayer}else{Icon::MiniPlayer},true,Command::ToggleMiniBrowser,if st.search.detached{"Return to island"}else{"Open mini browser"}),
        ]{
            if r.w<=0.0{continue;}
            if enabled{let response=ui.icon_button(wid(id),r,icon,palette::TEXT_DIM,17.0);if response.clicked{st.cmd(action);}if response.hovered{tip=Some(label);}}
            else{ui.c.icon(icon,r.inset(6.0,6.0),palette::TEXT_FAINT.mul_alpha(0.45));}
        }
        if let Some(r)=toolbar.more{let response=ui.button(wid("search.more"),r);if response.hovered{ui.c.fill_rrect(r,8.0,Color::white(0.08));tip=Some("More browser actions");}ui.c.text("⋯",r,&TextStyle::body().center());if response.clicked{st.cmd(Command::SearchBrowserMenu(r.center()));}}
        if let Some(r)=toolbar.close{let response=ui.icon_button(wid("search.close"),r,Icon::Close,palette::TEXT_DIM,17.0);if response.clicked{st.cmd(Command::CloseMiniBrowser);}if response.hovered{tip=Some("Close mini browser");}}
    }
    let scale = ui.c.scale();
    let page = snapped_web_rect(body, scale);
    ui.c.fill_rrect(page, 18.0, Color::rgba(0.075, 0.075, 0.08, 1.0));
    if st.search.web_visible {
        // Two physical pixels cover both GDI's hard region rounding and the
        // antialiased fringe of the cutout, including partially clear corners.
        let seal = 2.0 / scale;
        ui.c.web_aperture(page.inset(seal, seal), 18.0 - seal);
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
    } else if !st.search.web_visible {
        ui.c.text(
            "Opening page…",
            page.inset(24.0, 24.0),
            &TextStyle::body().color(palette::TEXT_DIM).center(),
        );
    }
    if let Some(drop)=address_dropdown(st,body){
        let cover=address_cover(st,body,scale) as f32/scale;
        ui.c.push_outline_mask(&crate::pill::shape::segments(page,18.0));ui.c.fill_rect(Rect::new(page.x,page.y,page.w,cover),Color::rgba(0.075,0.075,0.08,1.0));ui.c.pop_shape_mask();
        ui.c.fill_rrect(drop,12.0,Color::rgba(0.10,0.10,0.11,1.0));ui.c.stroke_rrect(drop.inset(0.5,0.5),12.0,Color::white(0.10),1.0);
        let count=((drop.h-12.0)/36.0) as usize;let start=st.search.address_selected.map_or(0,|i|(i+1).saturating_sub(count));
        for (row,i) in (start..st.search.address_suggestions.len()).take(count).enumerate(){let r=Rect::new(drop.x+6.0,drop.y+6.0+row as f32*36.0,drop.w-12.0,36.0);let response=ui.button(crate::ui::wid_n("search.address-suggestion",i as u64),r);
            if response.hovered || st.search.address_selected==Some(i){ui.c.fill_rrect(r,8.0,Color::white(0.08));}
            ui.c.icon(Icon::Search,Rect::new(r.x+10.0,r.y+10.0,16.0,16.0),palette::TEXT_DIM);ui.c.text(&st.search.address_suggestions[i],Rect::new(r.x+36.0,r.y,r.w-46.0,r.h),&TextStyle::body());
            if response.clicked{if let Some(url)=st.settings.search.engine.url(&st.search.address_suggestions[i]){st.cmd(Command::NavigateSearchWeb(url));ui.input.focused=None;st.search.address_editing=false;st.search.address_dirty=false;st.search.address_suggestions.clear();break;}}
        }
    }
    draw_toolbar_tip_at(ui, body, tip,4.0);
}
fn draw_browser_controls(
    ui: &mut crate::ui::Ui,
    st: &mut AppState,
    toolbar: &BrowserToolbar,
) -> Option<&'static str> {
    use crate::{commands::Command, icons::Icon, ui::wid};
    use lumo_core::geom::palette;
    let mut tip = None;
    for (id, r, on, icon, command, label) in [
        (
            "search.keep_open",
            toolbar.pin,
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
            toolbar.resume,
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
        if r.w<=0.0{continue;}
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
    draw_toolbar_tip_at(ui,body,tip,62.0);
}
fn draw_toolbar_tip_at(ui: &mut crate::ui::Ui, body: lumo_core::geom::Rect, tip: Option<&str>, top:f32) {
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
        let r = Rect::new(x, body.y + top, w, 28.0);
        ui.c.fill_rrect(r, 8.0, Color::rgba(0.16, 0.16, 0.17, 1.0));
        ui.c.stroke_rrect(r, 8.0, Color::white(0.12), 1.0);
        ui.c.text(tip, r, &style.center());
    }
}

#[cfg(test)]
mod tests {
    #[test]#[ignore="uses an isolated, hidden native window; no Code app is launched"]
    fn closed_code_windows_disappear_from_cached_results(){unsafe{
        use windows::{core::w,Win32::{System::Threading::GetCurrentProcessId,UI::WindowsAndMessaging::*}};
        let hwnd=CreateWindowExW(WINDOW_EX_STYLE(0),w!("STATIC"),w!("Lumo Code lifecycle fixture"),WS_OVERLAPPEDWINDOW,0,0,100,100,None,None,None,None).unwrap();
        let target=Target::AppCommand{window:hwnd.0 as usize as u64,pid:GetCurrentProcessId(),keys:vec![0x11,0xBC]};
        let mut st=AppState::new(Default::default(),Default::default());st.settings.search.apps=false;st.settings.search.settings=false;
        st.search.integrations=vec![Entry::new("code-fixture".into(),"VS Code: Settings".into(),String::new(),"",Group::VSCode,target.clone())];
        let mut view=View::new(&st);view.query="VS Code".into();view.rank(&st);assert_eq!(view.results.len(),1);
        let old_id=result_id(&view.entries[view.results[0]],0);
        DestroyWindow(hwnd).unwrap();
        assert!(!crate::services::search_integrations::target_available(&target));
        view.rank(&st);assert!(view.results.is_empty());assert!(entries(&st).iter().all(|e|e.group!=Group::VSCode));
        assert!(view.result_at(old_id).is_none(),"a stale command click must not select another result");
    }}
    #[test]
    fn service_source_preferences_update_live_and_saved_views(){
        let mut st=AppState::new(Default::default(),Default::default());st.settings.search.apps=false;st.settings.search.settings=false;st.settings.search.services=true;
        st.search.integrations=vec![Entry::new("fixture".into(),"Fixture service".into(),String::new(),"",Group::Service,Target::Service(lumo_core::search::Service{name:"fixture".into(),state:1,accepts_stop:false}))];
        st.search.view=Some(View::new(&st));st.search.saved_view=Some(View::new(&st));
        assert_eq!(entries(&st).iter().filter(|e|e.enabled(&st.settings.search)).count(),1);
        st.settings.search.services=false;refresh(&mut st);
        for view in [&st.search.view,&st.search.saved_view]{assert!(view.as_ref().unwrap().entries.iter().all(|e|e.group!=Group::Service));}
    }

    use super::*;
    #[test]
    fn toolbar_fits_small_windows_and_only_detached_browser_has_close(){
        use lumo_core::geom::Rect;
        for width in [280.0,320.0,560.0,680.0,900.0]{for detached in [false,true]{
            let t=browser_toolbar(Rect::new(0.0,0.0,width,680.0),detached);
            assert_eq!(t.close.is_some(),detached);
            let mut all=vec![t.address,t.back,t.forward,t.reload,t.external,t.mini,t.pin,t.resume];if let Some(c)=t.close{all.push(c);}if let Some(m)=t.more{all.push(m);}all.retain(|r|r.w>0.0);
            for(i,a)in all.iter().enumerate(){assert!(a.x>=0.0 && a.right()<=width && a.bottom()<=web_rect(Rect::new(0.0,0.0,width,680.0)).y);
                for b in &all[i+1..]{assert!(!(a.x<b.right() && b.x<a.right() && a.y<b.bottom() && b.y<a.bottom()),"overlap at {width}, detached={detached}");}
            }
        }}
    }
    #[test]
    fn google_suggestions_reject_stale_results_and_are_keyboard_selectable(){
        let mut st=AppState::new(Default::default(),Default::default());st.settings.search.web=true;st.settings.search.google_suggestions=true;st.search.active=true;
        let mut v=View::new(&st);v.query="no-app-987".into();v.rank(&st);st.search.view=Some(v);
        st.search.suggestions.pending=Some((7,Arc::new(crate::services::http::Cancel::default())));
        apply_suggestions(&mut st,SuggestionsEvent{request:6,query:"no-app-987".into(),items:vec!["stale".into()]});
        assert!(st.search.view.as_ref().unwrap().suggestions.is_empty());
        apply_suggestions(&mut st,SuggestionsEvent{request:7,query:"no-app-987".into(),items:vec!["Google query".into()]});
        let v=st.search.view.as_mut().unwrap();v.navigate(1);assert!(matches!(v.selected().unwrap().target,Target::Web{query,..}if query=="Google query"));
        assert!(st.search.view.as_ref().unwrap().height(&st)>204.0);
        st.settings.search.google_suggestions=false;st.search.suggestions.pending=Some((8,Arc::new(crate::services::http::Cancel::default())));
        apply_suggestions(&mut st,SuggestionsEvent{request:8,query:"no-app-987".into(),items:vec!["late".into()]});
        assert!(st.search.view.as_ref().unwrap().suggestions.is_empty());
        st.search.cancel();assert!(st.search.view.as_ref().unwrap().suggestions.is_empty());
        st.settings.search.google_suggestions=true;
        let v=st.search.view.as_mut().unwrap();v.query="hirunews.lk".into();v.dirty=true;
        assert!(suggestion_query(&mut st).is_none(),"website input is never sent as a suggestion query");
    }
    #[test]
    fn address_suggestions_are_scoped_opt_in_and_do_not_send_urls(){
        let mut st=AppState::new(Default::default(),Default::default());st.settings.search.web=true;st.settings.search.google_suggestions=true;
        st.search.detached=true;st.search.address_editing=true;st.search.address_dirty=true;st.search.address="weather tomorrow".into();
        assert_eq!(suggestion_query(&mut st).as_deref(),Some("weather tomorrow"));
        st.search.suggestions.address=true;st.search.suggestions.pending=Some((7,Arc::new(crate::services::http::Cancel::default())));
        apply_suggestions(&mut st,SuggestionsEvent{request:7,query:"weather tomorrow".into(),items:vec!["weather tomorrow colombo".into()]});
        assert_eq!(st.search.address_suggestions.len(),1);
        st.search.address="https://example.com/private".into();assert!(suggestion_query(&mut st).is_none());
        st.search.address="weather tomorrow".into();st.settings.search.google_suggestions=false;assert!(suggestion_query(&mut st).is_none());
        st.settings.search.google_suggestions=true;st.search.address_editing=false;assert!(suggestion_query(&mut st).is_none());
    }
    #[test]
    fn address_navigation_is_explicit_and_web_only() {
        use lumo_core::search::Engine;
        for (input,expected) in [
            ("hirunews.lk","https://hirunews.lk"),
            ("www.hirunews.lk","https://www.hirunews.lk"),
            ("http://hirunews.lk","http://hirunews.lk"),
            ("https://hirunews.lk","https://hirunews.lk"),
            ("HTTPS://example.com/news?q=one#top","https://example.com/news?q=one#top"),
            (" example.com:8443/path ","https://example.com:8443/path"),
            ("example.com?q=test","https://example.com?q=test"),
            ("http://[::1]:8080/news","http://[::1]:8080/news"),
        ] {
            for engine in Engine::ALL {
                let entry=web_entry(input,engine).unwrap();assert_eq!(entry.title,"Visit");
                assert!(matches!(entry.target,Target::Web{url,..}if url==expected));
                assert!(matches!(browser_target(input,engine),Target::Web{url,..}if url==expected));
            }
        }
        for query in ["weather today","version 1.2","example..com",".com","example.com:invalid"]{
            assert!(direct_address(query).is_none(),"{query}");
            assert_eq!(web_entry(query,Engine::Google).unwrap().title,"Search Google");
        }
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
            "JAVASCRIPT:alert(1)",
            "file:///C:/test",
            "data:text/html,test",
            "https://",
            "https://bad host",
            "abc\nxyz",
        ] {
            assert!(address_target(bad, Engine::Google).is_none(), "{bad}");
        }
        for engine in Engine::ALL {
            let Target::Web { query, url } = browser_target("  ", engine) else {
                panic!()
            };
            assert!(query.is_empty());
            assert!(url.starts_with("https://") && !url.contains('?'));
            let Target::Web { query, url } = browser_target(" About your PC ", engine) else {
                panic!()
            };
            assert_eq!(query, "About your PC");
            assert_eq!(Some(url), engine.url("About your PC"));
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
