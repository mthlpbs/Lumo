//! Native model picker and capability filters. No workers, polls, or idle animation.
use crate::{
    ai::{Action, ModelRow},
    commands::Command,
    gfx::{TextStyle, Weight},
    icons::Icon,
    state::AppState,
    ui::{wid, wid_n, Ui},
};
use lumo_core::{
    ai::{Kind, Model},
    ai_reasoning,
    geom::{palette, Color, Point, Rect},
};
const BLUE: Color = Color::rgba(0.52, 0.72, 1.0, 1.0);
const YELLOW: Color = Color::rgba(1.0, 0.78, 0.22, 1.0);
const VIOLET: Color = Color::rgba(0.75, 0.60, 1.0, 1.0);
const FREE: u8 = 1;
const IMAGES: u8 = 2;
const FILES: u8 = 4;
const GENERATE: u8 = 8;
fn matches_filters(m: &Model, mask: u8) -> bool {
    (mask & FREE == 0 || m.free == Some(true))
        && (mask & IMAGES == 0 || m.inputs.iter().any(|v| v == "image"))
        && (mask & FILES == 0 || m.inputs.iter().any(|v| v == "file"))
        && (mask & GENERATE == 0 || m.outputs.iter().any(|v| v == "image"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_filters_combine_with_search_and_keep_selections() {
        let mut st = AppState::new(Default::default(), Default::default());
        st.ai.models = lumo_core::ai::parse_models(&serde_json::json!({"data":[
            {"id":"p/plain","name":"Plain model"},
            {"id":"p/vision:free","name":"Vision model", "architecture":{"input_modalities":["text","image","file"],"output_modalities":["text"]}},
            {"id":"p/image","name":"Image studio", "architecture":{"input_modalities":["text","image"],"output_modalities":["image"]}}
        ]})).unwrap();
        let count = |st: &AppState| {
            st.ai
                .model_rows
                .iter()
                .filter(|row| matches!(row, ModelRow::Model(_)))
                .count()
        };
        for (mask, expected) in [
            (FREE, 1),
            (IMAGES, 2),
            (FILES, 1),
            (GENERATE, 1),
            (FREE | IMAGES | FILES, 1),
            (FREE | GENERATE, 0),
        ] {
            st.ai.model_filters = mask;
            rows(&mut st);
            assert_eq!(count(&st), expected, "mask {mask}");
        }
        st.ai.model_filters = FREE | IMAGES;
        st.ai.search = "Vision".into();
        rows(&mut st);
        assert_eq!(count(&st), 1);
        st.ai.search = "Plain".into();
        rows(&mut st);
        assert_eq!(count(&st), 0);
        st.ai.model_filters_open = true;
        st.ai.close_models();
        assert!(!st.ai.model_filters_open);
        assert_eq!(st.ai.model_filters, FREE | IMAGES);
    }
}
fn text(ui: &mut Ui, s: &str, r: Rect, size: f32, col: Color) {
    ui.c.text(s, r, &TextStyle::new(size).color(col));
}
fn surface(ui: &mut Ui, r: Rect) {
    ui.c.fill_rrect(r, 14.0, Color::rgba(0.12, 0.12, 0.13, 1.0));
    ui.c.stroke_rrect(r, 14.0, Color::white(0.15), 1.0);
}
fn close(ui: &mut Ui, r: Rect) -> bool {
    ui.icon_button(
        wid("ai.model-close"),
        Rect::new(r.right() - 37.0, r.y + 7.0, 30.0, 28.0),
        Icon::Close,
        palette::TEXT_DIM,
        14.0,
    )
    .clicked
}
fn selected(ui: &mut Ui, st: &mut AppState, id: String, settings: bool) {
    let adjustable = st
        .ai
        .models
        .iter()
        .any(|m| m.id == id && m.reasoning.adjustable());
    let current = if settings {
        st.settings.ai.default_model.clone()
    } else {
        st.ai.model(&st.settings.ai).id
    };
    let edit_effort = adjustable && current == id;
    // Switching models always starts with its own default. Re-selecting the
    // active starred row is the deliberate action that edits its effort.
    if adjustable && !edit_effort {
        remember_effort(st, &id, None, true);
    }
    st.ai.model_filters_open = false;
    ui.request_repaint();
    if settings {
        st.settings.ai.default_model = id.clone();
        st.cmd(Command::SaveSettings);
    }
    st.ai.selected = id;
    st.ai.model_menu = edit_effort;
    st.ai.reasoning_menu = edit_effort;
    st.ai.model_detail = None;
    st.ai.model_detail_scroll = 0.0;
    st.ai.hover_model = None;
    if edit_effort {
        ui.input.focused = Some(wid("ai.effort-slider"));
        if !settings {
            st.cmd(Command::FocusText);
        }
    } else if !settings {
        ui.input.focused = Some(wid("ai.prompt"));
        ui.input.caret = st.ai.draft.chars().count();
        st.cmd(Command::FocusText);
    } else {
        ui.input.focused = None;
    }
}

fn remember_effort(st: &mut AppState, id: &str, value: Option<String>, commit: bool) {
    if let Some(value) = value {
        if st.settings.ai.reasoning_efforts.len() >= 128
            && !st.settings.ai.reasoning_efforts.contains_key(id)
        {
            if let Some(first) = st.settings.ai.reasoning_efforts.keys().next().cloned() {
                st.settings.ai.reasoning_efforts.remove(&first);
            }
        }
        st.settings.ai.reasoning_efforts.insert(id.into(), value);
    } else {
        st.settings.ai.reasoning_efforts.remove(id);
    }
    if commit {
        st.cmd(Command::SaveSettings);
    }
}
fn effort(ui: &mut Ui, st: &mut AppState, r: Rect, settings: bool) {
    let m = st.ai.model(&st.settings.ai);
    if !m.reasoning.adjustable() {
        st.ai.reasoning_menu = false;
        st.ai.model_menu = true;
        ui.request_repaint();
        return;
    }
    let value = st
        .settings
        .ai
        .reasoning_efforts
        .get(&m.id)
        .filter(|v| m.reasoning.levels.contains(v))
        .or(m.reasoning.default_effort.as_ref());
    let index = value.and_then(|v| m.reasoning.levels.iter().position(|x| x == v));
    let label = value
        .map(|s| ai_reasoning::label(s))
        .unwrap_or_else(|| "Choose effort".into());
    ui.c.text(
        &label,
        Rect::new(r.x + 36.0, r.y + 9.0, r.w - 72.0, 25.0),
        &TextStyle::new(16.0)
            .weight(Weight::Semibold)
            .center()
            .color(BLUE),
    );
    let reset = ui.icon_button(
        wid("ai.effort-reset"),
        Rect::new(r.right() - 37.0, r.y + 8.0, 28.0, 28.0),
        Icon::Reset,
        palette::TEXT_DIM,
        15.0,
    );
    if reset.clicked {
        remember_effort(st, &m.id, None, true);
        ui.request_repaint();
    }
    let model_r = Rect::new(r.x + 32.0, r.y + 35.0, r.w - 64.0, 24.0);
    let change = ui.button(wid("ai.effort-model"), model_r);
    ui.c.text(
        &format!("{}  ›", m.display_name()),
        model_r,
        &TextStyle::new(11.0).center().color(if change.hovered {
            palette::TEXT
        } else {
            palette::TEXT_DIM
        }),
    );
    if change.clicked {
        st.ai.model_filters_open = false;
        ui.request_repaint();
        st.ai.reasoning_menu = false;
        st.ai.model_menu = true;
        st.ai.model_detail = None;
        st.ai.model_scroll = 0.0;
        st.ai.search.clear();
        ui.input.focused = Some(wid("ai.model-search"));
        ui.input.caret = 0;
        st.ai.reveal_model = true;
        if !settings {
            st.cmd(Command::FocusText);
        }
        return;
    }
    let track = Rect::new(r.x + 17.0, r.y + 77.0, r.w - 34.0, 24.0);
    let response = ui.button(wid("ai.effort-slider"), track.inset(0.0, -5.0));
    ui.c.fill_rrect(track, 12.0, Color::white(0.09));
    let x0 = track.x + 12.0;
    let length = track.w - 24.0;
    let count = m.reasoning.levels.len();
    let position = |i: usize| x0 + length * i as f32 / (count - 1) as f32;
    if ui.input.focused == Some(wid("ai.effort-slider")) {
        let mut next = index.unwrap_or(0);
        let mut navigate = false;
        let keys = std::mem::take(&mut ui.input.keys);
        for key in &keys {
            use crate::ui::KeyInput;
            match key {
                KeyInput::Left => {
                    next = next.saturating_sub(1);
                    navigate = true;
                }
                KeyInput::Right => {
                    next = (next + 1).min(count - 1);
                    navigate = true;
                }
                KeyInput::Home => {
                    next = 0;
                    navigate = true;
                }
                KeyInput::End => {
                    next = count - 1;
                    navigate = true;
                }
                _ => {}
            }
        }
        if navigate && index != Some(next) {
            remember_effort(st, &m.id, Some(m.reasoning.levels[next].clone()), true);
            ui.request_repaint();
        }
    }
    if let Some(i) = index {
        ui.c.fill_rrect(
            Rect::new(track.x, track.y, position(i) - track.x + 10.0, track.h),
            12.0,
            BLUE,
        );
    }
    for i in 0..count {
        ui.c.fill_circle(
            Point::new(position(i), track.center().y),
            2.0,
            Color::white(0.32),
        );
    }
    if let Some(i) = index {
        ui.c.fill_circle(
            Point::new(position(i), track.center().y),
            12.0,
            palette::TEXT,
        );
    }
    let mouse = ui
        .input
        .released
        .filter(|(id, _)| *id == wid("ai.effort-slider"))
        .map(|(_, p)| p)
        .or_else(|| {
            if response.pressed {
                ui.input.mouse
            } else {
                None
            }
        });
    if let Some(p) = mouse {
        let i = (((p.x - x0) / length).clamp(0.0, 1.0) * (count - 1) as f32).round() as usize;
        if index != Some(i) {
            remember_effort(st, &m.id, Some(m.reasoning.levels[i].clone()), false);
            ui.request_repaint();
        }
    }
    if ui
        .input
        .released
        .is_some_and(|(id, _)| id == wid("ai.effort-slider"))
    {
        st.cmd(Command::SaveSettings);
    }
    if response.hovered || reset.hovered {
        let hint = if reset.hovered {
            "Reset to the model's own setting"
        } else {
            "Higher effort can take longer and use more tokens"
        };
        text(
            ui,
            hint,
            Rect::new(r.x + 12.0, r.y + 107.0, r.w - 24.0, 22.0),
            10.0,
            palette::TEXT_DIM,
        );
    }
}
fn rows(st: &mut AppState) {
    let key = (
        st.ai.catalog_revision,
        st.ai.models.len(),
        st.ai.search.to_lowercase(),
        st.ai.model_filters,
    );
    if st.ai.model_rows_key.as_ref() == Some(&key) {
        return;
    }
    let q = &key.2;
    let mut indices: Vec<_> = st
        .ai
        .models
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            if !matches_filters(m, key.3) {
                return false;
            }
            if q == "free" {
                return m.free == Some(true);
            }
            q.is_empty()
                || [&m.id, &m.name, &m.provider, &m.description]
                    .iter()
                    .any(|s| s.to_lowercase().contains(q))
                || m.capabilities.iter().any(|s| s.to_lowercase().contains(q))
                || m.provider_name().to_lowercase().contains(q)
                || m.kind.name().to_lowercase().contains(q)
                || m.cost_group().to_lowercase().contains(q)
                || m.inputs
                    .iter()
                    .chain(&m.outputs)
                    .any(|s| s.to_lowercase().contains(q))
                || (q == "reasoning" && (m.reasoning.mandatory || !m.reasoning.levels.is_empty()))
        })
        .map(|(i, _)| i)
        .collect();
    indices.sort_by(|a, b| {
        let (a, b) = (&st.ai.models[*a], &st.ai.models[*b]);
        (a.provider_name(), a.kind, a.cost_group(), &a.name).cmp(&(
            b.provider_name(),
            b.kind,
            b.cost_group(),
            &b.name,
        ))
    });
    let mut result = vec![];
    let mut last = String::new();
    for i in indices {
        let m = &st.ai.models[i];
        if m.provider_name() != last {
            last = m.provider_name().into();
            result.push(ModelRow::Provider(last.clone()));
        }
        result.push(ModelRow::Model(i));
    }
    st.ai.model_rows = result;
    st.ai.model_rows_key = Some(key);
}
/// Effort needs only its supported choices, rather than the catalog's full
/// browsing height. Share these bounds with the island's input mask.
pub fn popup_rect(st: &AppState, r: Rect) -> Rect {
    if !st.ai.reasoning_menu {
        return r;
    }
    let width = r.w.min(280.0);
    Rect::new(r.right() - width, r.y, width, r.h.min(132.0))
}

