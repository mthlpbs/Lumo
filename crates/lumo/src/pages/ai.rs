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
pub fn height(st: &AppState) -> f32 {
    st.ai.island_height
}
pub fn next_wake(st: &AppState) -> Option<f64> {
    st.ai
        .hover_model
        .as_ref()
        .map(|(_, t)| t + 1.0)
        .filter(|t| *t > st.now)
}
pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    draw_chat(ui, st, area, false);
}
pub fn draw_chat(ui: &mut Ui, st: &mut AppState, area: Rect, full: bool) {
    ai::ensure(st);
    // A model picker is modal to chat. Paint the conversation without input
    // so dismissing the picker cannot also send, attach or change chats.
    let model_input = (st.ai.model_menu && !st.ai.model_settings)
        .then(|| std::mem::take(ui.input));
    let mut toolbar_tip: Option<(&str, Rect)> = None;
    let toolbar = Rect::new(area.x, area.y, area.w, 28.0);
    if action(
        ui,
        "ai.history",
        Rect::new(toolbar.x, toolbar.y, 85.0, 26.0),
        "History",
        Icon::Clock,
    ) {
        st.ai.history = !st.ai.history;
        st.ai.model_menu = false;
    }
    let new_rect = Rect::new(toolbar.x + 94.0, toolbar.y - 1.0, 30.0, 28.0);
    let new = ui.icon_button(
        wid("ai.new"),
        new_rect,
        Icon::NewChat,
        palette::TEXT_DIM,
        18.0,
    );
    if new.hovered {
        toolbar_tip = Some(("New chat", new_rect));
    }
    if new.clicked {
        st.cmd(Command::Ai(Action::New(false)));
    }
    let temporary_rect = Rect::new(toolbar.x + 132.0, toolbar.y - 1.0, 30.0, 28.0);
    let temporary = ui.icon_button(
        wid("ai.temporary"),
        temporary_rect,
        if st.ai.temporary {
            Icon::TemporaryChatChecked
        } else {
            Icon::TemporaryChat
        },
        if st.ai.temporary {
            palette::TEXT
        } else {
            palette::TEXT_DIM
        },
        18.0,
    );
    if temporary.hovered {
        toolbar_tip = Some((
            if st.ai.temporary {
                "Turn off temporary chat"
            } else {
                "Turn on temporary chat · not saved"
            },
            temporary_rect,
        ));
    }
    if temporary.clicked {
        st.cmd(Command::Ai(Action::ToggleTemporary));
    }
    if ui
        .icon_button(
            wid("ai.settings"),
            Rect::new(toolbar.right() - 30.0, toolbar.y, 28.0, 26.0),
            Icon::Gear,
            palette::TEXT_DIM,
            15.0,
        )
        .clicked
    {
        st.cmd(Command::OpenSettings);
    }
    if !full
        && ui
            .icon_button(
                wid("ai.open-full"),
                Rect::new(toolbar.right() - 108.0, toolbar.y, 28.0, 26.0),
                Icon::PopOut,
                palette::TEXT_DIM,
                15.0,
            )
            .clicked
    {
        st.cmd(Command::OpenAiWindow);
    }
    if !full && st.ai.max_island_height > 245.0 {
        let expanded = st.ai.island_height > 245.0;
        let r = Rect::new(toolbar.right() - 145.0, toolbar.y, 28.0, 26.0);
        let response = ui.icon_button(
            wid("ai.resize"),
            r,
            if expanded {
                Icon::CollapseUp
            } else {
                Icon::ExpandDown
            },
            palette::TEXT_DIM,
            16.0,
        );
        if response.clicked {
            st.cmd(Command::AiHeight(if expanded {
                240.0
            } else {
                st.ai.max_island_height
            }));
        }
        if response.hovered {
            toolbar_tip = Some((
                if expanded {
                    "Compact chat"
                } else {
                    "Expand chat toward the desktop middle"
                },
                r,
            ));
        }
    }
    let progress_rect = Rect::new(toolbar.right() - 72.0, toolbar.y - 2.0, 32.0, 30.0);
    let progress_tip = generation_indicator(ui, st, progress_rect);
    let composer = Rect::new(area.x, area.bottom() - 54.0, area.w, 54.0);
    let pending_h = if st.ai.pending.is_empty() { 0.0 } else { 30.0 };
    let error_h = if st.ai.error.is_empty() { 0.0 } else { 32.0 };
    let view = Rect::new(
        area.x,
        area.y + 36.0,
        area.w,
        (composer.y - area.y - 42.0 - pending_h - error_h).max(38.0),
    );
    let wheel = model_input.as_ref().map_or(ui.input.wheel, |input| input.wheel);
    if (st.ai.model_menu && !st.ai.model_settings) || st.ai.reasoning_menu || st.ai.attach_menu {
        ui.input.wheel = 0.0;
    }
    if st.ai.history {
        history(ui, st, view);
    } else {
        transcript(ui, st, view, full);
    }
    if error_h > 0.0 {
        let r = Rect::new(area.x, composer.y - pending_h - 32.0, area.w, 28.0);
        text(ui, &st.ai.error, r, 12.0, palette::ORANGE);
        if ui.hovered(r) {
            let w = area.w.min(360.0);
            let style = TextStyle::new(12.0).top().wrap();
            let h = (ui.c.measure(&st.ai.error, &style, w - 24.0).1 + 24.0).min(110.0);
            tooltip(ui, &st.ai.error, Rect::new(r.x, r.y - h - 5.0, w, h));
        }
    }
    let mut remove = None;
    let mut x = area.x;
    for (i, a) in st.ai.pending.iter().enumerate() {
        let r = Rect::new(x, composer.y - 29.0, 150.0f32.min(area.right() - x), 24.0);
        if r.w < 40.0 {
            break;
        }
        ui.c.fill_rrect(r, 7.0, Color::white(0.1));
        text(
            ui,
            &a.name,
            Rect::new(r.x + 8.0, r.y, r.w - 31.0, r.h),
            11.0,
            palette::TEXT_DIM,
        );
        if ui
            .icon_button(
                wid_n("ai.remove-attachment", i as u64),
                Rect::new(r.right() - 24.0, r.y, 24.0, 24.0),
                Icon::Close,
                palette::TEXT_DIM,
                12.0,
            )
            .clicked
        {
            remove = Some(i);
        }
        x += r.w + 6.0;
    }
    if let Some(i) = remove {
        st.ai.pending.remove(i);
    }
    ui.c.fill_rrect(composer, 27.0, Color::white(0.10));
    if ui
        .icon_button(
            wid("ai.plus"),
            Rect::new(composer.x + 7.0, composer.y + 9.0, 36.0, 36.0),
            if st.ai.attaching {
                Icon::Clock
            } else {
                Icon::Plus
            },
            palette::TEXT_DIM,
            19.0,
        )
        .clicked
    {
        let open = !st.ai.attach_menu && !st.ai.attaching;
        st.ai.close_popovers();
        st.ai.attach_menu = open;
    }
    let m = st.ai.model(&st.settings.ai);
    let model_label = if m.id.is_empty() { "Choose model" } else { m.display_name() };
    // Text layouts are cached by the renderer. Fit short names without
    // taking more than 104 DIPs away from the message field for long ones.
    let model_w = (ui.c.measure(model_label, &TextStyle::new(11.0), 10000.0).0 + 20.0)
        .clamp(60.0, 104.0);
    let model_r = Rect::new(
        composer.right() - model_w - 54.0,
        composer.y + 11.0,
        model_w,
        32.0,
    );
    let input_r = Rect::new(
        composer.x + 46.0,
        composer.y + 6.0,
        (model_r.x - composer.x - 54.0).max(40.0),
        42.0,
    );
    let input = ui.text_area(
        wid("ai.prompt"),
        input_r,
        &mut st.ai.draft,
        if st.ai.attaching {
            "Adding attachment…"
        } else {
            "Ask anything…"
        },
        12000,
    );
    if input.focused {
        st.cmd(Command::FocusText);
    }
    let resp = ui.button(wid("ai.models"), model_r);
    ui.c.fill_rrect(
        model_r,
        16.0,
        Color::white(if resp.hovered { 0.13 } else { 0.06 }),
    );
    text(
        ui,
        model_label,
        Rect::new(model_r.x + 10.0, model_r.y, model_r.w - 20.0, model_r.h),
        11.0,
        palette::TEXT_DIM,
    );
    if resp.clicked {
        let was_open = st.ai.reasoning_menu || (st.ai.model_menu && !st.ai.model_settings);
        st.ai.model_filters_open = false;
        st.ai.reasoning_menu = false;
        st.ai.model_menu = !was_open;
        st.ai.model_settings = false;
        st.ai.model_detail = None;
        st.ai.hover_model = None;
        st.ai.attach_menu = false;
        st.ai.model_scroll = 0.0;
        st.ai.reveal_model = true;
        st.ai.search.clear();
        if !was_open {
            if st.ai.models.is_empty() {
                st.cmd(Command::Ai(Action::EnsureModels));
            }
            ui.input.focused = Some(wid("ai.model-search"));
            ui.input.caret = 0;
            st.cmd(Command::FocusText);
        }
    }
    let send = Rect::new(composer.right() - 45.0, composer.y + 8.0, 38.0, 38.0);
    let r = ui.button(wid("ai.send"), send);
    ui.c.fill_circle(
        send.center(),
        19.0,
        Color::white(if r.pressed { 0.7 } else { 0.96 }),
    );
    ui.c.icon(
        if st.ai.busy { Icon::Stop } else { Icon::Send },
        send.inset(10.0, 10.0),
        Color::rgba(0.0, 0.0, 0.0, 1.0),
    );
    if r.clicked || input.submitted {
        st.cmd(Command::Ai(if st.ai.busy {
            Action::Stop
        } else {
            Action::Send
        }));
    }
    if ui.hovered(send) {
        tooltip(
            ui,
            if st.ai.busy {
                "Stop waiting for this reply"
            } else {
                "Send message · Enter\nShift+Enter for a new line"
            },
            Rect::new(area.x, composer.y - 65.0, area.w, 50.0),
        );
    }
    if !full && st.ai.max_island_height > 245.0 {
        let grip = Rect::new(area.center().x - 30.0, area.bottom() + 3.0, 60.0, 12.0);
        let response = ui.button(wid("ai.resize-grip"), grip);
        ui.c.fill_rrect(
            Rect::new(grip.center().x - 16.0, grip.y + 5.0, 32.0, 3.0),
            1.5,
            Color::white(if response.hovered || response.pressed {
                0.5
            } else {
                0.2
            }),
        );
        if response.pressed {
            if ui.input.just_pressed {
                st.ai.resize_from = st.ai.island_height;
                st.ai.resize_dragged = false;
            }
            if let Some((_, dy)) = response.drag {
                if dy.abs() > 3.0 {
                    st.ai.resize_dragged = true;
                    st.cmd(Command::AiHeight(
                        (st.ai.resize_from + dy).clamp(240.0, st.ai.max_island_height),
                    ));
                }
            }
        }
        if response.clicked && !st.ai.resize_dragged {
            st.cmd(Command::AiHeight(if st.ai.island_height > 245.0 {
                240.0
            } else {
                st.ai.max_island_height
            }));
        }
    }

    if let Some(input) = model_input { *ui.input = input; }
    if st.ai.attach_menu {
        attach_menu(
            ui,
            st,
            Rect::new(
                area.x,
                (composer.y - 188.0).max(area.y),
                area.w.min(320.0),
                180.0,
            ),
        );
    }
    if st.ai.model_menu && !st.ai.model_settings && (full || !picker_below(st)) {
        ui.input.wheel = wheel;
        let w = area.w.min(460.0);
        model_picker(
            ui,
            st,
            Rect::new(area.right() - w, area.y + 1.0, w, composer.y - area.y - 8.0),
            false,
        );
    }
    if picker_below(st) { ui.input.wheel = wheel; }
    // Labels paint last so response text cannot bleed through them.
    if !st.ai.model_menu && !st.ai.reasoning_menu && !st.ai.attach_menu {
        if let Some(label) = progress_tip {
            let w = label
                .lines()
                .map(|line| ui.c.measure(line, &TextStyle::new(12.0), 280.0).0)
                .fold(0.0f32, f32::max)
                + 24.0;
            let w = w.clamp(140.0, 280.0);
            tooltip(
                ui,
                &label,
                Rect::new(area.right() - w, toolbar.bottom() + 5.0, w, 70.0),
            );
        } else if let Some((label, button)) = toolbar_tip {
            let w = (ui.c.measure(label, &TextStyle::new(12.0), 240.0).0 + 24.0)
                .clamp(100.0, 240.0)
                .min(area.w);
            let x = button.x.min(area.right() - w).max(area.x);
            tooltip(ui, label, Rect::new(x, toolbar.bottom() + 5.0, w, 54.0));
        }
    }
}
fn generation_indicator(ui: &mut Ui, st: &mut AppState, r: Rect) -> Option<String> {
    use ai::Generation;
    let state = if st.ai.busy {
        Generation::Working
    } else {
        st.ai.generation
    };
    let (icon, color) = match state {
        Generation::Working => (Icon::Stop, BLUE),
        Generation::Ready => (Icon::Check, BLUE),
        Generation::Failed => (Icon::Close, palette::ORANGE),
        Generation::Stopped => (Icon::Stop, palette::TEXT_DIM),
        Generation::Idle => (Icon::Minus, palette::TEXT_DIM),
    };
    let response = ui.button(wid("ai.generation"), r);
    if response.hovered {
        ui.c.fill_rrect(r, 15.0, Color::white(0.06));
    }
    ui.c.stroke_circle(r.center(), 12.0, Color::white(0.16), 2.0);
    if state == Generation::Working {
        let phase = if st.power.saver {
            0.0
        } else {
            (ui.now * 0.75).rem_euclid(1.0) as f32
        };
        ui.c.arc(r.center(), 12.0, phase, 0.27, BLUE, 2.0);
        // Activity, not a fabricated completion percentage. Visible only;
        // App pauses live paints for lock/display-off/battery saver.
        if !st.power.saver {
            ui.request_live();
        }
    } else if state != Generation::Idle {
        ui.c.stroke_circle(r.center(), 12.0, color.mul_alpha(0.65), 2.0);
    }
    ui.c.icon(icon, Rect::from_center(r.center(), 11.0, 11.0), color);
    let other = st.ai.running_chat != st.ai.current.id;
    let can_open = other
        && state != Generation::Idle
        && state != Generation::Stopped
        && st.ai.chats.iter().any(|c| c.id == st.ai.running_chat);
    if response.clicked {
        if can_open {
            st.cmd(Command::Ai(Action::Select(st.ai.running_chat)));
        } else if state == Generation::Working {
            st.cmd(Command::Ai(Action::Stop));
        } else if state == Generation::Ready {
            st.ai.history = false;
            st.ai.scroll_to_reply = st
                .ai
                .current
                .messages
                .iter()
                .rposition(|m| m.role == "assistant");
        }
    }
    if !response.hovered {
        return None;
    }
    let end = if st.ai.busy {
        st.now
    } else {
        st.ai.generation_finished
    };
    let seconds = (end - st.ai.generation_started).max(0.0) as u64;
    let elapsed = if st.ai.generation_started > 0.0 {
        format!(" · {}:{:02}", seconds / 60, seconds % 60)
    } else {
        String::new()
    };
    Some(match state {
        Generation::Working => format!(
            "Generating a reply{elapsed}\n{}",
            if can_open {
                "Click to open the active chat"
            } else {
                "Click to stop generation"
            }
        ),
        Generation::Ready => format!("Reply ready{elapsed}\nClick to read the reply"),
        Generation::Stopped => format!("Generation stopped{elapsed}"),
        Generation::Failed => {
            format!("Reply failed{elapsed}\nSee the error above the message field")
        }
        Generation::Idle => "Ready for your next message".into(),
    })
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
        ui.c.icon(
            Icon::Sparkles,
            Rect::new(view.x + 4.0, view.y + 12.0, 24.0, 24.0),
            BLUE,
        );
        text(
            ui,
            "A little help, right here.",
            Rect::new(view.x + 40.0, view.y + 6.0, view.w - 42.0, 32.0),
            19.0,
            palette::TEXT,
        );
        text(
            ui,
            if st.ai.temporary {
                "Temporary chat · This conversation won't be saved."
            } else if st.settings.ai.default_model.is_empty() {
                "Add your provider, key and model in Settings → AI companion."
            } else {
                "Ask a question, attach a PDF, or capture part of your screen."
            },
            Rect::new(view.x + 40.0, view.y + 43.0, view.w - 44.0, 48.0),
            13.0,
            palette::TEXT_DIM,
        );
        return;
    }
    let first = if full {
        0
    } else {
        chat.messages
            .len()
            .saturating_sub(if st.ai.busy { 1 } else { 2 })
    };
    let size = if full { 16.0 } else { 15.0 };
    let style = TextStyle::new(size).top().wrap().line_spacing(1.4);
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
        let markdown = if user {
            None
        } else {
            Some(super::ai_markdown::layout(ui, &m.text, view.w - 8.0, size))
        };
        let text_h = markdown
            .as_ref()
            .map_or_else(
                || ui.c.measure(&content, &style, view.w * 0.83 - 28.0).1,
                |m| m.height,
            )
            .max(21.0);
        let image_h = if user {
            0.0
        } else {
            m.attachments
                .iter()
                .filter(|a| a.mime.starts_with("image/"))
                .count() as f32
                * if full { 230.0 } else { 110.0 }
        };
        let h = text_h + if user { 24.0 } else { 40.0 } + image_h;
        if st.ai.scroll_to_reply == Some(i) {
            anchor = Some(total);
        }
        messages.push((i, h, text_h, content, markdown));
        total += h + 12.0;
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
    for (i, h, text_h, content, markdown) in messages {
        let m = &chat.messages[i];
        let user = m.role == "user";
        let w = if user { view.w * 0.83 } else { view.w };
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
                ui.c.fill_rrect(r, 18.0, Color::white(0.14));
                ui.c.text(
                    &content,
                    Rect::new(r.x + 14.0, r.y + 10.0, r.w - 28.0, text_h + 2.0),
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
                        if full { 220.0 } else { 100.0 },
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
        y += h + 12.0;
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
    let fixed_hits = ui.hits.len();
    let mut commands = vec![];
    let wheel = ui.wheel_in(view);
    st.ai.scroll = (st.ai.scroll - wheel * 40.0).clamp(
        0.0,
        (st.ai.chats.len() as f32 * 43.0 + 32.0 - view.h).max(0.0),
    );
    ui.c.clip(view);
    text(
        ui,
        "Saved on this PC · encrypted for your Windows account",
        Rect::new(view.x, view.y - st.ai.scroll, view.w, 24.0),
        12.0,
        palette::TEXT_DIM,
    );
    for (i, c) in st.ai.chats.iter().enumerate() {
        let r = Rect::new(
            view.x,
            view.y + 30.0 + i as f32 * 43.0 - st.ai.scroll,
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
    if st.ai.chats.is_empty() {
        text(
            ui,
            "No saved chats yet",
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
            "PDF documents",
            "PDF files only",
            Icon::File,
            Action::Attach(false),
        ),
        ("Capture screen", "", Icon::ScreenSnip, Action::Snip),
        ("Paste image", "Ctrl+V", Icon::Clipboard, Action::ReadSnip),
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
        } else if i == 3 {
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
pub fn tooltip(ui: &mut Ui, content: &str, area: Rect) {
    let w = area.w.min(480.0).max(100.0);
    let style = TextStyle::new(12.0).top().wrap();
    let (_, h) = ui.c.measure(content, &style, w - 24.0);
    let h = (h + 24.0).min(area.h.max(60.0));
    let r = Rect::new(area.right() - w, area.y, w, h);
    ui.c.fill_rrect(r, 10.0, Color::rgba(0.18, 0.18, 0.19, 1.0));
    ui.c.stroke_rrect(r, 10.0, Color::white(0.13), 1.0);
    ui.c.text(content, r.inset(12.0, 10.0), &style.color(palette::TEXT));
}
pub fn model_picker(ui: &mut Ui, st: &mut AppState, r: Rect, settings: bool) {
    super::ai_models::picker(ui, st, r, settings);
}

/// A compact chat keeps its composer visible; its model list is drawn by
/// the island host below the card, outside the chat's content clip.
pub fn picker_below(st: &AppState) -> bool {
    st.ai.model_menu && !st.ai.model_settings && !st.ai.full_open && st.ai.island_height <= 245.0
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
