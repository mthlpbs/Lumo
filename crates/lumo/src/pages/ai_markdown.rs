//! Native chat Markdown, parsed and laid out only when text/width/type changes.
use crate::{
    commands::Command,
    gfx::{TextStyle, Weight},
    ui::{wid_n, Ui},
};
use lumo_core::{
    geom::{palette, Color, Point, Rect},
    markdown::{self, Block, Inline},
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

enum Paint {
    Text {
        content: Inline,
        r: Rect,
        style: TextStyle,
    },
    Box(Rect, u8),
    Rule(f32),
    Line(Point, Point),
    CodeHeader {
        block: Rect,
        language: String,
        source: String,
    },
}
pub struct Layout {
    paint: Vec<Paint>,
    pub height: f32,
}
struct Entry {
    source: String,
    doc: markdown::Document,
    key: (u32, u32, u32, usize),
    layout: Rc<Layout>,
}
thread_local! {static CACHE:RefCell<VecDeque<Entry>>=const{RefCell::new(VecDeque::new())};}
pub fn clear() {
    CACHE.with(|c| c.borrow_mut().clear());
}
fn styled(size: f32) -> TextStyle {
    TextStyle::new(size).top().wrap().line_spacing(1.5).color(Color::white(0.86))
}
fn cell(
    ui: &mut Ui,
    paint: &mut Vec<Paint>,
    content: &Inline,
    x: f32,
    y: f32,
    w: f32,
    style: TextStyle,
) -> f32 {
    let h =
        ui.c.measure_rich(content, &style, w)
            .1
            .max(style.size * 1.5);
    paint.push(Paint::Text {
        content: content.clone(),
        r: Rect::new(x, y, w, h),
        style,
    });
    h
}
fn build(ui: &mut Ui, doc: &markdown::Document, width: f32, size: f32) -> Layout {
    let mut paint = vec![];
    let mut y = 0.0;
    let gap = 10.0;
    for b in &doc.blocks {
        match b {
            Block::Heading(level, t) => {
                if y > 0.0 {
                    y += 5.0;
                }
                let style = styled(match level {
                    1 => size + 5.0,
                    2 => size + 3.0,
                    3 => size + 2.0,
                    _ => size + 0.5,
                })
                .weight(Weight::Semibold);
                y += cell(ui, &mut paint, t, 0.0, y, width, style) + gap;
            }
            Block::Paragraph(t) => {
                y += cell(ui, &mut paint, t, 0.0, y, width, styled(size)) + gap;
            }
            Block::List {
                marker,
                depth,
                content,
            } => {
                let x = (*depth as f32 * 16.0).min(width * 0.3);
                let st = styled(size);
                cell(
                    ui,
                    &mut paint,
                    &markdown::inline(marker),
                    x,
                    y,
                    14.0,
                    st.color(palette::TEXT_DIM),
                );
                y += cell(
                    ui,
                    &mut paint,
                    content,
                    x + 18.0,
                    y,
                    (width - x - 18.0).max(30.0),
                    st,
                ) + 4.0;
            }
            Block::Quote(t) => {
                let h =
                    ui.c.measure_rich(t, &styled(size), width - 24.0)
                        .1
                        .max(size * 1.4);
                paint.push(Paint::Box(Rect::new(0.0, y, width, h + 16.0), 1));
                cell(
                    ui,
                    &mut paint,
                    t,
                    14.0,
                    y + 8.0,
                    width - 24.0,
                    styled(size).color(palette::TEXT_DIM),
                );
                y += h + 16.0 + gap;
            }
            Block::Code { language, content } => {
                let top = 38.0;
                let h =
                    ui.c.measure_rich(content, &styled(size - 1.0), width - 24.0)
                        .1
                        .max(size * 1.4);
                paint.push(Paint::Box(Rect::new(0.0, y, width, h + top + 12.0), 2));
                cell(
                    ui,
                    &mut paint,
                    content,
                    12.0,
                    y + top,
                    width - 24.0,
                    styled(size - 1.0),
                );
                paint.push(Paint::CodeHeader {
                    block: Rect::new(0.0, y, width, h + top + 12.0),
                    language: if language.is_empty() {
                        "Code".into()
                    } else {
                        language.clone()
                    },
                    source: content.text.clone(),
                });
                y += h + top + 12.0 + gap;
            }
            Block::Rule => {
                y += 7.0;
                paint.push(Paint::Rule(y));
                y += 15.0;
            }
            Block::Table { rows, align } => {
                let cols = align.len().max(1);
                let mut widths = vec![48.0f32; cols];
                for row in rows {
                    for (i, t) in row.iter().take(cols).enumerate() {
                        let w =
                            ui.c.measure_rich(t, &TextStyle::new(size - 1.0).top(), 900.0)
                                .0
                                + 22.0;
                        widths[i] = widths[i].max(w.min(360.0));
                    }
                }
                let sum: f32 = widths.iter().sum();
                for w in &mut widths {
                    *w = width * (*w / sum);
                }
                for (ri, row) in rows.iter().enumerate() {
                    let mut rh = 0.0f32;
                    let mut x = 0.0;
                    let mut cells = vec![];
                    for (i, t) in row.iter().take(cols).enumerate() {
                        let mut st = styled(size - 1.0);
                        if ri == 0 {
                            st = st.weight(Weight::Semibold);
                        }
                        st = match align[i] {
                            markdown::Align::Left => st,
                            markdown::Align::Center => st.center(),
                            markdown::Align::Right => st.right(),
                        };
                        let w = (widths[i] - 20.0).max(14.0);
                        let h = ui.c.measure_rich(t, &st, w).1.max(st.size * 1.4);
                        rh = rh.max(h);
                        cells.push((t, x + 10.0, w, st));
                        x += widths[i];
                    }
                    rh += 18.0;
                    paint.push(Paint::Box(
                        Rect::new(0.0, y, width, rh),
                        if ri == 0 { 3 } else { 4 + (ri % 2) as u8 },
                    ));
                    for (t, x, w, st) in cells {
                        cell(ui, &mut paint, t, x, y + 9.0, w, st);
                    }
                    paint.push(Paint::Line(
                        Point::new(0.0, y + rh),
                        Point::new(width, y + rh),
                    ));
                    y += rh;
                }
                y += gap;
            }
        }
    }
    Layout {
        paint,
        height: (y - gap).max(size * 1.4),
    }
}
pub fn layout(ui: &mut Ui, source: &str, width: f32, size: f32) -> Rc<Layout> {
    let key = (
        width.to_bits(),
        size.to_bits(),
        ui.c.scale().to_bits(),
        ui.c.look() as *const _ as usize,
    );
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let position = cache.iter().position(|e| e.source == source);
        let mut entry = if let Some(p) = position {
            cache.remove(p).unwrap()
        } else {
            Entry {
                source: source.into(),
                doc: markdown::parse(source),
                key,
                layout: Rc::new(Layout {
                    paint: vec![],
                    height: 0.0,
                }),
            }
        };
        if entry.key != key || entry.layout.height == 0.0 {
            entry.layout = Rc::new(build(ui, &entry.doc, width, size));
            entry.key = key;
        }
        let result = entry.layout.clone();
        cache.push_front(entry);
        let mut bytes: usize = cache.iter().map(|e| e.source.len()).sum();
        while cache.len() > 24 || bytes > 2 * 1024 * 1024 {
            if let Some(e) = cache.pop_back() {
                bytes = bytes.saturating_sub(e.source.len());
            } else {
                break;
            }
        }
        result
    })
}
pub fn draw(
    ui: &mut Ui,
    layout: &Layout,
    origin: Point,
    view: Rect,
    message_id: u64,
    commands: &mut Vec<Command>,
) {
    let mut link_index = 0u64;
    let mut code_index = 0u64;
    for p in &layout.paint {
        match p {
            Paint::Text { content, r, style } => {
                let r = r.offset(origin.x, origin.y);
                if r.bottom() < view.y || r.y > view.bottom() {
                    continue;
                }
                ui.c.rich_text(content, r, style);
                for link in &content.links {
                    for hit in ui.c.rich_range(content, style, r.w, link.start, link.len) {
                        let hit = hit.offset(r.x, r.y);
                        let id = message_id.wrapping_mul(8191).wrapping_add(link_index);
                        link_index += 1;
                        let resp = ui.button(wid_n("ai.markdown.link", id), hit);
                        if resp.hovered {
                            ui.c.line(
                                Point::new(hit.x, hit.bottom() - 1.0),
                                Point::new(hit.right(), hit.bottom() - 1.0),
                                palette::BLUE,
                                1.0,
                            );
                        }
                        if resp.clicked {
                            commands.push(Command::OpenUrl(link.url.clone()));
                        }
                    }
                }
            }
            Paint::Box(r, kind) => {
                let r = r.offset(origin.x, origin.y);
                if r.bottom() < view.y || r.y > view.bottom() {
                    continue;
                }
                ui.c.fill_rrect(
                    r,
                    if *kind >= 3 { 0.0 } else { 8.0 },
                    Color::white(match kind {
                        1 => 0.04,
                        2 => 0.075,
                        3 => 0.10,
                        4 => 0.025,
                        _ => 0.045,
                    }),
                );
                if *kind == 1 {
                    ui.c.fill_rrect(
                        Rect::new(r.x, r.y + 4.0, 3.0, r.h - 8.0),
                        1.5,
                        Color::white(0.3),
                    );
                }
            }
            Paint::Rule(y) => {
                let y = origin.y + y;
                if y >= view.y && y <= view.bottom() {
                    ui.c.line(
                        Point::new(view.x, y),
                        Point::new(view.right(), y),
                        Color::white(0.14),
                        1.0,
                    );
                }
            }
            Paint::Line(a, b) => {
                let a = Point::new(origin.x + a.x, origin.y + a.y);
                let b = Point::new(origin.x + b.x, origin.y + b.y);
                if a.y >= view.y && a.y <= view.bottom() {
                    ui.c.line(a, b, Color::white(0.10), 1.0);
                }
            }
            Paint::CodeHeader {
                block,
                language,
                source,
            } => {
                let id = wid_n(
                    "ai.markdown.copy",
                    message_id.wrapping_mul(8191).wrapping_add(code_index),
                );
                code_index += 1;
                let block = block.offset(origin.x, origin.y);
                if block.bottom() < view.y + 32.0 || block.y > view.bottom() {
                    continue;
                }
                // Keep Copy reachable while scrolling a long code/formula
                // block. An opaque header prevents text bleeding through it.
                let header = Rect::new(
                    block.x,
                    block.y.max(view.y).min(block.bottom() - 32.0),
                    block.w,
                    32.0,
                );
                ui.c.fill_rrect(header, 6.0, Color::rgba(0.12, 0.12, 0.13, 1.0));
                ui.c.text(
                    language,
                    Rect::new(header.x + 12.0, header.y, header.w - 96.0, 32.0),
                    &TextStyle::new(11.0).color(palette::TEXT_DIM),
                );
                let button = Rect::new(header.right() - 78.0, header.y + 2.0, 70.0, 28.0);
                if ui
                    .pill_button(
                        id,
                        button,
                        "Copy",
                        Some(crate::icons::Icon::Copy),
                        Color::white(0.06),
                        palette::TEXT_DIM,
                    )
                    .clicked
                {
                    commands.push(Command::CopyText(source.clone()));
                }
            }
        }
    }
}