pub fn picker(ui: &mut Ui, st: &mut AppState, r: Rect, settings: bool) {
    let r = popup_rect(st, r);
    if ui.input.just_pressed && ui.input.mouse.is_some_and(|p| !r.contains(p)) {
        st.ai.close_models();
        ui.blur();
        ui.input.pressed_id = None;
        ui.input.clicked = None;
        ui.input.down = false;
        ui.request_repaint();
        return;
    }
    ui.button(wid("ai.model-surface"), r);
    surface(ui, r);
    if st.ai.reasoning_menu {
        effort(ui, st, r, settings);
        return;
    }
    if st.ai.model_detail.is_some() {
        details(ui, st, r, settings);
        return;
    }
    text(
        ui,
        "Select model",
        Rect::new(r.x + 14.0, r.y + 6.0, r.w - 148.0, 28.0),
        13.0,
        palette::TEXT,
    );
    let filter_r = Rect::new(r.right() - 77.0, r.y + 6.0, 32.0, 30.0);
    let filter = ui.button(wid("ai.model-filters"), filter_r);
    let filtered = st.ai.model_filters != 0;
    ui.c.fill_rrect(
        filter_r,
        8.0,
        if filtered || st.ai.model_filters_open {
            BLUE.mul_alpha(0.18)
        } else {
            Color::white(if filter.hovered { 0.10 } else { 0.045 })
        },
    );
    ui.c.icon(
        Icon::Sliders,
        filter_r.inset(8.0, 7.0),
        if filtered || st.ai.model_filters_open {
            BLUE
        } else {
            palette::TEXT_DIM
        },
    );
    if filtered {
        let badge = Rect::new(filter_r.right() - 10.0, filter_r.y - 1.0, 15.0, 15.0);
        ui.c.fill_rrect(badge, 7.5, BLUE);
        ui.c.text(
            &st.ai.model_filters.count_ones().to_string(),
            badge,
            &TextStyle::new(9.0).center().color(Color::black(1.0)),
        );
    }
    if filter.clicked {
        st.ai.model_filters_open = !st.ai.model_filters_open;
        st.ai.hover_model = None;
        ui.blur();
        ui.request_repaint();
    }
    if close(ui, r) {
        st.ai.close_models();
        ui.input.focused = if settings {
            None
        } else {
            Some(wid("ai.prompt"))
        };
        ui.input.caret = st.ai.draft.chars().count();
        ui.request_repaint();
        return;
    }
    if st.ai.model_filters_open {
        filters(ui, st, r);
        return;
    }
    let search = Rect::new(r.x + 9.0, r.y + 37.0, r.w - 54.0, 30.0);
    let response = ui.text_input(
        wid("ai.model-search"),
        search,
        &mut st.ai.search,
        "Search models or providers…",
        120,
    );
    if response.focused {
        st.cmd(Command::FocusText);
    }
    if response.changed {
        st.ai.model_scroll = 0.0;
        st.ai.hover_model = None;
    }
    if ui
        .icon_button(
            wid("ai.model-refresh"),
            Rect::new(r.right() - 38.0, r.y + 37.0, 29.0, 30.0),
            Icon::Reset,
            palette::TEXT_DIM,
            14.0,
        )
        .clicked
    {
        st.cmd(Command::Ai(Action::FetchModels));
    }
    rows(st);
    let view = Rect::new(r.x + 8.0, r.y + 76.0, r.w - 16.0, (r.h - 84.0).max(24.0));
    const ROW_H: f32 = 40.0;
    let total = st.ai.model_rows.len() as f32 * ROW_H;
    let max = (total - view.h).max(0.0);
    if st.ai.reveal_model && !st.ai.model_rows.is_empty() {
        st.ai.reveal_model = false;
        let active = if settings || st.ai.selected.is_empty() {
            &st.settings.ai.default_model
        } else {
            &st.ai.selected
        };
        if let Some(index) = st
            .ai
            .model_rows
            .iter()
            .position(|row| matches!(row, ModelRow::Model(i) if &st.ai.models[*i].id == active))
        {
            st.ai.model_scroll = ((index as f32 - 1.0).max(0.0) * ROW_H).min(max);
        }
    }
    st.ai.model_scroll = (st.ai.model_scroll - ui.wheel_in(view) * 40.0).clamp(0.0, max);
    let mut choose = None;
    let mut info = None;
    let mut preview = None;
    let mut hovered = false;
    let fixed = ui.hits.len();
    ui.c.clip(view);
    if st.ai.model_rows.is_empty() {
        text(
            ui,
            if st.ai.fetching {
                "Fetching models…"
            } else if st.ai.models.is_empty() {
                "Fetch models or add a model ID in Settings."
            } else {
                "No matching models"
            },
            view,
            12.0,
            palette::TEXT_DIM,
        );
    }
    for (row_index, row) in st.ai.model_rows.iter().enumerate() {
        let rr = Rect::new(
            view.x,
            view.y + row_index as f32 * ROW_H - st.ai.model_scroll,
            view.w - 6.0,
            ROW_H - 2.0,
        );
        if rr.bottom() < view.y || rr.y >= view.bottom() {
            continue;
        }
        match row {
            ModelRow::Provider(provider) => {
                ui.c.line(
                    Point::new(rr.x + 8.0, rr.y + 2.0),
                    Point::new(rr.right() - 8.0, rr.y + 2.0),
                    Color::white(0.07),
                    1.0,
                );
                text(ui, provider, rr.inset(8.0, 0.0), 11.0, palette::TEXT_DIM);
            }
            ModelRow::Model(i) => {
                let m = &st.ai.models[*i];
                let response = ui.button(wid_n("ai.model", *i as u64), rr);
                let active = if settings || st.ai.selected.is_empty() {
                    &st.settings.ai.default_model
                } else {
                    &st.ai.selected
                };
                if response.hovered || active == &m.id {
                    ui.c.fill_rrect(
                        rr,
                        8.0,
                        if active == &m.id {
                            BLUE.mul_alpha(0.16)
                        } else {
                            Color::white(0.065)
                        },
                    );
                }
                let name = Rect::new(
                    rr.x + 8.0,
                    rr.y,
                    rr.w - if active == &m.id { 130.0 } else { 54.0 },
                    if m.kind == Kind::Language { rr.h } else { 19.0 },
                );
                let marker_w = if m.free == Some(true) { 12.0 } else { 0.0 }
                    + if m.reasoning.adjustable() { 16.0 } else { 0.0 };
                text(
                    ui,
                    m.display_name(),
                    Rect::new(name.x, name.y, name.w - marker_w, name.h),
                    12.0,
                    if active == &m.id { BLUE } else { palette::TEXT },
                );
                if marker_w > 0.0 {
                    let width =
                        ui.c.measure(m.display_name(), &TextStyle::new(12.0), 10000.0)
                            .0;
                    let mut marker_x = name.x + width.min((name.w - marker_w).max(0.0));
                    if m.free == Some(true) {
                        ui.c.fill_circle(Point::new(marker_x + 6.0, name.center().y), 2.7, YELLOW);
                        marker_x += 12.0;
                    }
                    if m.reasoning.adjustable() {
                        ui.c.icon(
                            Icon::Star,
                            Rect::new(marker_x + 2.0, name.center().y - 6.0, 12.0, 12.0),
                            VIOLET,
                        );
                    }
                }
                if m.kind != Kind::Language {
                    text(
                        ui,
                        if m.kind == Kind::Image {
                            "Image generation"
                        } else {
                            "Embeddings"
                        },
                        Rect::new(name.x, rr.y + 18.0, name.w, 12.0),
                        9.0,
                        palette::TEXT_FAINT,
                    );
                }
                if active == &m.id {
                    ui.c.fill_rrect(Rect::new(rr.x, rr.y + 5.0, 3.0, rr.h - 10.0), 1.5, BLUE);
                    ui.c.icon(
                        Icon::Check,
                        Rect::new(rr.right() - 99.0, rr.center().y - 6.0, 13.0, 13.0),
                        BLUE,
                    );
                    text(
                        ui,
                        "Selected",
                        Rect::new(rr.right() - 82.0, rr.y, 52.0, rr.h),
                        10.0,
                        BLUE,
                    );
                }
                let ir = Rect::new(rr.right() - 27.0, rr.center().y - 14.0, 25.0, 28.0);
                let irsp = ui.button(wid_n("ai.model-info", *i as u64), ir);
                if irsp.hovered {
                    ui.c.fill_rrect(ir, 9.0, Color::white(0.12));
                }
                ui.c.stroke_circle(ir.center(), 6.0, Color::white(0.4), 1.0);
                ui.c.text(
                    "i",
                    ir,
                    &TextStyle::new(10.0).center().color(palette::TEXT_DIM),
                );
                if irsp.clicked && !st.ai.model_filters_open {
                    info = Some(m.id.clone());
                }
                if response.clicked && !st.ai.model_filters_open {
                    choose = Some(m.id.clone());
                }
                if response.hovered && !irsp.hovered && !st.ai.model_filters_open {
                    hovered = true;
                    if st.ai.hover_model.as_ref().is_none_or(|(id, _)| id != &m.id) {
                        st.ai.hover_model = Some((m.id.clone(), ui.now));
                    }
                    if st
                        .ai
                        .hover_model
                        .as_ref()
                        .is_some_and(|(_, t)| ui.now - t >= 1.0)
                    {
                        preview = Some((*i, rr));
                    }
                }
            }
        }
    }
    ui.c.unclip();
    super::ai::clip_hits(ui, fixed, view);
    scrollbar(
        ui,
        wid("ai.model-scrollbar"),
        view,
        total,
        &mut st.ai.model_scroll,
    );
    if !hovered {
        st.ai.hover_model = None;
    }
    if let Some(id) = choose {
        selected(ui, st, id, settings);
    }
    if let Some(id) = info {
        ui.request_repaint();
        st.ai.model_detail = Some(id);
        st.ai.model_detail_scroll = 0.0;
        st.ai.hover_model = None;
    }
    if let Some((i, rr)) = preview {
        preview_card(ui, &st.ai.models[i], r, rr);
    }
}
fn filters(ui: &mut Ui, st: &mut AppState, r: Rect) {
    let panel = Rect::new(r.x + 12.0, r.y + 43.0, r.w - 24.0, r.h - 55.0);
    text(
        ui,
        "Match all selected filters",
        Rect::new(panel.x + 4.0, panel.y, panel.w, 24.0),
        12.0,
        palette::TEXT_DIM,
    );
    let bottom = panel.bottom() - 34.0;
    let row_h = ((bottom - panel.y - 30.0) / 4.0).min(56.0).max(30.0);
    for (i, (mask, label, hint, icon)) in [
        (
            FREE,
            "Free models",
            "No input or output charge",
            Icon::Circle,
        ),
        (
            IMAGES,
            "Accepts images",
            "Ask questions about a picture",
            Icon::Image,
        ),
        (
            FILES,
            "Accepts files",
            "Send documents with your message",
            Icon::File,
        ),
        (
            GENERATE,
            "Creates images",
            "Generate pictures from a prompt",
            Icon::Sparkles,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let row = Rect::new(
            panel.x,
            panel.y + 28.0 + i as f32 * row_h,
            panel.w,
            row_h - 4.0,
        );
        let response = ui.button(wid_n("ai.model-filter", mask as u64), row);
        let on = st.ai.model_filters & mask != 0;
        ui.c.fill_rrect(
            row,
            9.0,
            if on {
                BLUE.mul_alpha(0.15)
            } else {
                Color::white(if response.hovered { 0.09 } else { 0.035 })
            },
        );
        if on {
            ui.c.stroke_rrect(row, 9.0, BLUE.mul_alpha(0.5), 1.0);
        }
        ui.c.icon(
            icon,
            Rect::new(row.x + 12.0, row.center().y - 8.0, 16.0, 16.0),
            if on { BLUE } else { palette::TEXT_DIM },
        );
        let label_h = if row.h >= 42.0 { 24.0 } else { row.h };
        text(
            ui,
            label,
            Rect::new(
                row.x + 40.0,
                row.y + if row.h >= 42.0 { 3.0 } else { 0.0 },
                row.w - 80.0,
                label_h,
            ),
            12.0,
            if on { BLUE } else { palette::TEXT },
        );
        if row.h >= 42.0 {
            text(
                ui,
                hint,
                Rect::new(row.x + 40.0, row.y + 25.0, row.w - 80.0, 17.0),
                10.0,
                palette::TEXT_DIM,
            );
        }
        let check = Rect::new(row.right() - 30.0, row.center().y - 8.0, 16.0, 16.0);
        ui.c.stroke_rrect(check, 5.0, if on { BLUE } else { Color::white(0.35) }, 1.0);
        if on {
            ui.c.fill_rrect(check, 5.0, BLUE);
            ui.c.icon(Icon::Check, check.inset(3.0, 3.0), Color::black(1.0));
        }
        if response.clicked {
            st.ai.model_filters ^= mask;
            st.ai.model_scroll = 0.0;
            st.ai.hover_model = None;
            ui.request_repaint();
        }
    }
    if ui
        .pill_button(
            wid("ai.model-filter-clear"),
            Rect::new(panel.x, bottom, 90.0, 30.0),
            "Reset filters",
            None,
            Color::white(0.06),
            palette::TEXT_DIM,
        )
        .clicked
    {
        st.ai.model_filters = 0;
        st.ai.model_scroll = 0.0;
        ui.request_repaint();
    }
    rows(st);
    let count = st
        .ai
        .model_rows
        .iter()
        .filter(|row| matches!(row, ModelRow::Model(_)))
        .count();
    let label = format!("Show {count} model{}", if count == 1 { "" } else { "s" });
    if ui
        .pill_button(
            wid("ai.model-filter-done"),
            Rect::new(panel.right() - 136.0, bottom, 136.0, 30.0),
            &label,
            None,
            BLUE.mul_alpha(0.20),
            BLUE,
        )
        .clicked
    {
        st.ai.model_filters_open = false;
        ui.input.focused = Some(wid("ai.model-search"));
        ui.input.caret = st.ai.search.chars().count();
        st.cmd(Command::FocusText);
        ui.request_repaint();
    }
}
fn scrollbar(ui: &mut Ui, id: u64, r: Rect, total: f32, scroll: &mut f32) {
    if total <= r.h {
        return;
    }
    let h = (r.h * r.h / total).max(20.0).min(r.h);
    let max = total - r.h;
    let b = ui.button(id, Rect::new(r.right() - 8.0, r.y, 8.0, r.h));
    if b.pressed && r.h > h {
        if let Some(p) = ui.input.mouse {
            let value = ((p.y - r.y - h / 2.0) / (r.h - h) * max).clamp(0.0, max);
            if (*scroll - value).abs() > 0.1 {
                *scroll = value;
                ui.request_repaint();
            }
        }
    }
    ui.c.fill_rrect(
        Rect::new(r.right() - 3.0, r.y + (r.h - h) * *scroll / max, 2.0, h),
        1.0,
        Color::white(0.3),
    );
}
fn context(m: &Model) -> String {
    if m.context == 0 {
        "Not supplied".into()
    } else if m.context >= 1_000_000 {
        format!("{:.2}M tokens", m.context as f64 / 1_000_000.0)
    } else {
        format!("{:.0}K tokens", m.context as f64 / 1024.0)
    }
}
fn modalities(v: &[String]) -> String {
    if v.is_empty() {
        "Not supplied".into()
    } else {
        v.iter()
            .map(|s| match s.as_str() {
                "text" => "Text",
                "image" => "Images",
                "file" => "Files / PDF",
                "audio" => "Audio",
                "video" => "Video",
                v => v,
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
}
fn capabilities(m: &Model) -> String {
    let mut result = vec![];
    if m.capabilities
        .iter()
        .any(|s| s == "tools" || s == "tool_choice")
    {
        result.push("Tool use");
    }
    if m.capabilities
        .iter()
        .any(|s| s == "structured_outputs" || s == "response_format")
    {
        result.push("Structured replies");
    }
    if m.outputs.iter().any(|s| s == "image") {
        result.push("Image generation");
    }
    if m.inputs.iter().any(|s| s == "image") {
        result.push("Image understanding");
    }
    if m.reasoning.mandatory
        || !m.reasoning.levels.is_empty()
        || m.capabilities.iter().any(|s| s == "reasoning")
    {
        result.push("Reasoning");
    }
    if result.is_empty() {
        "No additional capabilities supplied".into()
    } else {
        result.join(" · ")
    }
}
fn reasoning(m: &Model) -> String {
    if m.reasoning.adjustable() {
        format!(
            "{} effort levels · {}",
            m.reasoning.levels.len(),
            m.reasoning
                .levels
                .iter()
                .map(|s| ai_reasoning::label(s))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else if m.reasoning.mandatory || m.capabilities.iter().any(|s| s == "reasoning") {
        "Model-managed reasoning · no effort slider".into()
    } else {
        "Adjustable reasoning not advertised".into()
    }
}
fn price(m: &Model) -> String {
    if m.free == Some(true) {
        return "Free · input and output".into();
    }
    let fmt = |p: Option<f64>| {
        p.map(|p| {
            let p = p * 1e6;
            if p == 0.0 {
                "$0".into()
            } else if p < 0.0001 {
                "<$0.0001".into()
            } else if p < 0.01 {
                format!("${p:.4}")
            } else {
                format!("${p:.2}")
            }
        })
        .unwrap_or_else(|| "Not supplied".into())
    };
    format!(
        "Input {} · output {} per 1M tokens",
        fmt(m.input_price),
        fmt(m.output_price)
    )
}
fn preview_card(ui: &mut Ui, m: &Model, r: Rect, row: Rect) {
    let above = row.y - r.y - 6.0;
    let below = r.bottom() - row.bottom() - 6.0;
    let available = above.max(below);
    if available < 76.0 {
        return;
    }
    let h = available.min(226.0);
    let w = (r.w - 16.0).min(370.0);
    let card = Rect::new(
        r.right() - w - 8.0,
        if above > below {
            row.y - h - 5.0
        } else {
            row.bottom() + 5.0
        },
        w,
        h,
    );
    // A tooltip never covers the row being inspected and never overflows the
    // popover. The info button opens the complete scrollable details sheet.
    ui.hits.retain(|hit| {
        hit.rect.right() <= card.x
            || hit.rect.x >= card.right()
            || hit.rect.bottom() <= card.y
            || hit.rect.y >= card.bottom()
    });
    surface(ui, card);
    ui.c.clip(card.inset(10.0, 6.0));
    ui.c.text(
        m.display_name(),
        Rect::new(card.x + 12.0, card.y + 8.0, card.w - 24.0, 23.0),
        &TextStyle::new(13.0).weight(Weight::Semibold),
    );
    text(
        ui,
        &format!(
            "{} · {} · {}",
            m.provider_name(),
            m.kind.name(),
            m.cost_group()
        ),
        Rect::new(card.x + 12.0, card.y + 32.0, card.w - 24.0, 19.0),
        11.0,
        palette::TEXT_DIM,
    );
    let mut y = card.y + 57.0;
    if h >= 140.0 {
        text(
            ui,
            &m.description,
            Rect::new(card.x + 12.0, y, card.w - 24.0, 20.0),
            11.0,
            palette::TEXT_DIM,
        );
        y += 26.0;
    }
    for (label, value) in [
        ("Accepts", modalities(&m.inputs)),
        ("Context", context(m)),
        (
            "Effort",
            if m.reasoning.adjustable() {
                format!("{} levels available", m.reasoning.levels.len())
            } else if m.reasoning.mandatory || m.capabilities.iter().any(|s| s == "reasoning") {
                "Model-managed".into()
            } else {
                "Not advertised".into()
            },
        ),
        ("Cost", price(m)),
    ] {
        if y + 23.0 > card.bottom() - 10.0 {
            break;
        }
        text(
            ui,
            label,
            Rect::new(card.x + 12.0, y, 55.0, 21.0),
            10.0,
            palette::TEXT_FAINT,
        );
        text(
            ui,
            &value,
            Rect::new(card.x + 70.0, y, card.w - 82.0, 21.0),
            11.0,
            palette::TEXT,
        );
        y += 25.0;
    }
    if y + 22.0 < card.bottom() - 8.0 {
        text(
            ui,
            "ⓘ Click the info button for full details",
            Rect::new(card.x + 12.0, y, card.w - 24.0, 20.0),
            10.0,
            palette::TEXT_DIM,
        );
    }
    ui.c.unclip();
}
fn details(ui: &mut Ui, st: &mut AppState, r: Rect, settings: bool) {
    let Some(m) = st
        .ai
        .models
        .iter()
        .find(|m| Some(&m.id) == st.ai.model_detail.as_ref())
        .cloned()
    else {
        st.ai.model_detail = None;
        ui.request_repaint();
        return;
    };
    let back = ui.button(
        wid("ai.model-detail-back"),
        Rect::new(r.x + 8.0, r.y + 7.0, 90.0, 28.0),
    );
    text(
        ui,
        "‹ Models",
        Rect::new(r.x + 14.0, r.y + 7.0, 90.0, 28.0),
        12.0,
        palette::TEXT_DIM,
    );
    if back.clicked {
        st.ai.model_detail = None;
        ui.request_repaint();
        return;
    }
    if close(ui, r) {
        st.ai.close_models();
        ui.input.focused = if settings {
            None
        } else {
            Some(wid("ai.prompt"))
        };
        ui.input.caret = st.ai.draft.chars().count();
        ui.request_repaint();
        return;
    }
    let footer = Rect::new(r.x + 12.0, r.bottom() - 41.0, r.w - 24.0, 30.0);
    let view = Rect::new(
        r.x + 12.0,
        r.y + 42.0,
        r.w - 24.0,
        (footer.y - r.y - 49.0).max(24.0),
    );
    let mut y = view.y - st.ai.model_detail_scroll;
    let x = view.x;
    let w = view.w - 8.0;
    let start = y;
    ui.c.clip(view);
    let title = TextStyle::new(16.0).weight(Weight::Semibold).top().wrap();
    let th = ui.c.measure(m.display_name(), &title, w).1.max(22.0);
    ui.c.text(m.display_name(), Rect::new(x, y, w, th), &title);
    y += th + 7.0;
    text(
        ui,
        &format!(
            "{} · {} · {}",
            m.provider_name(),
            m.kind.name(),
            m.cost_group()
        ),
        Rect::new(x, y, w, 21.0),
        11.0,
        palette::TEXT_DIM,
    );
    y += 31.0;
    for (heading, value) in [
        ("Inputs", modalities(&m.inputs)),
        ("Outputs", modalities(&m.outputs)),
        ("Model capabilities", capabilities(&m)),
        ("Reasoning / effort", reasoning(&m)),
        ("Context window", context(&m)),
        ("Price", price(&m)),
        ("About this model", m.description.clone()),
    ] {
        text(
            ui,
            heading,
            Rect::new(x, y, w, 20.0),
            11.0,
            palette::TEXT_FAINT,
        );
        y += 23.0;
        let style = TextStyle::new(12.0).top().wrap().line_spacing(1.35);
        let h = ui.c.measure(&value, &style, w).1.max(18.0);
        ui.c.text(&value, Rect::new(x, y, w, h), &style);
        y += h + 17.0;
    }
    ui.c.unclip();
    let total = y - start;
    let before = st.ai.model_detail_scroll;
    st.ai.model_detail_scroll = (st.ai.model_detail_scroll - ui.wheel_in(view) * 34.0)
        .clamp(0.0, (total - view.h).max(0.0));
    if (before - st.ai.model_detail_scroll).abs() > 0.1 {
        ui.request_repaint();
    }
    scrollbar(
        ui,
        wid("ai.model-detail-scroll"),
        view,
        total,
        &mut st.ai.model_detail_scroll,
    );
    if ui
        .pill_button(
            wid("ai.model-detail-select"),
            footer,
            if settings {
                "Set as default model"
            } else {
                "Use this model"
            },
            Some(Icon::Check),
            Color::white(0.10),
            palette::TEXT,
        )
        .clicked
    {
        selected(ui, st, m.id, settings);
    }
}
