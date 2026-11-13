//! Compact Ask AI and the shared full-chat/model/settings surfaces.
use crate::{
    ai::{self, Action},
    commands::Command,
    gfx::TextStyle,
    icons::Icon,
    state::AppState,
    ui::{wid, wid_n, Ui},
};
use lumo_core::{
    ai::Provider,
    geom::{palette, Color, Point, Rect},
};
const BLUE: Color = Color::rgba(0.52, 0.72, 1.0, 1.0);
/// Shared conversation scale in the island and detached chat.
pub(crate) const CHAT_SIZE:f32=14.0;
const MESSAGE_GAP:f32=16.0;
fn conversation_style()->TextStyle{TextStyle::new(CHAT_SIZE).top().wrap().line_spacing(1.5).color(Color::white(0.86))}
fn text(ui: &mut Ui, s: &str, r: Rect, size: f32, col: Color) {
    ui.c.text(s, r, &TextStyle::new(size).color(col));
}
fn action(ui: &mut Ui, id: &str, r: Rect, label: &str, icon: Icon) -> bool {
    let resp = ui.button(wid(id), r);
    ui.c.fill_rrect(
        r,
        r.h / 2.0,
        Color::white(if resp.pressed {
            0.17
        } else if resp.hovered {
            0.13
        } else {
            0.085
        }),
    );
    ui.c.icon(
        icon,
        Rect::new(r.x + 10.0, r.center().y - 7.0, 14.0, 14.0),
        palette::TEXT_DIM,
    );
    text(
        ui,
        label,
        Rect::new(r.x + 30.0, r.y, r.w - 36.0, r.h),
        12.0,
        palette::TEXT_DIM,
    );
    resp.clicked
}
pub(crate) fn workspace_area(body:Rect)->Rect {Rect::new(body.x+20.0,body.y+8.0,body.w-40.0,body.h-24.0)}
pub fn height(st: &AppState) -> f32 {
    st.ai.island_height
}
pub fn next_wake(_st: &AppState) -> Option<f64> { None }
pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    draw_chat(ui, st, area, false);
}
/// One workspace in both hosts. The conversation and composer share the same
/// state; detaching never creates another chat or sends a request.
pub fn draw_chat(ui: &mut Ui, st: &mut AppState, area: Rect, full: bool) {
    ai::ensure(st);
    if st.ai.attach_menu && ui.input.just_pressed {
        let error=if st.ai.error.is_empty(){0.0}else{34.0};
        let pending=if st.ai.pending.is_empty(){0.0}else{(area.h-288.0-error).clamp(48.0,112.0)+12.0};
        let menu=Rect::new(area.x,(area.bottom()-104.0-pending-160.0).max(area.y+48.0),area.w.min(320.0),151.0);
        if ui.input.press_pos.or(ui.input.mouse).is_some_and(|p|!menu.contains(p)){
            st.ai.attach_menu=false;ui.input.clicked=None;ui.input.pressed_id=None;ui.input.released=None;ui.input.just_pressed=false;ui.input.down=false;
            ui.input.focused=Some(wid("ai.prompt"));st.cmd(Command::FocusText);
        }
    }
    let modal = st.ai.model_menu && !st.ai.model_settings;
    let providers_was_open=st.ai.sites.menu;
    let provider_input=providers_was_open.then(||std::mem::take(ui.input));
    let model_input = modal.then(|| std::mem::take(ui.input));
    let mut tip = None;
    let toolbar = Rect::new(area.x, area.y, area.w, 36.0);
    crate::ai_sites::title(ui,st,Rect::new(toolbar.x,toolbar.y,if area.w<380.0{50.0}else{86.0},36.0));
    let left = if area.w < 380.0 { 56.0 } else { 96.0 };
    for (index, (id, icon, label)) in [
        (
            "ai.history",
            Icon::Clock,
            if st.ai.history {
                "Back to conversation"
            } else {
                "Chat history"
            },
        ),
        ("ai.new", Icon::NewChat, "New chat"),
        (
            "ai.temporary",
            if st.ai.temporary {
                Icon::TemporaryChatChecked
            } else {
                Icon::TemporaryChat
            },
            "Temporary chat · never saved",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(
            toolbar.x + left + index as f32 * 28.0,
            toolbar.y + 2.0,
            28.0,
            32.0,
        );
        let active =
            (id == "ai.history" && st.ai.history) || (id == "ai.temporary" && st.ai.temporary);
        if active {
            ui.c.fill_rrect(r, 9.0, Color::white(0.10));
        }
        let response = ui.icon_button(
            wid(id),
            r,
            icon,
            if active {
                palette::TEXT
            } else {
                palette::TEXT_DIM
            },
            17.0,
        );
        if response.hovered {
            tip = Some((label, r));
        }
        if response.clicked {
            match id {
                "ai.history" => {
                    st.ai.history = !st.ai.history;
                    st.ai.close_popovers();
                }
                "ai.new" => st.cmd(Command::Ai(Action::New(st.ai.temporary))),
                _ => st.cmd(Command::Ai(Action::ToggleTemporary)),
            }
        }
    }
    for (index, (id, icon, label, active)) in [
        (
            "ai.keep-open",
            Icon::Pin,
            if st.settings.ai.keep_open {
                "Keep open: On"
            } else {
                "Keep open: Off"
            },
            st.settings.ai.keep_open,
        ),
        (
            "ai.resume",
            Icon::ResumePage,
            if st.settings.ai.resume {
                "Resume chat: On"
            } else {
                "Resume chat: Off"
            },
            st.settings.ai.resume,
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
        ),
        ("ai.close", Icon::Close, "Close AI (Esc)", false),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(
            toolbar.right() - 112.0 + index as f32 * 28.0,
            toolbar.y + 2.0,
            28.0,
            32.0,
        );
        if active {
            ui.c.fill_rrect(r, 9.0, BLUE.mul_alpha(0.12));
        }
        let response = ui.icon_button(
            wid(id),
            r,
            icon,
            if active { BLUE } else { palette::TEXT_DIM },
            16.0,
        );
        if response.hovered {
            tip = Some((label, r));
        }
        if response.clicked {
            st.cmd(match id {
                "ai.keep-open" => Command::ToggleAiKeepOpen,
                "ai.resume" => Command::ToggleAiResume,
                "ai.open-full" => {
                    if full {
                        Command::ReturnAiToIsland
                    } else {
                        Command::OpenAiWindow
                    }
                }
                "ai.settings" => Command::OpenSettings,
                _ => Command::CloseAi,
            });
        }
    }
    ui.c.fill_rect(
        Rect::new(area.x, toolbar.bottom() + 10.0, area.w, 1.0),
        Color::white(0.075),
    );
    let model = st.ai.model(&st.settings.ai);
    let availability_error = model.unavailable.as_ref().map(|s| format!("{s} Choose another model."));
    let error = if st.ai.error.is_empty() { availability_error.as_deref().unwrap_or("") } else { &st.ai.error }.to_owned();
    let error_h = if error.is_empty() { 0.0 } else { 34.0 };
    let pending_h = if st.ai.pending.is_empty() { 0.0 } else {
        (area.h - 288.0 - error_h).clamp(48.0, 112.0) + 12.0
    };
    let composer = Rect::new(area.x, area.bottom() - 104.0 - pending_h, area.w, 104.0 + pending_h);
    let view_top = toolbar.bottom() + if area.h < 400.0 { 12.0 } else { 20.0 };
    let view = Rect::new(
        area.x + 4.0,
        view_top,
        area.w - 8.0,
        (composer.y - view_top - 10.0 - error_h).max(1.0),
    );
    let wheel = model_input
        .as_ref()
        .map_or(ui.input.wheel, |input| input.wheel);
    if modal || st.ai.reasoning_menu || st.ai.attach_menu {
        ui.input.wheel = 0.0;
    }
    if st.ai.history {
        history(ui, st, view);
    } else {
        transcript(ui, st, view, true);
    }
    if st.ai.max_scroll > 80.0 && st.ai.scroll < st.ai.max_scroll - 45.0 && !st.ai.history {
        let r = Rect::new(view.center().x - 62.0, view.bottom() - 34.0, 124.0, 30.0);
        if action(ui, "ai.jump-latest", r, "Latest reply", Icon::ExpandDown) {
            st.ai.scroll = f32::MAX;
        }
    }
    if error_h > 0.0 {
        let r = Rect::new(area.x, composer.y - 34.0, area.w, 28.0);
        ui.c.fill_rrect(r, 8.0, Color::rgba(0.22, 0.13, 0.07, 1.0));
        text(ui, &error, r.inset(10.0, 0.0), 12.0, palette::ORANGE);
        if ui.hovered(r) {
            tooltip(
                ui,
                &error,
                Rect::new(area.x, r.y - 80.0, area.w, 74.0),
            );
        }
    }
    ui.c.fill_rrect(composer, 20.0, Color::white(0.095));
    ui.c.stroke_rrect(
        composer.inset(0.5, 0.5),
        20.0,
        if st.drag_hover { BLUE } else { Color::white(if ui.input.focused == Some(wid("ai.prompt")) {
            0.18
        } else {
            0.09
        }) },
        1.0,
    );
    if pending_h > 0.0 { pending_attachments(ui, st, Rect::new(composer.x + 12.0, composer.y + 12.0, composer.w - 24.0, pending_h - 12.0)); }
    let input_r = Rect::new(composer.x + 14.0, composer.y + 9.0 + pending_h, composer.w - 28.0, 48.0);
    let input = ui.text_area(
        wid("ai.prompt"),
        input_r,
        &mut st.ai.draft,
        "",
        12000,
    );
    if input.focused {
        st.cmd(Command::FocusText);
    }
    if input.cancelled && !modal && !st.ai.attach_menu {
        st.cmd(Command::CloseAi);
    }
    let plus = Rect::new(composer.x + 8.0, composer.bottom() - 42.0, 32.0, 32.0);
    let attach = ui.icon_button(
        wid("ai.plus"),
        plus,
        if st.ai.attaching {
            Icon::Clock
        } else {
            Icon::Plus
        },
        palette::TEXT_DIM,
        18.0,
    );
    if attach.clicked {
        let open = !st.ai.attach_menu && !st.ai.attaching;
        st.ai.close_popovers();
        st.ai.attach_menu = open;
    }
    let label = if model.id.is_empty() {
        "Choose model"
    } else {
        model.display_name()
    };
    let model_w = (ui.c.measure(label, &TextStyle::new(13.0), 10000.0).0 + 34.0)
        .clamp(110.0, 240.0).min((composer.w - 137.0).max(80.0));
    let model_r = Rect::new(
        composer.right() - 53.0 - model_w,
        composer.bottom() - 42.0,
        model_w,
        32.0,
    );
    let response = ui.button(wid("ai.models"), model_r);
    if response.hovered {
        ui.c.fill_rrect(model_r, 9.0, Color::white(0.07));
    }
    text(
        ui,
        label,
        Rect::new(model_r.x + 8.0, model_r.y, model_r.w - 28.0, 32.0),
        13.0,
        palette::TEXT_DIM,
    );
    ui.c.icon(
        Icon::ChevronDown,
        Rect::new(model_r.right() - 19.0, model_r.y + 11.0, 11.0, 11.0),
        palette::TEXT_DIM,
    );
    if response.clicked {
        let was = st.ai.reasoning_menu || st.ai.model_menu;
        st.ai.close_popovers();
        st.ai.model_menu = !was;
        st.ai.model_settings = false;
        st.ai.model_scroll = 0.0;
        st.ai.reveal_model = true;
        st.ai.search.clear();
        if !was {
            if st.ai.models.is_empty() {
                st.cmd(Command::Ai(Action::EnsureModels));
            }
            ui.input.focused = Some(wid("ai.model-search"));
            ui.input.caret = 0;
            st.cmd(Command::FocusText);
        }
    }
    let can_send = st.ai.busy
        || (model.unavailable.is_none() && !st.ai.attaching && (!st.ai.draft.trim().is_empty() || !st.ai.pending.is_empty()));
    let send = Rect::new(
        composer.right() - 45.0,
        composer.bottom() - 43.0,
        34.0,
        34.0,
    );
    let response = if can_send {
        Some(ui.button(wid("ai.send"), send))
    } else {
        None
    };
    ui.c.fill_rrect(send, 17.0, if can_send { BLUE } else { Color::white(0.13) });
    ui.c.icon(
        if st.ai.busy { Icon::Stop } else { Icon::Send },
        send.inset(9.0, 9.0),
        if can_send {
            Color::black(1.0)
        } else {
            palette::TEXT_FAINT
        },
    );
    if can_send && (response.is_some_and(|r| r.clicked) || input.submitted) {
        st.cmd(Command::Ai(if st.ai.busy {
            Action::Stop
        } else {
            Action::Send
        }));
    }
    if let Some(input) = model_input {
        *ui.input = input;
    }
    if st.ai.attach_menu {
        attach_menu(
            ui,
            st,
            Rect::new(
                area.x,
                (composer.y - 160.0).max(toolbar.bottom() + 12.0),
                area.w.min(320.0),
                151.0,
            ),
        );
    }
    if st.ai.model_menu && !st.ai.model_settings {
        ui.input.wheel = wheel;
        let w = area.w.min(460.0);
        model_picker(
            ui,
            st,
            Rect::new(
                area.right() - w,
                view.y,
                w,
                (composer.bottom() - 50.0 - view.y).max(100.0),
            ),
            false,
        );
    }
    if !st.ai.model_menu && !st.ai.reasoning_menu && !st.ai.attach_menu && !st.ai.sites.menu {
        if let Some((label, button)) = tip {
            let w = (ui.c.measure(label, &TextStyle::new(12.0), 260.0).0 + 24.0)
                .clamp(100.0, 280.0)
                .min(area.w);
            let y = if button.y > composer.y {
                composer.y - 56.0
            } else {
                toolbar.bottom() + 6.0
            };
            tooltip(
                ui,
                label,
                Rect::new(button.x.min(area.right() - w).max(area.x), y, w, 48.0),
            );
        }
    }
    if let Some(input)=provider_input{*ui.input=input;}
    if providers_was_open&&ui.input.clicked==Some((wid("ai.providers"),crate::ui::MouseButton::Left)){st.ai.sites.menu=false;}
    crate::ai_sites::draw_menu(ui,st,area,false);
}
fn pending_attachments(ui: &mut Ui, st: &mut AppState, tray: Rect) {
    let count = st.ai.pending.len();
    let gap = 8.0;
    let width = ((tray.w - gap * count.saturating_sub(1) as f32) / count.max(1) as f32).min(148.0);
    let mut remove = None;
    for (i, a) in st.ai.pending.iter().enumerate() {
        let r = Rect::new(tray.x + i as f32 * (width + gap), tray.y, width, tray.h);
        ui.c.fill_rrect(r, 10.0, Color::black(0.35));
        if let Some(Some(image)) = st.ai.pending_previews.get(i) {
            ui.c.image(image, r, 10.0);
            ui.c.fill_rrect(Rect::new(r.x, r.bottom() - 25.0, r.w, 25.0), 6.0, Color::black(0.72));
        } else {
            ui.c.icon(if a.mime.starts_with("image/") { Icon::Image } else { Icon::File },
                Rect::from_center(Point::new(r.center().x, r.y + (r.h - 22.0) * 0.5), 25.0, 25.0), palette::TEXT_DIM);
        }
        text(ui, &a.name, Rect::new(r.x + 7.0, r.bottom() - 24.0, r.w - 14.0, 23.0), 11.0, palette::TEXT);
        let close = Rect::new(r.right() - 25.0, r.y + 3.0, 22.0, 22.0);
        ui.c.fill_rrect(close, 11.0, Color::black(0.68));
        let response = ui.icon_button(wid_n("ai.remove-attachment", i as u64), close, Icon::Close, palette::TEXT, 11.0);
        if response.hovered{tooltip(ui,"Remove attachment",Rect::new((close.right()-160.0).max(tray.x),tray.y-38.0,160.0f32.min(tray.w),30.0));}
        if response.clicked { remove = Some(i); }
        if ui.hovered(r) && !response.hovered {
            tooltip(ui, &format!("{}\n{} · Sent when you press Send", a.name, if a.mime.starts_with("image/") { "Image" } else if a.mime == "application/pdf" { "PDF document" } else { "Text document" }),
                Rect::new(r.x.min(tray.right() - 240.0).max(tray.x), tray.y - 70.0, 240.0f32.min(tray.w), 62.0));
        }
    }
    if let Some(i) = remove {
        st.ai.pending.remove(i);
        if i < st.ai.pending_previews.len() { st.ai.pending_previews.remove(i); }
    }
}
fn message_icon(
    ui: &mut Ui,
    id: u64,
    r: Rect,
    icon: Icon,
    label: &str,
    tip: &mut Option<(String, Rect)>,
) -> bool {
    let response = ui.icon_button(id, r, icon, palette::TEXT_DIM, 15.0);
    if response.hovered {
        *tip = Some((label.into(), r));
    }
    response.clicked
}
fn transcript(ui: &mut Ui, st: &mut AppState, view: Rect, full: bool) {
    let chat = &st.ai.current;
    if chat.messages.is_empty() {
        st.ai.max_scroll = 0.0;
        st.ai.scroll = 0.0;
        if st.ai.temporary {
            let y = view.y + (view.h * 0.32).max(0.0);
            ui.c.text("Temporary chat", Rect::new(view.x, y, view.w, 38.0f32.min(view.h)),
                &TextStyle::new(if view.w < 380.0 { 24.0 } else { 28.0 }).center());
            if view.h > 95.0 {
                ui.c.text("This conversation won’t appear in your history.\nMessages are sent to your provider only when you press Send.",
                    Rect::new(view.x + 14.0, y + 48.0, view.w - 28.0, (view.bottom() - y - 48.0).min(72.0)),
                    &TextStyle::new(13.0).center().top().wrap().color(palette::TEXT_DIM));
            }
            return;
        }
        if view.h < 230.0 {
            text(
                ui,
                "Ask a question",
                Rect::new(view.x + 4.0, view.y + 4.0, view.w - 8.0, 26.0),
                18.0,
                palette::TEXT,
            );
            ui.c.text(
                if st.ai.temporary {
                    "Temporary chat · not saved"
                } else {
                    "Write a message or add a file below."
                },
                Rect::new(
                    view.x + 4.0,
                    view.y + 36.0,
                    view.w - 8.0,
                    (view.h - 40.0).max(1.0),
                ),
                &TextStyle::new(12.0).color(palette::TEXT_DIM).top().wrap(),
            );
            return;
        }
        let y = view.y + (view.h * 0.23).max(12.0);
        ui.c.icon(
            Icon::Sparkles,
            Rect::new(view.center().x - 16.0, y, 32.0, 32.0),
            BLUE,
        );
        ui.c.text(
            "What are you working on?",
            Rect::new(view.x, y + 48.0, view.w, 38.0),
            &TextStyle::new(24.0).center(),
        );
        let hint = if st.ai.temporary {
            "Temporary chat. This conversation stays out of history."
        } else if st.settings.ai.default_model.is_empty() {
            "Connect a provider in AI settings to get started."
        } else {
            "Ask a question, work through an idea, or add a document."
        };
        ui.c.text(
            hint,
            Rect::new(view.x + 12.0, y + 90.0, view.w - 24.0, 46.0),
            &TextStyle::new(13.0)
                .color(palette::TEXT_DIM)
                .center()
                .wrap(),
        );
        if view.h > 250.0 {
            if st.settings.ai.default_model.is_empty() {
                let r = Rect::new(view.center().x - 96.0, y + 154.0, 192.0, 36.0);
                if ui
                    .pill(
                        wid("ai.setup"),
                        r,
                        "Connect a provider",
                        Some(Icon::Gear),
                        crate::ui::PillStyle::Primary(BLUE),
                    )
                    .clicked
                {
                    st.cmd(Command::OpenSettings);
                }
                return;
            }
            for (i, (label, prompt)) in [
                ("Explain something", "Explain this in simple terms: "),
                ("Draft a message", "Help me write a message about "),
                ("Work through an idea", "Help me think through "),
            ]
            .into_iter()
            .enumerate()
            {
                let r = Rect::new(
                    view.center().x - ((view.w - 24.0).min(340.0)) / 2.0,
                    y + 154.0 + i as f32 * 36.0,
                    (view.w - 24.0).min(340.0),
                    30.0,
                );
                if r.bottom() > view.bottom() {
                    break;
                }
                if action(ui, &format!("ai.starter.{i}"), r, label, Icon::ChevronRight) {
                    st.ai.draft = prompt.into();
                    ui.input.focused = Some(wid("ai.prompt"));
                    ui.input.caret = st.ai.draft.chars().count();
                    st.cmd(Command::FocusText);
                }
            }
        }
        return;
    }
    let first = if full {
        0
    } else {
        chat.messages
            .len()
            .saturating_sub(if st.ai.busy { 1 } else { 2 })
    };
    let size = CHAT_SIZE;
    let style = conversation_style();
    let mut messages = vec![];
    let mut total = 0.0;
    let mut anchor = None;
    for (i, m) in chat.messages.iter().enumerate().skip(first) {
        let user = m.role == "user";
        let mut content = m.text.clone();
        if user && !m.attachments.is_empty() {
            content += &format!(
                "\n+ {}",
                m.attachments
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let max_user_w=(view.w*0.80).max(80.0);
        let message_w=if user{(ui.c.measure(&content,&style,max_user_w-24.0).0+24.0).ceil().clamp(48.0,max_user_w)}else{view.w};
        let markdown = if user {
            None
        } else {
            Some(super::ai_markdown::layout(ui, &m.text, view.w - 8.0, size))
        };
        let text_h = markdown
            .as_ref()
            .map_or_else(
                || ui.c.measure(&content, &style, message_w - 24.0).1,
                |m| m.height,
            )
            .max(CHAT_SIZE*1.5);
        let image_h = if user {
            0.0
        } else {
            m.attachments
                .iter()
                .filter(|a| a.mime.starts_with("image/"))
                .count() as f32
                * if full { 230.0 } else { 110.0 }
        };
        let h = text_h + if user { 24.0 } else { 32.0 } + image_h;
        if st.ai.scroll_to_reply == Some(i) {
            anchor = Some(total);
        }
        messages.push((i, h, text_h, content, markdown,message_w));
        total += h + MESSAGE_GAP;
    }
    if st.ai.busy {
        total += 36.0;
    }
    st.ai.max_scroll = (total - view.h).max(0.0);
    if let Some(y) = anchor {
        st.ai.scroll = y;
        st.ai.scroll_to_reply = None;
    }
    st.ai.scroll = st.ai.scroll.clamp(0.0, st.ai.max_scroll);
    let wheel = ui.wheel_in(view);
    if wheel != 0.0 {
        st.ai.scroll = (st.ai.scroll - wheel * 42.0).clamp(0.0, st.ai.max_scroll);
    }
    let mut y = view.y - st.ai.scroll;
    let mut commands = vec![];
    let mut tip = None;
    let fixed_hits = ui.hits.len();
    ui.c.clip(view);
    for (i, h, text_h, content, markdown,w) in messages {
        let m = &chat.messages[i];
        let user = m.role == "user";
        let r = Rect::new(if user { view.right() - w } else { view.x }, y, w, h);
        if r.bottom() >= view.y && r.y < view.bottom() {
            let alpha = ui.c.alpha();
            let age = (ui.now - st.ai.reply_at) as f32;
            if !user
                && i == chat.messages.len() - 1
                && (0.0..0.18).contains(&age)
                && !st.power.saver
            {
                ui.c.set_alpha(alpha * super::ease_out_cubic(age / 0.18));
                ui.request_anim();
            }
            if user {
                ui.c.fill_rrect(r, 16.0, Color::white(0.12));
                ui.c.text(
                    &content,
                    Rect::new(r.x + 12.0, r.y + 12.0, r.w - 24.0, text_h + 2.0),
                    &style,
                );
            } else if let Some(markdown) = markdown {
                super::ai_markdown::draw(
                    ui,
                    &markdown,
                    Point::new(r.x, r.y),
                    view,
                    chat.id.wrapping_add(i as u64),
                    &mut commands,
                );
            }
            let hovered = !user
                && ui.hovered(r)
                && ui.hovered(view)
                && !st.ai.model_menu
                && !st.ai.attach_menu;
            if !user {
                let mut image_y = r.y + text_h + 10.0;
                for (a_index, a) in m.attachments.iter().enumerate() {
                    if !a.mime.starts_with("image/") {
                        continue;
                    }
                    let image_r = Rect::new(
                        r.x,
                        image_y,
                        if full {
                            220.0f32.min(view.w - 112.0)
                        } else {
                            100.0
                        },
                        if full { 216.0 } else { 100.0 },
                    );
                    image_y += if full { 230.0 } else { 110.0 };
                    if image_r.bottom() >= view.y && image_r.y < view.bottom() {
                        if let Some(image) = st.ai.previews.get(&(i, a_index)) {
                            ui.c.image(image, image_r, 10.0);
                        } else {
                            text(ui, "Generated image", image_r, 12.0, palette::TEXT_DIM);
                        }
                        if hovered {
                            let x = image_r.right() + 8.0;
                            let controls = Rect::new(x, image_r.y + 4.0, 104.0, 32.0);
                            ui.c.fill_rrect(controls, 16.0, Color::white(0.09));
                            for (n, (icon, label, action)) in [
                                (Icon::Copy, "Copy image", Action::CopyImage(i, a_index)),
                                (
                                    Icon::Download,
                                    "Download image",
                                    Action::DownloadImage(i, a_index),
                                ),
                                (
                                    Icon::Plus,
                                    "Use image in prompt",
                                    Action::ReuseImage(i, a_index),
                                ),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                if message_icon(
                                    ui,
                                    wid(&format!("ai.image.{i}.{a_index}.{n}")),
                                    Rect::new(
                                        x + 4.0 + n as f32 * 32.0,
                                        controls.y + 2.0,
                                        30.0,
                                        28.0,
                                    ),
                                    icon,
                                    label,
                                    &mut tip,
                                ) {
                                    commands.push(Command::Ai(action));
                                }
                            }
                        }
                    }
                }
                if hovered {
                    let retry = i == chat.messages.len() - 1 && !st.ai.busy;
                    let count = 2 + usize::from(retry) + usize::from(!full);
                    let bar = Rect::new(r.x, r.bottom() - 30.0, count as f32 * 34.0 + 8.0, 30.0);
                    ui.c.fill_rrect(bar, 15.0, Color::rgba(0.11, 0.11, 0.12, 0.98));
                    let mut x = bar.x + 4.0;
                    for (kind, icon, label) in [
                        (0, Icon::Copy, "Copy as plain text"),
                        (1, Icon::Markdown, "Copy Markdown"),
                        (2, Icon::Reset, "Retry reply"),
                        (
                            3,
                            if st.ai.island_height > 245.0 {
                                Icon::CollapseUp
                            } else {
                                Icon::ExpandDown
                            },
                            if st.ai.island_height > 245.0 {
                                "Compact chat"
                            } else {
                                "Expand chat"
                            },
                        ),
                    ] {
                        if kind == 2 && !retry || kind == 3 && full {
                            continue;
                        }
                        let id = match kind {
                            0 => wid(&format!("ai.copy.{i}")),
                            1 => wid(&format!("ai.markdown.{i}")),
                            2 => wid("ai.retry"),
                            _ => wid("ai.full"),
                        };
                        if message_icon(
                            ui,
                            id,
                            Rect::new(x, bar.y + 1.0, 32.0, 28.0),
                            icon,
                            label,
                            &mut tip,
                        ) {
                            match kind {
                                0 => commands
                                    .push(Command::CopyText(lumo_core::ai::plain_text(&m.text))),
                                1 => commands.push(Command::CopyText(m.text.clone())),
                                2 => commands.push(Command::Ai(Action::Retry)),
                                _ => commands.push(Command::AiHeight(
                                    if st.ai.island_height > 245.0 {
                                        240.0
                                    } else {
                                        st.ai.max_island_height
                                    },
                                )),
                            }
                        }
                        x += 34.0;
                    }
                }
            }
            ui.c.set_alpha(alpha);
        }
        y += h + MESSAGE_GAP;
    }
    if st.ai.busy {
        let row = Rect::new(view.x, y, view.w, 30.0);
        if row.bottom() > view.y && row.y < view.bottom() {
            text(
                ui,
                if st.ai.running_chat == chat.id {
                    "Generating a reply"
                } else {
                    "Generating in another chat"
                },
                Rect::new(row.x + 42.0, row.y, row.w - 42.0, row.h),
                13.0,
                palette::TEXT_DIM,
            );
            for i in 0..3 {
                let phase = (ui.now * 1.4 - i as f64 * 0.18).rem_euclid(1.0);
                let pulse = if st.power.saver {
                    0.4
                } else {
                    ((phase * std::f64::consts::TAU).cos() as f32 + 1.0) * 0.5
                };
                ui.c.fill_rrect(
                    Rect::new(row.x + i as f32 * 11.0, row.y + 12.0, 6.0, 6.0),
                    3.0,
                    BLUE.mul_alpha(0.3 + 0.7 * pulse),
                );
            }
            // Only the visible pending reply animates, and never in saver.
            // Locked/display-off windows already skip drawing in App.
            if !st.power.saver {
                ui.request_live();
            }
        }
    }
    ui.c.unclip();
    clip_hits(ui, fixed_hits, view);
    for c in commands {
        st.cmd(c);
    }
    if st.ai.max_scroll > 0.0 {
        let h = (view.h * view.h / total).max(20.0);
        let track = Rect::new(view.right() - 10.0, view.y, 10.0, view.h);
        let response = ui.button(wid("ai.transcript-scrollbar"), track);
        if response.pressed {
            if let Some(p) = ui.input.mouse {
                st.ai.scroll = ((p.y - view.y - h / 2.0) / (view.h - h) * st.ai.max_scroll)
                    .clamp(0.0, st.ai.max_scroll);
            }
        }
        ui.c.fill_rrect(
            Rect::new(
                view.right() - 3.0,
                view.y + (view.h - h) * st.ai.scroll / st.ai.max_scroll,
                2.0,
                h,
            ),
            1.0,
            Color::white(0.28),
        );
    }
    if let Some((label, button)) = tip {
        tooltip(
            ui,
            &label,
            Rect::new(button.x, button.y - 35.0, 180.0, 30.0),
        );
    }
}

fn history(ui: &mut Ui, st: &mut AppState, view: Rect) {
    let filter = Rect::new(view.x, view.y, view.w, 34.0);
    let input = ui.text_input(
        wid("ai.history-filter"),
        filter,
        &mut st.ai.history_query,
        "Search saved chats",
        120,
    );
    if input.focused {
        st.cmd(Command::FocusText);
    }
    if input.changed {
        st.ai.history_scroll = 0.0;
    }
    let view = Rect::new(view.x, view.y + 48.0, view.w, (view.h - 48.0).max(1.0));
    let query = st.ai.history_query.trim().to_lowercase();
    let matches: Vec<_> = st
        .ai
        .chats
        .iter()
        .filter(|c| query.is_empty() || c.title.to_lowercase().contains(&query))
        .collect();
    let fixed_hits = ui.hits.len();
    let mut commands = vec![];
    let wheel = ui.wheel_in(view);
    st.ai.history_scroll = (st.ai.history_scroll - wheel * 40.0)
        .clamp(0.0, (matches.len() as f32 * 43.0 + 32.0 - view.h).max(0.0));
    ui.c.clip(view);
    text(
        ui,
        "Saved on this PC · encrypted for your Windows account",
        Rect::new(view.x, view.y - st.ai.history_scroll, view.w, 24.0),
        12.0,
        palette::TEXT_DIM,
    );
    for (i, c) in matches.iter().enumerate() {
        let r = Rect::new(
            view.x,
            view.y + 30.0 + i as f32 * 43.0 - st.ai.history_scroll,
            view.w,
            37.0,
        );
        if r.bottom() < view.y || r.y > view.bottom() {
            continue;
        }
        let b = ui.button(wid_n("ai.chat", c.id), Rect::new(r.x, r.y, r.w - 40.0, r.h));
        if b.hovered {
            ui.c.fill_rrect(r, 9.0, Color::white(0.08));
        }
        text(
            ui,
            &c.title,
            Rect::new(r.x + 8.0, r.y, r.w - 56.0, r.h),
            14.0,
            palette::TEXT,
        );
        if b.clicked {
            commands.push(Action::Select(c.id));
        }
        if ui
            .icon_button(
                wid_n("ai.delete", c.id),
                Rect::new(r.right() - 34.0, r.y + 3.0, 30.0, 30.0),
                Icon::Trash,
                palette::TEXT_DIM,
                15.0,
            )
            .clicked
        {
            commands.push(Action::Delete(c.id));
        }
    }
    if matches.is_empty() {
        text(
            ui,
            if query.is_empty() {
                "No saved chats yet"
            } else {
                "No matching chats"
            },
            view.inset(10.0, 30.0),
            15.0,
            palette::TEXT_DIM,
        );
    }
    ui.c.unclip();
    clip_hits(ui, fixed_hits, view);
    for a in commands {
        st.cmd(Command::Ai(a));
    }
}
fn attach_menu(ui: &mut Ui, st: &mut AppState, r: Rect) {
    ui.button(wid("ai.attach-surface"), r);
    ui.c.fill_rrect(r, 14.0, Color::rgba(0.14, 0.14, 0.15, 1.0));
    ui.c.stroke_rrect(r, 14.0, Color::white(0.12), 1.0);
    text(
        ui,
        "Add to message",
        Rect::new(r.x + 12.0, r.y + 5.0, 120.0, 21.0),
        12.0,
        palette::TEXT,
    );
    text(
        ui,
        "4 files · 6 MB each",
        Rect::new(r.right() - 122.0, r.y + 5.0, 112.0, 21.0),
        10.0,
        palette::TEXT_DIM,
    );
    for (i, (label, detail, icon, a)) in [
        (
            "Images",
            "PNG, JPEG, WebP or GIF",
            Icon::Image,
            Action::Attach(true),
        ),
        (
            "Documents",
            "PDF or UTF-8 text",
            Icon::File,
            Action::Attach(false),
        ),
        ("Paste image or file", "Ctrl+V", Icon::Clipboard, Action::PasteAttachment),
    ]
    .into_iter()
    .enumerate()
    {
        let row = Rect::new(
            r.x + 6.0,
            r.y + if i < 2 {
                28.0 + i as f32 * 40.0
            } else {
                115.0 + (i - 2) as f32 * 29.0
            },
            r.w - 12.0,
            if i < 2 { 38.0 } else { 28.0 },
        );
        let response = ui.button(wid_n("ai.attach", i as u64), row);
        if response.hovered || response.pressed {
            ui.c.fill_rrect(
                row,
                8.0,
                Color::white(if response.pressed { 0.14 } else { 0.08 }),
            );
        }
        ui.c.icon(
            icon,
            Rect::new(row.x + 7.0, row.center().y - 9.0, 18.0, 18.0),
            palette::TEXT_DIM,
        );
        text(
            ui,
            label,
            Rect::new(
                row.x + 36.0,
                row.y + if i < 2 { 1.0 } else { 0.0 },
                row.w - 102.0,
                if i < 2 { 21.0 } else { 28.0 },
            ),
            12.0,
            palette::TEXT,
        );
        if i < 2 {
            text(
                ui,
                detail,
                Rect::new(row.x + 36.0, row.y + 20.0, row.w - 44.0, 16.0),
                10.0,
                palette::TEXT_DIM,
            );
        } else if i == 2 {
            text(
                ui,
                detail,
                Rect::new(row.right() - 54.0, row.y, 50.0, row.h),
                10.0,
                palette::TEXT_FAINT,
            );
        }
        if response.clicked {
            st.cmd(Command::Ai(a));
            st.ai.attach_menu = false;
        }
    }
    ui.c.line(
        Point::new(r.x + 12.0, r.y + 111.0),
        Point::new(r.right() - 12.0, r.y + 111.0),
        Color::white(0.08),
        1.0,
    );
}
/// Draw a passive action hint on the current surface. No window, timer, focus
/// change or idle work; mouse movement already asks the host to repaint.
pub fn tooltip(ui: &mut Ui, content: &str, area: Rect) {
    if content.is_empty()||area.w<=0.0||area.h<=0.0{return;}
    let style=TextStyle::new(12.0).top().wrap();
    let bounds=ui.c.bounds().inset(8.0,8.0);
    let width=area.w.min(bounds.w).max(1.0);
    let height=(ui.c.measure(content,&style,(width-24.0).max(1.0)).1+12.0).min(area.h).min(bounds.h);
    let r=Rect::new(area.x.clamp(bounds.x,(bounds.right()-width).max(bounds.x)),area.y.clamp(bounds.y,(bounds.bottom()-height).max(bounds.y)),width,height);
    ui.c.fill_rrect(r,8.0,Color::rgba(0.16,0.16,0.17,1.0));
    ui.c.stroke_rrect(r.inset(0.5,0.5),8.0,Color::white(0.12),1.0);
    ui.c.text(content,r.inset(12.0,6.0),&style.color(palette::TEXT));
}
pub fn model_picker(ui: &mut Ui, st: &mut AppState, r: Rect, settings: bool) {
    super::ai_models::picker(ui, st, r, settings);
}

/// A compact chat keeps its composer visible; its model list is drawn by
/// the island host below the card, outside the chat's content clip.
pub fn picker_below(st: &AppState) -> bool {
    let _ = st;
    false
}
pub fn settings(ui: &mut Ui, st: &mut AppState, area: Rect) -> f32 {
    if !st.ai.loaded && !st.ai.loading {
        st.ai.initialize();
        st.cmd(Command::Ai(Action::Load));
    }
    let mut y = area.y;
    let w = area.w;
    text(
        ui,
        "Open AI",
        Rect::new(area.x, y, w, 24.0),
        18.0,
        palette::TEXT,
    );
    y += 34.0;
    for (id, label, hint, value) in [
        (
            "ai.pref-gesture",
            "Scroll left on the closed island",
            "Right scroll opens Search.",
            st.settings.ai.horizontal_scroll,
        ),
        (
            "ai.pref-pin",
            "Keep open",
            "Prevent automatic closing when you switch apps.",
            st.settings.ai.keep_open,
        ),
        (
            "ai.pref-resume",
            "Resume chat",
            "Return to this conversation and its draft on reopening.",
            st.settings.ai.resume,
        ),
    ] {
        let r = Rect::new(area.x, y, w, 62.0);
        text(
            ui,
            label,
            Rect::new(r.x + 4.0, r.y + 6.0, w - 70.0, 22.0),
            14.0,
            palette::TEXT,
        );
        text(
            ui,
            hint,
            Rect::new(r.x + 4.0, r.y + 29.0, w - 70.0, 25.0),
            12.0,
            palette::TEXT_DIM,
        );
        let switch = Rect::new(r.right() - 56.0, r.y + 18.0, 50.0, 28.0);
        let response = ui.button(wid(id), r);
        ui.c.fill_rrect(switch, 14.0, if value { BLUE } else { Color::white(0.16) });
        ui.c.fill_circle(
            Point::new(
                if value {
                    switch.right() - 14.0
                } else {
                    switch.x + 14.0
                },
                switch.center().y,
            ),
            10.0,
            Color::white(0.95),
        );
        if response.clicked {
            match id {
                "ai.pref-gesture" => {
                    st.settings.ai.horizontal_scroll = !value;
                    st.cmd(Command::SaveSettings);
                }
                "ai.pref-pin" => st.cmd(Command::ToggleAiKeepOpen),
                _ => st.cmd(Command::ToggleAiResume),
            }
        }
        y += 64.0;
    }
    text(
        ui,
        &format!(
            "Mini window: {} × {}",
            st.settings.ai.mini_width, st.settings.ai.mini_height
        ),
        Rect::new(area.x + 4.0, y, w - 100.0, 30.0),
        12.0,
        palette::TEXT_DIM,
    );
    if action(
        ui,
        "ai.pref-reset-size",
        Rect::new(area.right() - 88.0, y, 88.0, 30.0),
        "Reset",
        Icon::Reset,
    ) {
        st.settings.ai.mini_width = 560;
        st.settings.ai.mini_height = 680;
        st.cmd(Command::SaveSettings);
    }
    y += 58.0;
    text(
        ui,
        "Provider",
        Rect::new(area.x, y, w, 20.0),
        14.0,
        palette::TEXT,
    );
    y += 26.0;
    let mut x = area.x;
    for (i, p) in Provider::ALL.into_iter().enumerate() {
        let width = if i == 3 { 145.0 } else { 88.0 };
        let r = Rect::new(x, y, width, 30.0);
        let b = ui.button(wid_n("ai.provider", i as u64), r);
        ui.c.fill_rrect(
            r,
            15.0,
            Color::white(if st.settings.ai.provider == p {
                0.20
            } else if b.hovered {
                0.12
            } else {
                0.065
            }),
        );
        text(ui, p.name(), r.inset(10.0, 0.0), 12.0, palette::TEXT);
        if b.clicked && st.settings.ai.provider != p {
            st.settings.ai.switch(p);
            st.ai.models.clear();
            st.ai.catalog_saved_at = 0;
            st.ai.catalog_from_cache = false;
            st.ai.selected.clear();
            st.ai.key_present = false;
            st.ai.key_draft.clear();
            st.ai.key_visible = false;
            st.ai.fetching = false;
            st.cmd(Command::SaveSettings);
            st.cmd(Command::Ai(Action::CheckKey));
        }
        x += width + 5.0;
    }
    y += 46.0;
    text(
        ui,
        "API key · encrypted for your Windows account",
        Rect::new(area.x, y, w, 20.0),
        13.0,
        palette::TEXT_DIM,
    );
    y += 25.0;
    let response = if st.ai.key_visible {
        ui.text_input(
            wid("ai.api-key"),
            Rect::new(area.x, y, w - 174.0, 34.0),
            &mut st.ai.key_draft,
            if st.ai.key_present {
                "Key saved · enter to replace"
            } else {
                "Enter your API key"
            },
            1000,
        )
    } else {
        ui.text_secret(
            wid("ai.api-key"),
            Rect::new(area.x, y, w - 174.0, 34.0),
            &mut st.ai.key_draft,
            if st.ai.key_present {
                "Key saved · enter to replace"
            } else {
                "Enter your API key"
            },
            1000,
        )
    };
    if response.focused {
        st.cmd(Command::FocusText);
    }
    if action(
        ui,
        "ai.key-show",
        Rect::new(area.right() - 167.0, y, 80.0, 34.0),
        if st.ai.key_visible { "Hide" } else { "Show" },
        Icon::Circle,
    ) {
        st.ai.key_visible = !st.ai.key_visible;
    }
    if action(
        ui,
        "ai.key-save",
        Rect::new(area.right() - 80.0, y, 80.0, 34.0),
        if st.ai.key_saving { "Saving" } else { "Save" },
        Icon::Check,
    ) {
        st.cmd(Command::SaveSettings);
        st.cmd(Command::Ai(Action::SaveKey));
    }
    y += 42.0;
    if st.ai.key_present
        && action(
            ui,
            "ai.key-clear",
            Rect::new(area.x, y, 142.0, 27.0),
            "Remove saved key",
            Icon::Trash,
        )
    {
        st.cmd(Command::Ai(Action::ClearKey));
    }
    y += if st.ai.key_present { 38.0 } else { 12.0 };
    let was_editing = st.ai.config_editing;
    let mut editing = false;
    for (i, label) in [
        "Chat completion URL",
        "Model list fetch URL",
        "Default model ID",
    ]
    .into_iter()
    .enumerate()
    {
        text(
            ui,
            label,
            Rect::new(area.x, y, w, 20.0),
            13.0,
            palette::TEXT_DIM,
        );
        y += 24.0;
        let placeholder = match i {
            0 => st.settings.ai.provider.completion(),
            1 => st.settings.ai.provider.models(),
            _ => "Fetch and select a model, or enter its exact ID",
        };
        let buf = match i {
            0 => &mut st.settings.ai.completion_url,
            1 => &mut st.settings.ai.models_url,
            _ => &mut st.settings.ai.default_model,
        };
        let response = ui.text_input(
            wid_n("ai.config", i as u64),
            Rect::new(area.x, y, w, 36.0),
            buf,
            placeholder,
            2048,
        );
        if response.focused {
            editing = true;
            st.cmd(Command::FocusText);
        }
        if response.changed && i < 2 {
            st.ai.models.clear();
            st.ai.catalog_saved_at = 0;
            st.ai.catalog_from_cache = false;
            st.ai.fetching = false;
        }
        if response.submitted {
            st.cmd(Command::SaveSettings);
            if i == 1 {
                st.cmd(Command::Ai(Action::EnsureModels));
            }
        }
        y += 49.0;
    }
    st.ai.config_editing = editing;
    if was_editing && !editing {
        st.cmd(Command::SaveSettings);
        if !st.settings.ai.models().is_empty() && st.ai.catalog_key != ai::catalog_key(st) {
            st.cmd(Command::Ai(Action::EnsureModels));
        }
    }
    if action(
        ui,
        "ai.config-save",
        Rect::new(area.right() - 145.0, y, 145.0, 30.0),
        "Save configuration",
        Icon::Check,
    ) {
        st.cmd(Command::SaveSettings);
        if !st.settings.ai.models().is_empty() {
            st.cmd(Command::Ai(Action::EnsureModels));
        }
    }
    if action(
        ui,
        "ai.settings-models",
        Rect::new(area.x, y, 162.0, 30.0),
        if st.ai.fetching {
            "Fetching models…"
        } else {
            "Choose model"
        },
        Icon::ListChecks,
    ) {
        st.ai.model_menu = !st.ai.model_menu || !st.ai.model_settings;
        st.ai.model_settings = true;
        st.ai.model_filters_open = false;
        st.ai.reasoning_menu = false;
        st.ai.reveal_model = true;
        st.ai.model_detail = None;
        if st.ai.models.is_empty() {
            st.cmd(Command::Ai(Action::EnsureModels));
        }
    }
    y += 42.0;
    let save = st.settings.ai.save_models;
    if super::settings::switch_row(
        ui,
        wid("ai.save-models"),
        Rect::new(area.x, y, w, 34.0),
        "Save model list",
        Some(save),
        st.accent,
    ) {
        st.settings.ai.save_models = !save;
        if save {
            st.ai.catalog_saved_at = 0;
            st.ai.catalog_from_cache = false;
        }
        st.cmd(Command::SaveSettings);
        if !save {
            st.cmd(Command::Ai(Action::EnsureModels));
        }
    }
    y += 34.0;
    text(
        ui,
        if st.settings.ai.save_models {
            "Reuse this list after restarting. Refresh it when you want newer models."
        } else {
            "Fetch once each session. Use Refresh whenever you want to update the list."
        },
        Rect::new(area.x, y, w, 32.0),
        11.0,
        palette::TEXT_DIM,
    );
    y += 37.0;
    if st.settings.ai.hackclub() {
        let hide = st.settings.ai.hide_unavailable_models;
        if super::settings::switch_row(ui, wid("ai.hide-unavailable-models"), Rect::new(area.x,y,w,34.0), "Hide unavailable models", Some(hide), st.accent) {
            st.settings.ai.hide_unavailable_models = !hide;
            st.ai.model_rows_key = None;
            st.cmd(Command::SaveSettings);
        }
        y += 34.0;
        text(ui, "Hide deprecated and unavailable models. Off shows disabled rows.", Rect::new(area.x,y,w,32.0),11.0,palette::TEXT_DIM);
        y += 37.0;
    }
    if !st.ai.error.is_empty() {
        text(
            ui,
            &st.ai.error,
            Rect::new(area.x, y, w, 58.0),
            12.0,
            palette::ORANGE,
        );
        y += 66.0;
    }
    ui.c.text("Chats stay on this PC. Temporary chats are never saved. Attachments are sent only when you press Send. Model support and charges depend on your provider.",Rect::new(area.x,y,w,55.0),&TextStyle::new(12.0).top().wrap().color(palette::TEXT_DIM));
    y += 65.0;
    if action(
        ui,
        "ai.delete-history",
        Rect::new(area.x, y, 172.0, 30.0),
        "Delete all chat history",
        Icon::Trash,
    ) {
        st.cmd(Command::Ai(Action::DeleteAll));
    }
    y + 48.0
}

pub(super) fn clip_hits(ui: &mut Ui, start: usize, view: Rect) {
    let mut n = 0;
    ui.hits.retain_mut(|h| {
        n += 1;
        if n <= start {
            return true;
        }
        let bottom = h.rect.bottom().min(view.bottom());
        h.rect.y = h.rect.y.max(view.y);
        h.rect.h = (bottom - h.rect.y).max(0.0);
        h.rect.h > 1.0
    });
}
