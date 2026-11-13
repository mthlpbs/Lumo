//! Website AI choices share Search's on-demand WebView2 profile.
use crate::{
    commands::Command,
    gfx::TextStyle,
    icons::Icon,
    state::{AppState, NoticeAction},
    ui::{wid, wid_n, KeyInput, Ui},
};
use lumo_core::geom::{palette, Color, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    ChatGpt,
    Gemini,
    Claude,
    Muse,
    Perplexity,
}
impl Site {
    pub fn id(self) -> u64 {
        match self { Self::ChatGpt=>1, Self::Gemini=>2, Self::Claude=>3, Self::Muse=>4, Self::Perplexity=>5 }
    }
    pub fn configured<'a>(self, settings: &'a lumo_core::settings::Settings) -> Option<&'a lumo_core::web::ChatSite> {
        settings.web.chat_sites.iter().find(|s|s.id==self.id())
    }
    pub const ALL: [Self; 4] = [
        Self::ChatGpt,
        Self::Gemini,
        Self::Claude,
        Self::Muse,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::ChatGpt => "ChatGPT",
            Self::Gemini => "Gemini",
            Self::Claude => "Claude",
            Self::Muse => "Muse",
            Self::Perplexity => "Perplexity AI",
        }
    }
    pub fn host(self) -> &'static str {
        match self {
            Self::ChatGpt => "chatgpt.com",
            Self::Gemini => "gemini.google.com",
            Self::Claude => "claude.ai",
            Self::Muse => "muse.ai",
            Self::Perplexity => "www.perplexity.ai",
        }
    }
    pub fn url(self) -> &'static str {
        match self {
            Self::ChatGpt => "https://chatgpt.com/",
            Self::Gemini => "https://gemini.google.com/app",
            Self::Claude => "https://claude.ai/",
            Self::Muse => "https://muse.ai/",
            Self::Perplexity => "https://www.perplexity.ai/",
        }
    }
    pub fn icon(self) -> Icon {
        match self {
            Self::ChatGpt => Icon::ChatGpt,
            Self::Gemini => Icon::Gemini,
            Self::Claude => Icon::Claude,
            Self::Muse => Icon::Muse,
            Self::Perplexity => Icon::Perplexity,
        }
    }
    pub fn color(self) -> Color {
        match self {
            Self::ChatGpt => palette::TEXT,
            Self::Gemini => Color::hex(0x8AB4F8),
            Self::Claude => Color::hex(0xD97757),
            Self::Muse => Color::hex(0x0082FB),
            Self::Perplexity => Color::hex(0x6CCAD0),
        }
    }
    pub fn accepts(self, url: &str) -> bool {
        let Some(rest) = url.strip_prefix("https://") else {
            return cfg!(test) && url == "about:blank";
        };
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        host == self.host() || (self == Self::Perplexity && host == "perplexity.ai") || (self == Self::Muse && matches!(host,"www.muse.ai"|"app.muse.ai"))
    }
    pub fn script(self) -> String {
        include_str!("ai_sites_bridge.js")
            .replace(
                "__LUMO_SITE__",
                &serde_json::to_string(self.name()).unwrap(),
            )
            .replace(
                "__LUMO_HOST__",
                &serde_json::to_string(self.host()).unwrap(),
            )
            .replace("__LUMO_TEST__", if cfg!(test) { "true" } else { "false" })
    }
}
#[derive(Clone, Debug)]
pub struct Reply {
    pub site: Site,
    pub url: String,
    pub preview: String,
}
#[derive(Default)]
pub struct State {
    pub attempted: bool,
    pub selected: Option<Site>,
    pub menu: bool,
    pub row: usize,
    pub first_row: usize,
    pub hint: Option<&'static str>,
    pub url: String,
    pub error: String,
    pub web_ready: bool,
    pub web_visible: bool,
    pub working: bool,
    pub ready: Option<Reply>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Working,
    Ready(String),
    Idle,
    DismissMenu,
}
pub fn event(site: Site, source: &str, json: &str) -> Option<Event> {
    if !site.accepts(source) || json.len() > 8192 {
        return None;
    }
    let data: serde_json::Value = serde_json::from_str(json).ok()?;
    if data.get("channel")?.as_str()? != "lumo.ai-site" {
        return None;
    }
    match data.get("kind")?.as_str()? {
        "working" => Some(Event::Working),
        "idle" => Some(Event::Idle),
        "dismiss" => Some(Event::DismissMenu),
        "ready" => {
            let preview = data
                .get("text")?
                .as_str()?
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (!preview.is_empty()).then(|| Event::Ready(preview.chars().take(240).collect()))
        }
        _ => None,
    }
}
pub fn clear_ready(st: &mut AppState) {
    st.ai.sites.ready = None;
    if st
        .notice
        .as_ref()
        .is_some_and(|n| matches!(n.action, Some(NoticeAction::OpenAiSite(_))))
    {
        st.notice = None;
    }
}
pub fn announce_ready(st: &mut AppState) {
    let Some(reply) = st.ai.sites.ready.as_ref() else {
        return;
    };
    let (site, preview) = (reply.site, reply.preview.clone());
    st.notify(
        site.icon(),
        site.color(),
        format!("{} reply ready", site.name()),
        preview,
        0.0,
        Some(NoticeAction::OpenAiSite(site)),
    );
    st.set_notice_inline(crate::state::InlineAction::Hide);
}
pub fn toolbar(area: Rect) -> Rect {
    Rect::new(area.right() - 172.0, area.y + 8.0, 164.0, 38.0)
}
pub fn hint_rect(area: Rect) -> Rect {
    let bar = toolbar(area);
    Rect::new(bar.x, bar.bottom() + 5.0, bar.w, 28.0)
}
pub fn menu_rect(area: Rect, web: bool) -> Rect {
    let width = 232.0_f32.min(area.w - 16.0);
    Rect::new(
        if web {
            area.right() - width - 8.0
        } else {
            area.x
        },
        area.y + 48.0,
        width,
        (14.0 + (Site::ALL.len() + 1) as f32 * 39.0).min(area.h - 56.0),
    )
}
pub fn covers(area: Rect, web: bool, menu: bool, hint: bool) -> Vec<Rect> {
    let mut covers = vec![toolbar(area)];
    if menu {
        covers.push(menu_rect(area, web));
    } else if hint {
        covers.push(hint_rect(area));
    }
    covers
}
pub fn title(ui: &mut Ui, st: &mut AppState, r: Rect) {
    let response = ui.button(wid("ai.providers"), r);
    if response.hovered || st.ai.sites.menu {
        ui.c.fill_rrect(r, 8.0, Color::white(0.08));
    }
    ui.c.text(
        if r.w < 70.0 { "AI" } else { "Ask AI" },
        Rect::new(r.x + 2.0, r.y, r.w - 19.0, r.h),
        &TextStyle::new(16.0),
    );
    ui.c.icon(
        Icon::ChevronDown,
        Rect::new(r.right() - 17.0, r.center().y - 6.0, 12.0, 12.0),
        palette::TEXT_DIM,
    );
    if response.clicked {
        let open = !st.ai.sites.menu;
        st.ai.close_popovers();
        st.ai.sites.menu = open;
        st.ai.sites.row = 0;
    }
}
pub fn draw_web(ui: &mut Ui, st: &mut AppState, area: Rect, full: bool) {
    if st.ai.sites.web_visible && full == st.ai.full_open {
        let radius = if full {
            24.0
        } else if crate::pill::on(st.settings.theme) {
            40.0
        } else {
            30.0
        };
        ui.c.web_surface(area, radius);
    }
    if !st.ai.sites.web_ready && st.ai.sites.error.is_empty() {
        ui.c.text(
            &format!("Opening {}…", st.ai.sites.selected.unwrap().configured(&st.settings).map_or("chat",|s|s.name.as_str())),
            area.inset(24.0, 70.0),
            &TextStyle::body().center().color(palette::TEXT_DIM),
        );
    }
    let bar = toolbar(area);
    ui.c.fill_rrect(bar, 14.0, Color::hex(0x202024));
    // A small grip leaves a native drag target in the frameless window.
    if full {
        for y in [bar.y + 14.0, bar.y + 19.0, bar.y + 24.0] {
            ui.c.fill_circle(
                lumo_core::geom::Point::new(bar.x + 10.0, y),
                1.0,
                palette::TEXT_FAINT,
            );
        }
    }
    let selected = st.ai.sites.selected.unwrap();
    let pick = Rect::new(bar.x + 20.0, bar.y + 3.0, 48.0, 32.0);
    let response = ui.button(wid("ai.providers"), pick);
    ui.c.icon(
        selected.icon(),
        Rect::new(pick.x + 6.0, pick.y + 7.0, 18.0, 18.0),
        selected.color(),
    );
    ui.c.icon(
        Icon::ChevronDown,
        Rect::new(pick.right() - 16.0, pick.y + 11.0, 10.0, 10.0),
        palette::TEXT_DIM,
    );
    if response.clicked {
        st.ai.sites.menu = !st.ai.sites.menu;
        st.ai.sites.row = st.settings.web.chat_sites.iter().position(|s| s.id == selected.id()).unwrap_or(0) + 1;
    }
    let mut hint = None;
    for (i, (id, icon, label, on, command)) in [
        (
            "ai.keep-open",
            Icon::Pin,
            "Keep open",
            st.settings.ai.keep_open,
            Command::ToggleAiKeepOpen,
        ),
        (
            "ai.open-full",
            if full {
                Icon::ExitMiniPlayer
            } else {
                Icon::MiniPlayer
            },
            if full {
                "Return to island"
            } else {
                "Open mini window"
            },
            false,
            if full {
                Command::ReturnAiToIsland
            } else {
                Command::OpenAiWindow
            },
        ),
        ("ai.close", Icon::Close, "Close AI", false, Command::CloseAi),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(bar.x + 72.0 + i as f32 * 28.0, bar.y + 3.0, 28.0, 32.0);
        let response = ui.icon_button(
            wid(id),
            r,
            icon,
            if on { palette::BLUE } else { palette::TEXT_DIM },
            16.0,
        );
        if response.clicked {
            st.cmd(command);
        }
        if response.hovered {
            hint = Some(label);
        }
    }
    st.ai.sites.hint = hint;
    if !st.ai.sites.menu {
        if let Some(label) = hint {
            let r = hint_rect(area);
            ui.c.fill_rrect(r, 14.0, Color::hex(0x303034));
            ui.c.text(label, r, &TextStyle::footnote().center());
        }
    }
    if !st.ai.sites.error.is_empty() {
        let r = Rect::new(area.x + 24.0, area.y + 86.0, area.w - 48.0, 130.0);
        ui.c.text(&st.ai.sites.error, r, &TextStyle::body().wrap().top());
        if ui
            .pill(
                wid("ai.site.retry"),
                Rect::new(r.x, r.bottom() + 12.0, 100.0, 32.0),
                "Try again",
                None,
                crate::ui::PillStyle::Utility,
            )
            .clicked
        {
            st.cmd(Command::SelectAiSite(Some(selected)));
        }
    }
    draw_menu(ui, st, area, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_bridge_rejects_other_origins_and_bounds_previews() {
        let data =
            serde_json::json!({"channel":"lumo.ai-site","kind":"ready","text":" λ ".repeat(500)})
                .to_string();
        assert_eq!(
            event(Site::ChatGpt, "https://chatgpt.com/", &data),
            Some(Event::Ready("λ ".repeat(120)))
        );
        for source in [
            "http://chatgpt.com/",
            "https://chatgpt.com.evil.test/",
            "https://chatgpt.com@evil.test/",
            "https://claude.ai/",
            "file:///chatgpt.com",
        ] {
            assert!(event(Site::ChatGpt, source, &data).is_none());
        }
        assert!(event(Site::ChatGpt, Site::ChatGpt.url(), &"x".repeat(8193)).is_none());
        assert!(event(
            Site::Muse,
            Site::Muse.url(),
            r#"{"channel":"wrong","kind":"working"}"#
        )
        .is_none());
    }
    #[test]
    fn overlay_and_menu_fit_short_and_narrow_hosts() {
        for (w, h) in [
            (320.0, 240.0),
            (360.0, 360.0),
            (560.0, 680.0),
            (1400.0, 1000.0),
        ] {
            let area = Rect::new(33.0, 17.0, w, h);
            for r in covers(area, true, true, true) {
                assert!(
                    r.x >= area.x
                        && r.y >= area.y
                        && r.right() <= area.right()
                        && r.bottom() <= area.bottom()
                );
            }
        }
    }
    #[test]
    fn website_ready_notice_is_persistent_and_opens_the_provider() {
        let mut st = AppState::new(Default::default(), Default::default());
        st.ai.sites.ready = Some(Reply {
            site: Site::Muse,
            url: Site::Muse.url().into(),
            preview: "Finished answer".into(),
        });
        announce_ready(&mut st);
        let notice = st.notice.as_ref().unwrap();
        assert!(notice.is_ai_reply());
        assert_eq!(notice.action, Some(NoticeAction::OpenAiSite(Site::Muse)));
        assert_eq!(notice.subtitle, "Finished answer");
        assert_eq!(notice.inline, Some(crate::state::InlineAction::Hide));
        clear_ready(&mut st);
        assert!(st.notice.is_none() && st.ai.sites.ready.is_none());
    }
}
pub fn draw_menu(ui: &mut Ui, st: &mut AppState, area: Rect, web: bool) {
    if !st.ai.sites.menu {
        return;
    }
    let menu = menu_rect(area, web);
    if ui.input.just_pressed
        && ui.input.press_pos.or(ui.input.mouse).is_some_and(|p| {
            !menu.contains(p)
                && !toolbar(area).contains(p)
                && !(!web && Rect::new(area.x, area.y, 90.0, 36.0).contains(p))
        })
    {
        st.ai.sites.menu = false;
        ui.input.clicked = None;
        ui.input.just_pressed = false;
        return;
    }
    let sites=Site::ALL;
    let count=sites.len()+1; // local AI and configured chat websites only
    st.ai.sites.row=st.ai.sites.row.min(count-1);
    let mut chosen = None;
    if ui.hovered(menu)&&ui.input.wheel!=0.0 {
        st.ai.sites.row=if ui.input.wheel<0.0{(st.ai.sites.row+1).min(count-1)}else{st.ai.sites.row.saturating_sub(1)};
        ui.input.wheel=0.0;
    }
    for key in &ui.input.keys {
        match key {
            KeyInput::Down => st.ai.sites.row = (st.ai.sites.row + 1) % count,
            KeyInput::Up => st.ai.sites.row = (st.ai.sites.row + count - 1) % count,
            KeyInput::Escape => st.ai.sites.menu = false,
            KeyInput::Enter => chosen = Some(st.ai.sites.row),
            _ => {}
        }
    }
    ui.c.fill_rrect(menu, 14.0, Color::hex(0x252529));
    ui.c.stroke_rrect(menu.inset(0.5, 0.5), 14.0, Color::white(0.14), 1.0);
    let visible=count.min(((menu.h-14.0)/28.0).floor().max(1.0)as usize).min(8);
    if st.ai.sites.row<st.ai.sites.first_row{st.ai.sites.first_row=st.ai.sites.row;}
    if st.ai.sites.row>=st.ai.sites.first_row+visible{st.ai.sites.first_row=st.ai.sites.row+1-visible;}
    st.ai.sites.first_row=st.ai.sites.first_row.min(count-visible);
    let row_height = ((menu.h - 14.0) / visible as f32).min(39.0);
    if count>visible {
        let height=(menu.h-14.0)*visible as f32/count as f32;
        let y=menu.y+7.0+(menu.h-14.0-height)*st.ai.sites.first_row as f32/(count-visible) as f32;
        ui.c.fill_rrect(Rect::new(menu.right()-4.0,y,2.0,height),1.0,Color::white(0.4));
    }
    for index in st.ai.sites.first_row..(st.ai.sites.first_row+visible) {
        let site = index.checked_sub(1).and_then(|i| sites.get(i)).copied();
        let row = Rect::new(
            menu.x + 6.0,
            menu.y + 7.0 + (index-st.ai.sites.first_row) as f32 * row_height,
            menu.w - 12.0,
            row_height - 1.0,
        );
        let response = ui.button(wid_n("ai.provider", index as u64), row);
        if response.hovered || index == st.ai.sites.row {
            ui.c.fill_rrect(row, 9.0, Color::white(0.08));
        }
        ui.c.icon(
            site.map_or(Icon::Sparkles, Site::icon),
            Rect::new(row.x + 10.0, row.center().y - 10.0, 20.0, 20.0),
            site.map_or(palette::BLUE, Site::color),
        );
        ui.c.text(
            site.and_then(|s|s.configured(&st.settings)).map_or("Ask AI",|s|s.name.as_str()),
            Rect::new(row.x + 42.0, row.y, row.w - 74.0, row.h),
            &TextStyle::body(),
        );
        if index<=sites.len() && site == st.ai.sites.selected {
            ui.c.icon(
                Icon::Check,
                Rect::new(row.right() - 25.0, row.y + 12.0, 14.0, 14.0),
                palette::BLUE,
            );
        }
        if response.clicked {
            chosen = Some(index);
        }
    }
    if let Some(index) = chosen {
        st.ai.sites.menu = false;
        st.cmd(Command::SelectAiSite(index.checked_sub(1).and_then(|i|sites.get(i)).copied()));
    }
}
