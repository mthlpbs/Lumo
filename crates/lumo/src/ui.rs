//! Immediate-mode widgets on top of [`Canvas`].
//!
//! Pages call `ui.button(id, rect)` etc. while drawing; every interactive
//! region is recorded as a [`Hit`]. Mouse presses/releases from the window are
//! resolved against the previous frame's hits, so a click shows up as
//! `clicked == true` on the next frame (the app always repaints after input).

use crate::gfx::{Align, Canvas, TextStyle, Weight};
use crate::icons::Icon;
use lumo_core::geom::{palette, Color, Point, Rect};

pub type WidgetId = u64;

/// Stable widget id from a string (FNV-1a).
pub fn wid(s: &str) -> WidgetId {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Widget id for the `n`-th item of a list.
pub fn wid_n(s: &str, n: u64) -> WidgetId {
    wid(s) ^ n.wrapping_mul(0x9E3779B97F4A7C15)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Backspace,
    Delete,
    Enter,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Ctrl+V (paste from clipboard; the app supplies the text as chars).
    SelectAll,
    SelectLeft,
    SelectRight,
    SelectHome,
    SelectEnd,
    Copy,
    Cut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorKind {
    Arrow,
    Hand,
    Text,
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub id: WidgetId,
    pub rect: Rect,
    pub cursor: CursorKind,
}

/// Input state owned by the app and shared with widgets.
#[derive(Debug, Default)]
pub struct Input {
    pub ctrl:bool,
    /// Cursor position in canvas DIPs (None when outside the island).
    pub mouse: Option<Point>,
    /// Left button is held.
    pub down: bool,
    /// Widget under the cursor when the button went down.
    pub pressed_id: Option<WidgetId>,
    /// The left button went down since the last frame (valid for one frame).
    pub just_pressed: bool,
    pub press_pos: Option<Point>,
    /// Click resolved on mouse-up (valid for exactly one frame).
    pub clicked: Option<(WidgetId, MouseButton)>,
    /// Widget that was held when the left button came up, and where it came
    /// up, even off the widget (a slider drag; valid for exactly one frame).
    pub released: Option<(WidgetId, Point)>,
    /// Mouse-wheel notches this frame (+ = away from user).
    pub wheel: f32,
    /// Keys typed while a text field has focus.
    pub keys: Vec<KeyInput>,
    /// Widget that owns keyboard focus.
    pub focused: Option<WidgetId>,
    /// Caret position (chars) inside the focused field.
    pub caret: usize,
    pub selection_all:bool,
    pub selection_anchor:Option<usize>,
    pub selection_owner:Option<WidgetId>,
    pub text_scroll:f32,
    pub shift:bool,
    pub double_click:bool,
    pub text_word_drag:bool,
    pub clipboard_write:Option<String>,
    /// Next visible-editor blink boundary; hosts use a one-shot wake, never a
    /// frame loop. Cleared before each paint so hidden editors cannot wake it.
    pub caret_wake:Option<f64>,
    pub caret_epoch:f64,
    pub caret_key:Option<(WidgetId,usize,Option<usize>)>,
    pub caret_period:Option<f64>,
    pub high_surrogate:Option<u16>,
}

impl Input {
    pub fn begin_frame(&mut self){self.caret_wake=None;if self.focused.is_none(){self.caret_key=None;}}
    pub fn caret_visible(&mut self,now:f64,edited:bool,enabled:bool)->bool{
        if !enabled { self.caret_wake=None;self.caret_key=None;return false; }
        let Some(owner)=self.focused else{self.caret_wake=None;self.caret_key=None;return false;};
        let key=(owner,self.caret,self.selection_anchor);
        if self.caret_key.is_none_or(|k|k.0!=owner){
            let ms=unsafe{windows::Win32::UI::WindowsAndMessaging::GetCaretBlinkTime()};
            self.caret_period=match ms{u32::MAX=>None,0=>Some(0.53),_=>Some(ms as f64/1000.0)};
        }
        if self.caret_key!=Some(key)||edited||self.just_pressed{self.caret_epoch=now;self.caret_key=Some(key);}
        let selected=self.selection_anchor.is_some_and(|a|a!=self.caret)||self.selection_all;
        if selected{self.caret_wake=None;return false;}
        let Some(period)=self.caret_period else{self.caret_wake=None;return true;};
        let phase=((now-self.caret_epoch).max(0.0)/period).floor();
        self.caret_wake=Some(self.caret_epoch+(phase+1.0)*period);
        phase as u64%2==0
    }
    /// Called after each frame: one-shot inputs are consumed.
    pub fn end_frame(&mut self) {
        self.clicked = None;
        self.released = None;
        self.just_pressed = false;
        self.double_click=false;
        self.wheel = 0.0;
        self.keys.clear();
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Resp {
    pub hovered: bool,
    /// Held down on this widget.
    pub pressed: bool,
    pub clicked: bool,
    pub rclicked: bool,
    /// While pressed: distance moved since the press (DIPs).
    pub drag: Option<(f32, f32)>,
}

/// What a seekable [`Ui::progress_bar`] did this frame (fractions 0..=1).
#[derive(Clone, Copy, Debug, Default)]
pub struct Scrub {
    /// Held down: where the pointer is along the bar.
    pub held: Option<f32>,
    /// Let go: where to seek.
    pub released: Option<f32>,
    /// The press on the bar began this frame.
    pub began: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextInputResp {
    pub focused: bool,
    pub submitted: bool,
    pub cancelled: bool,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum InputLook {
    Secret,
    Area,
    AreaTop,
    Box,
    Row { accent: Color, icon: Option<Icon> },
}

/// The button system: two heights and three looks.
pub const PILL_SMALL: f32 = 28.0;
/// One style for multiline drawing, hit-testing, selections and caret metrics.
pub(crate) fn text_area_style()->TextStyle{TextStyle::new(14.0).top().wrap().line_spacing(1.5)}
pub const PILL_REGULAR: f32 = 34.0;
/// Gap between neighbouring capsules.
pub const PILL_GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PillStyle {
    /// Secondary utility (Clear, Undo, …): neutral fill, dim label that brightens on hover.
    Utility,
    /// A page action: neutral fill, full-white label.
    Neutral,
    /// The page's one primary action: accent at 0.20 fill, accent label.
    Primary(Color),
    /// A utility capsule whose label is in an accent colour (e.g. "Undo").
    UtilityAccent(Color),
}

pub struct Ui<'a, 'g> {
    pub c: &'a mut Canvas<'g>,
    pub input: &'a mut Input,
    pub hits: &'a mut Vec<Hit>,
    /// Monotonic seconds.
    pub now: f64,
    repaint: bool,
    animating: bool,
    live: bool,
    wants_focus: Option<bool>,
}

impl<'a, 'g> Ui<'a, 'g> {
    pub fn new(c: &'a mut Canvas<'g>, input: &'a mut Input, hits: &'a mut Vec<Hit>, now: f64) -> Self {
        Self { c, input, hits, now, repaint: false, animating: false, live: false, wants_focus: None }
    }

    /// Ask for one more frame (e.g. after a state change).
    pub fn request_repaint(&mut self) {
        self.repaint = true;
    }

    /// Ask for continuous frames while something moves (e.g. a lyric scroll).
    pub fn request_anim(&mut self) {
        self.animating = true;
    }

    /// Ask for slow ambient frames (~30 fps) while something drifts steadily,
    /// e.g. a progress bar while music plays. Paused when the screen is off.
    pub fn request_live(&mut self) {
        self.live = true;
    }

    pub fn live_requested(&self) -> bool {
        self.live
    }

    pub fn repaint_requested(&self) -> bool {
        self.repaint
    }

    pub fn anim_requested(&self) -> bool {
        self.animating
    }

    /// `Some(true)` = a text field wants keyboard focus, `Some(false)` = release it.
    pub fn focus_request(&self) -> Option<bool> {
        self.wants_focus
    }

    pub fn hovered(&self, r: Rect) -> bool {
        self.input.mouse.is_some_and(|m| r.contains(m))
    }

    /// Register an interactive region and get its state.
    pub fn interact(&mut self, id: WidgetId, r: Rect, cursor: CursorKind) -> Resp {
        self.hits.push(Hit { id, rect: r, cursor });
        let hovered = self.hovered(r);
        let pressed = self.input.down && self.input.pressed_id == Some(id);
        let clicked = self.input.clicked == Some((id, MouseButton::Left));
        let rclicked = self.input.clicked == Some((id, MouseButton::Right));
        if clicked || rclicked {
            self.repaint = true;
        }
        let drag = if pressed {
            match (self.input.press_pos, self.input.mouse) {
                (Some(a), Some(b)) => Some((b.x - a.x, b.y - a.y)),
                _ => None,
            }
        } else {
            None
        };
        Resp { hovered, pressed, clicked, rclicked, drag }
    }

    /// Invisible clickable area.
    pub fn button(&mut self, id: WidgetId, r: Rect) -> Resp {
        self.interact(id, r, CursorKind::Hand)
    }

    /// Round icon button with hover/pressed background.
    pub fn icon_button(&mut self, id: WidgetId, r: Rect, icon: Icon, color: Color, icon_size: f32) -> Resp {
        let resp = self.button(id, r);
        let bg = if resp.pressed {
            palette::SURFACE_PRESSED
        } else if resp.hovered {
            palette::SURFACE_HOVER
        } else {
            Color::white(0.0)
        };
        if bg.a > 0.0 {
            self.c.fill_rrect(r, r.h.min(r.w) / 2.0, bg);
        }
        let ir = Rect::from_center(r.center(), icon_size, icon_size);
        let col = if resp.pressed { color.mul_alpha(0.8) } else { color };
        self.c.icon(icon, ir, col);
        resp
    }

    /// Filled capsule button with an optional leading icon.
    pub fn pill_button(&mut self, id: WidgetId, r: Rect, label: &str, icon: Option<Icon>, fill: Color, text: Color) -> Resp {
        let resp = self.button(id, r);
        let f = if resp.pressed {
            Color::lerp(fill, Color::white(1.0), 0.18)
        } else if resp.hovered {
            Color::lerp(fill, Color::white(1.0), 0.1)
        } else {
            fill
        };
        self.c.fill_rrect(r, r.h / 2.0, f);
        let st = TextStyle::new(12.5).weight(Weight::Semibold).color(text).center();
        match icon {
            Some(i) => {
                let (tw, _) = self.c.measure(label, &st, r.w);
                let total = 14.0 + if label.is_empty() { 0.0 } else { 5.0 + tw };
                let x = r.x + (r.w - total) / 2.0;
                self.c.icon(i, Rect::new(x, r.center().y - 7.0, 14.0, 14.0), text);
                if !label.is_empty() {
                    self.c.text(label, Rect::new(x + 19.0, r.y, tw + 2.0, r.h), &st.align(Align::Left));
                }
            }
            None => self.c.text(label, r, &st),
        }
        resp
    }

    /// Width a [`Ui::pill`] needs for `label` (and an optional icon).
    pub fn pill_width(&mut self, label: &str, icon: bool, small: bool) -> f32 {
        let (size, isz, pad) = if small { (crate::gfx::ts::FOOTNOTE, 13.0, 12.0) } else { (crate::gfx::ts::BODY, 14.0, 16.0) };
        let tw = if label.is_empty() { 0.0 } else { self.c.measure(label, &TextStyle::new(size).weight(Weight::Semibold), 400.0).0 };
        let iw = if icon { isz + if label.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
        (2.0 * pad + iw + tw).ceil()
    }

    /// A capsule button from the button system. Height decides the size:
    /// `PILL_SMALL` (28) = 12 pt label / 13 icon, `PILL_REGULAR` (34) = 13 / 14.
    pub fn pill(&mut self, id: WidgetId, r: Rect, label: &str, icon: Option<Icon>, style: PillStyle) -> Resp {
        let resp = self.button(id, r);
        let small = r.h <= 30.0;
        let (fill, text) = match style {
            PillStyle::Utility => {
                let f = if resp.pressed { Color::white(0.22) } else if resp.hovered { Color::white(0.15) } else { palette::SURFACE };
                (f, if resp.hovered || resp.pressed { palette::TEXT } else { palette::TEXT_DIM })
            }
            PillStyle::Neutral => {
                let f = if resp.pressed { Color::white(0.22) } else if resp.hovered { Color::white(0.15) } else { palette::SURFACE };
                (f, palette::TEXT)
            }
            PillStyle::Primary(c) => {
                let a = if resp.pressed { 0.36 } else if resp.hovered { 0.28 } else { 0.20 };
                (c.with_alpha(a), c)
            }
            PillStyle::UtilityAccent(c) => {
                let f = if resp.pressed { Color::white(0.22) } else if resp.hovered { Color::white(0.15) } else { palette::SURFACE };
                (f, c)
            }
        };
        self.c.fill_rrect(r, r.h / 2.0, fill);
        let (size, isz) = if small { (crate::gfx::ts::FOOTNOTE, 13.0) } else { (crate::gfx::ts::BODY, 14.0) };
        let st = TextStyle::new(size).weight(Weight::Semibold).color(text);
        let tw = if label.is_empty() { 0.0 } else { self.c.measure(label, &st, r.w).0 };
        let iw = if icon.is_some() { isz + if label.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
        let x = r.x + (r.w - iw - tw) / 2.0;
        if let Some(i) = icon {
            self.c.icon(i, Rect::new(x, r.center().y - isz / 2.0, isz, isz), text);
        }
        if !label.is_empty() {
            self.c.text(label, Rect::new(x + iw, r.y, tw + 2.0, r.h), &st);
        }
        resp
    }

    /// A widget that takes the keyboard without being a text field (the
    /// Script page's reading view): `clicked` focuses it, Esc lets go.
    /// Returns the keys pressed while it has focus.
    pub fn key_focus(&mut self, id: WidgetId, clicked: bool) -> Vec<KeyInput> {
        if clicked && self.input.focused != Some(id) {
            self.input.focused = Some(id);
            self.wants_focus = Some(true);
        }
        if self.input.focused != Some(id) {
            return Vec::new();
        }
        let keys = std::mem::take(&mut self.input.keys);
        if keys.contains(&KeyInput::Escape) {
            self.input.focused = None;
            self.wants_focus = Some(false);
        }
        if !keys.is_empty() {
            self.repaint = true;
        }
        keys
    }

    /// Mouse-wheel notches over `r` (consumed so only one widget scrolls).
    pub fn wheel_in(&mut self, r: Rect) -> f32 {
        if self.hovered(r) && self.input.wheel != 0.0 {
            let w = self.input.wheel;
            self.input.wheel = 0.0;
            w
        } else {
            0.0
        }
    }

    /// Single-line text field. Click to focus, Enter submits, Esc cancels.
    pub fn text_input(&mut self, id: WidgetId, r: Rect, buf: &mut String, placeholder: &str, max_chars: usize) -> TextInputResp {
        self.text_input_styled(id, r, buf, placeholder, max_chars, InputLook::Box)
    }

    /// A list-row style field: no box until hovered/focused, a leading icon in
    /// the same column as list checkboxes (centred at x+18, text at x+36), and
    /// an accent focus ring.
    pub fn text_input_row(&mut self, id: WidgetId, r: Rect, buf: &mut String, placeholder: &str, max_chars: usize, accent: Color, icon: Option<Icon>) -> TextInputResp {
        self.text_input_styled(id, r, buf, placeholder, max_chars, InputLook::Row { accent, icon })
    }
    pub fn text_input_capsule(&mut self, id: WidgetId, r: Rect, buf: &mut String, placeholder: &str, max_chars: usize, accent: Color) -> TextInputResp {
        self.search_editor(id, r, buf, placeholder, max_chars, accent, false, None)
    }
    /// Browser address field, with space reserved for its trailing reload control.
    pub fn address_input(&mut self, id: WidgetId, r: Rect, buf: &mut String, accent: Color, progress: Option<f32>) -> TextInputResp {
        self.search_editor(id, r, buf, "", 2048, accent, true, progress)
    }

    pub fn text_secret(&mut self,id:WidgetId,r:Rect,buf:&mut String,placeholder:&str,max_chars:usize)->TextInputResp {
        self.text_input_styled(id,r,buf,placeholder,max_chars,InputLook::Secret)
    }
    pub fn text_area(&mut self,id:WidgetId,r:Rect,buf:&mut String,placeholder:&str,max_chars:usize)->TextInputResp {
        self.text_input_styled(id,r,buf,placeholder,max_chars,InputLook::Area)
    }

    pub fn text_area_top(&mut self,id:WidgetId,r:Rect,buf:&mut String,placeholder:&str,max_chars:usize)->TextInputResp {
        self.text_input_styled(id,r,buf,placeholder,max_chars,InputLook::AreaTop)
    }
    fn text_pointer(&mut self,id:WidgetId,buf:&str,style:&TextStyle,width:f32,origin:Point,multiline:bool){
        let release=self.input.released.filter(|(owner,_)|*owner==id);
        // Windows can deliver down and up before the next paint. The release
        // retains its owner after capture ends, so that press still counts.
        let press=self.input.just_pressed&&(self.input.pressed_id==Some(id)||release.is_some());
        if self.input.selection_owner!=Some(id)&&self.input.focused==Some(id){self.input.selection_owner=Some(id);self.input.selection_anchor=None;self.input.text_scroll=0.0;}
        if press{
            if self.input.focused!=Some(id){self.input.focused=Some(id);self.input.selection_owner=Some(id);self.input.selection_anchor=None;self.input.text_scroll=0.0;self.wants_focus=Some(true);}
            let at=self.input.press_pos.or(self.input.mouse).unwrap_or(origin);
            let point=Point::new(at.x-origin.x,if multiline{at.y-origin.y}else{style.size/2.0});
            let caret=self.c.text_position_at(buf,style,width,point);
            let old=self.input.caret.min(buf.chars().count());
            self.input.selection_anchor=Some(if self.input.shift{self.input.selection_anchor.unwrap_or(old)}else{caret});
            self.input.caret=caret;self.input.selection_all=false;self.input.text_word_drag=self.input.double_click;
            if self.input.double_click{self.input.selection_anchor=Some(0);self.input.caret=buf.chars().count();self.input.selection_all=true;}
            self.repaint=true;
        }
        if self.input.focused==Some(id)&&!self.input.text_word_drag&&((self.input.pressed_id==Some(id)&&self.input.down&&!press)||release.is_some()){
            if let Some(at)=release.map(|(_,p)|p).or(self.input.mouse){
                self.input.caret=self.c.text_position_at(buf,style,width,Point::new(at.x-origin.x,if multiline{at.y-origin.y}else{style.size/2.0}));
                self.input.selection_all=false;self.repaint=true;
            }
        }
    }

    fn search_editor(&mut self,id:WidgetId,r:Rect,buf:&mut String,placeholder:&str,max_chars:usize,accent:Color,address:bool,progress:Option<f32>)->TextInputResp{
        // Editing an address uses the whole field; browser controls are hidden.
        self.interact(id,r,CursorKind::Text);
        let inner=Rect::new(r.x+if address{14.0}else{43.0},r.y,(r.w-if address{28.0}else{58.0}).max(1.0),r.h);
        let style=TextStyle::new(crate::gfx::ts::BODY);
        let mut out=TextInputResp::default();
        let scroll=if self.input.focused==Some(id)&&self.input.selection_owner==Some(id){self.input.text_scroll}else{0.0};
        self.text_pointer(id,buf,&style,10000.0,Point::new(inner.x-scroll,inner.y),false);
        let focused=self.input.focused==Some(id);out.focused=focused;
        if focused{
            if self.input.selection_all{self.input.selection_anchor=Some(0);self.input.caret=buf.chars().count();}
            let mut chars:Vec<char>=buf.chars().collect();let mut caret=self.input.caret.min(chars.len());let mut anchor=self.input.selection_anchor.map(|p|p.min(chars.len()));
            for key in std::mem::take(&mut self.input.keys){
                let selected=anchor.filter(|a|*a!=caret).map(|a|a.min(caret)..a.max(caret));
                match key{
                    KeyInput::Char(c) if !c.is_control()=>{
                        if let Some(range)=selected{caret=range.start;chars.drain(range);}
                        anchor=None;if chars.len()<max_chars{chars.insert(caret,c);caret+=1;out.changed=true;}
                    },
                    KeyInput::Backspace|KeyInput::Delete|KeyInput::Cut=>{
                        if key==KeyInput::Cut{if let Some(range)=selected.clone(){self.input.clipboard_write=Some(chars[range].iter().collect());}}
                        if let Some(range)=selected{caret=range.start;chars.drain(range);anchor=None;out.changed=true;}
                        else if key==KeyInput::Backspace&&caret>0{caret-=1;chars.remove(caret);anchor=None;out.changed=true;}
                        else if key==KeyInput::Delete&&caret<chars.len(){chars.remove(caret);anchor=None;out.changed=true;}
                    },
                    KeyInput::Copy=>{if let Some(range)=selected{self.input.clipboard_write=Some(chars[range].iter().collect());}},
                    KeyInput::SelectAll=>{anchor=Some(0);caret=chars.len();},
                    KeyInput::Left|KeyInput::Right|KeyInput::Home|KeyInput::End|KeyInput::SelectLeft|KeyInput::SelectRight|KeyInput::SelectHome|KeyInput::SelectEnd=>{
                        let extend=matches!(key,KeyInput::SelectLeft|KeyInput::SelectRight|KeyInput::SelectHome|KeyInput::SelectEnd);
                        let next=match key{KeyInput::Left if selected.is_some()=>selected.as_ref().unwrap().start,KeyInput::Right if selected.is_some()=>selected.as_ref().unwrap().end,KeyInput::Left|KeyInput::SelectLeft=>caret.saturating_sub(1),KeyInput::Right|KeyInput::SelectRight=>(caret+1).min(chars.len()),KeyInput::Home|KeyInput::SelectHome=>0,_=>chars.len()};
                        if extend{anchor=Some(anchor.unwrap_or(caret));}else{anchor=None;}caret=next;
                    },
                    KeyInput::Enter=>out.submitted=true,
                    KeyInput::Escape=>out.cancelled=true,
                    _=>{},
                }
            }
            *buf=chars.into_iter().collect();self.input.caret=caret;self.input.selection_anchor=anchor;
            self.input.selection_all=anchor==Some(0)&&caret==buf.chars().count()&&caret>0;
            if out.changed||out.submitted||out.cancelled{self.repaint=true;}
        }
        let radius=if address{12.0}else{r.h/2.0};
        self.c.fill_rrect(r,radius,Color::white(if focused{0.10}else{0.055}));
        if let Some(progress) = progress {
            // Actual navigation milestones fill the entire field behind its
            // text and controls. No timer, pulsing or simulated progress.
            self.c.push_outline_mask(&crate::pill::shape::segments(r, radius));
            self.c.fill_rect(r, accent.mul_alpha(0.035));
            let width = r.w * progress.clamp(0.0, 1.0);
            self.c.fill_rect(Rect::new(r.x, r.y, width, r.h), accent.mul_alpha(0.14));
            self.c.fill_rect(Rect::new(r.x, r.bottom()-2.0, width, 2.0), accent);
            self.c.pop_shape_mask();
        }
        self.c.stroke_rrect(r.inset(0.5,0.5),radius,Color::white(if focused{0.16}else{0.09}),1.0);
        if !address{self.c.icon(Icon::Search,Rect::new(r.x+15.0,r.center().y-8.0,16.0,16.0),palette::TEXT_DIM);}
        let compact=address&&!focused;let location=if compact{crate::search_web::display_address(buf)}else{String::new()};let display=if compact{&location}else{buf};
        let (cx,_,height)=self.c.text_caret(display,&style,10000.0,if focused{self.input.caret}else{0});let y=inner.center().y-height/2.0;
        let width=self.c.measure(display,&style,10000.0).0;
        if focused{
            if cx<self.input.text_scroll{self.input.text_scroll=cx;}else if cx>self.input.text_scroll+inner.w-3.0{self.input.text_scroll=cx-inner.w+3.0;}
            self.input.text_scroll=self.input.text_scroll.clamp(0.0,(width-inner.w+3.0).max(0.0));
        }
        let scroll=if focused{self.input.text_scroll}else{0.0};self.c.clip(inner);
        if focused{if let Some(a)=self.input.selection_anchor{for hit in self.c.text_selection(display,&style,a.min(self.input.caret),a.max(self.input.caret)){self.c.fill_rect(Rect::new(inner.x+hit.x-scroll,y+hit.y,hit.w,hit.h),accent.mul_alpha(0.35));}}}
        let text_style=if compact{style.center()}else{style};
        if display.is_empty(){self.c.text(placeholder,inner,&style.color(palette::TEXT_FAINT));}
        else{self.c.text(display,Rect::new(inner.x-scroll,inner.y,if focused{width.max(inner.w)+1.0}else{inner.w},inner.h),&text_style);}
        if focused&&self.input.caret_visible(self.now,out.changed,address||!display.is_empty()){self.c.line(Point::new(inner.x+cx-scroll+0.5,y+2.0),Point::new(inner.x+cx-scroll+0.5,y+height-2.0),palette::TEXT,1.5);}
        self.c.unclip();out
    }

    fn text_input_styled(&mut self, id: WidgetId, r: Rect, buf: &mut String, placeholder: &str, max_chars: usize, look: InputLook) -> TextInputResp {
        let resp = self.interact(id, r, CursorKind::Text);
        let mut out = TextInputResp::default();
        let pointer_active=(self.input.just_pressed&&self.input.pressed_id==Some(id))||(self.input.down&&self.input.pressed_id==Some(id))||self.input.released.is_some_and(|(owner,_)|owner==id);
        if pointer_active {
        let pointer_inner=match look{InputLook::Area|InputLook::AreaTop=>r.inset(3.0,0.0),InputLook::Row{..}=>Rect::new(r.x+36.0,r.y,(r.w-46.0).max(1.0),r.h),_=>r.inset(10.0,0.0)};
        let area=matches!(look,InputLook::Area|InputLook::AreaTop);
        let pointer_style=if area{text_area_style()}else{TextStyle::new(crate::gfx::ts::BODY)};
        let masked=if matches!(look,InputLook::Secret){"•".repeat(buf.chars().count())}else{String::new()};
        let display=if matches!(look,InputLook::Secret){masked.as_str()}else{buf.as_str()};
        let width=if area{pointer_inner.w}else{10000.0};
        let(cx,cy,ch)=self.c.text_caret(display,&pointer_style,width,if self.input.focused==Some(id){self.input.caret}else{display.chars().count()});
        let text_h=if area{self.c.measure(display,&pointer_style,width).1}else{ch};
        let origin=Point::new(pointer_inner.x-if !area&&self.input.focused==Some(id){(cx-pointer_inner.w+3.0).max(0.0)}else{0.0},
            pointer_inner.y+if area{(if look==InputLook::AreaTop{0.0}else{((pointer_inner.h-text_h)/2.0).max(0.0)})-(cy+ch-pointer_inner.h+3.0).max(0.0)}else{0.0});
        self.text_pointer(id,display,&pointer_style,width,origin,area);
        }else if self.input.focused==Some(id)&&self.input.selection_owner!=Some(id){self.input.selection_owner=Some(id);self.input.selection_anchor=None;self.input.text_scroll=0.0;}
        // Also support widgets activated programmatically through their click ID.
        if resp.clicked&&self.input.focused!=Some(id){self.input.focused=Some(id);self.input.selection_owner=Some(id);self.input.selection_anchor=None;self.input.caret=buf.chars().count();self.input.selection_all=false;self.wants_focus=Some(true);}
        let focused = self.input.focused == Some(id);
        out.focused = focused;
        if focused {
            let keys = std::mem::take(&mut self.input.keys);
            let mut chars: Vec<char> = buf.chars().collect();
            let mut caret = self.input.caret.min(chars.len());
            let mut anchor=if self.input.selection_all{Some(0)}else{self.input.selection_anchor.map(|a|a.min(chars.len()))};
            for k in keys {
                let selected=anchor.filter(|a|*a!=caret).map(|a|a.min(caret)..a.max(caret));
                match k {
                    KeyInput::Char(c) if !c.is_control() || (matches!(look,InputLook::Area|InputLook::AreaTop) && c=='\n') => {
                        if let Some(range)=selected{caret=range.start;chars.drain(range);}anchor=None;
                        if chars.len() < max_chars {
                            chars.insert(caret, c);
                            caret += 1;
                            out.changed = true;
                        }
                    }
                    KeyInput::Backspace|KeyInput::Delete|KeyInput::Cut=>{
                        if k==KeyInput::Cut{if let Some(range)=selected.clone(){self.input.clipboard_write=Some(chars[range].iter().collect());}}
                        if let Some(range)=selected{caret=range.start;chars.drain(range);anchor=None;out.changed=true;}
                        else if k==KeyInput::Backspace&&caret>0{caret-=1;chars.remove(caret);anchor=None;out.changed=true;}
                        else if k==KeyInput::Delete&&caret<chars.len(){chars.remove(caret);anchor=None;out.changed=true;}
                    }
                    KeyInput::Copy=>{if let Some(range)=selected{self.input.clipboard_write=Some(chars[range].iter().collect());}},
                    KeyInput::Left|KeyInput::Right|KeyInput::Home|KeyInput::End|KeyInput::SelectLeft|KeyInput::SelectRight|KeyInput::SelectHome|KeyInput::SelectEnd=>{
                        let extend=matches!(k,KeyInput::SelectLeft|KeyInput::SelectRight|KeyInput::SelectHome|KeyInput::SelectEnd);
                        let next=match k{KeyInput::Left if selected.is_some()=>selected.as_ref().unwrap().start,KeyInput::Right if selected.is_some()=>selected.as_ref().unwrap().end,KeyInput::Left|KeyInput::SelectLeft=>caret.saturating_sub(1),KeyInput::Right|KeyInput::SelectRight=>(caret+1).min(chars.len()),KeyInput::Home|KeyInput::SelectHome=>0,_=>chars.len()};
                        if extend{anchor=Some(anchor.unwrap_or(caret));}else{anchor=None;}caret=next;
                    }
                    KeyInput::SelectAll => {anchor=Some(0);caret=chars.len();}
                    KeyInput::Enter => out.submitted = true,
                    KeyInput::Escape => {
                        out.cancelled = true;
                    }
                    _ => {}
                }
            }
            *buf = chars.into_iter().collect();
            self.input.caret = caret;
            self.input.selection_anchor=anchor;
            self.input.selection_all=anchor==Some(0)&&caret==buf.chars().count()&&caret>0;
            if out.cancelled {
                self.input.focused = None;
                self.wants_focus = Some(false);
            }
            if out.changed || out.submitted || out.cancelled {
                self.repaint = true;
            }
        }
        // Draw.
        let inner = match look {
            InputLook::Area|InputLook::AreaTop => r.inset(3.0,0.0),
            InputLook::Box | InputLook::Secret => {
                let bg = if focused { Color::white(0.13) } else if resp.hovered { Color::white(0.1) } else { palette::SURFACE };
                self.c.fill_rrect(r, 9.0, bg);
                if focused {
                    self.c.stroke_rrect(r, 9.0, Color::white(0.28), 1.0);
                }
                r.inset(10.0, 0.0)
            }
            InputLook::Row { accent, icon } => {
                if focused || resp.hovered {
                    self.c.fill_rrect(r, 9.0, if focused { Color::white(0.10) } else { palette::SURFACE });
                }
                if focused {
                    self.c.stroke_rrect(r, 9.0, accent, 1.5);
                }
                if let Some(i) = icon {
                    let col = if buf.is_empty() { palette::TEXT_FAINT } else { accent };
                    self.c.icon(i, Rect::from_center(Point::new(r.x + 18.0, r.center().y), 16.0, 16.0), col);
                }
                Rect::new(r.x + 36.0, r.y, (r.w - 46.0).max(0.0), r.h)
            }
        };
        let area=matches!(look,InputLook::Area|InputLook::AreaTop);
        let st=if area {text_area_style()}else{TextStyle::new(crate::gfx::ts::BODY)};
        let masked=if matches!(look,InputLook::Secret){"•".repeat(buf.chars().count())}else{String::new()};
        let display=if matches!(look,InputLook::Secret){masked.as_str()}else{buf.as_str()};
        let caret=if focused{self.input.caret}else{display.chars().count()};
        let (cx,cy,ch)=if display.is_empty(){(0.0,0.0,st.size*1.3)}else{self.c.text_caret(display,&st,if area{inner.w}else{10000.0},caret)};
        let text_h=if area && !display.is_empty(){self.c.measure(display,&st,inner.w).1}else{st.size*1.3};
        let offset_y=if area&&look!=InputLook::AreaTop{((inner.h-text_h)/2.0).max(0.0)}else{0.0};
        let scroll_y=if area{(cy+ch-inner.h+3.0).max(0.0)}else{0.0};
        let scroll_x=if !area && focused{(cx-inner.w+3.0).max(0.0)}else{0.0};
        self.c.clip(inner);
        if focused{if let Some(a)=self.input.selection_anchor{for hit in self.c.text_selection_wrapped(display,&st,if area{inner.w}else{10000.0},a.min(self.input.caret),a.max(self.input.caret)){
            let y=if area{inner.y+offset_y-scroll_y}else{inner.center().y-ch/2.0};
            self.c.fill_rect(Rect::new(inner.x+hit.x-scroll_x,y+hit.y,hit.w,hit.h),Color::rgba(0.2,0.4,0.7,0.6));
        }}}
        if buf.is_empty(){let mut placeholder_style=st;placeholder_style.vcenter=look!=InputLook::AreaTop;self.c.text(placeholder,inner,&placeholder_style.color(palette::TEXT_FAINT));}
        else if area {let (_,h)=self.c.measure(display,&st,inner.w);self.c.text(display,Rect::new(inner.x,inner.y+offset_y-scroll_y,inner.w,h.max(inner.h)),&st);}
        else {let(w,_)=self.c.measure(display,&st,10000.0);self.c.text(display,Rect::new(inner.x-scroll_x,inner.y,if focused{w.max(inner.w)+1.0}else{inner.w},inner.h),&st);}
        if focused&&self.input.caret_visible(self.now,out.changed,!area||!buf.is_empty()) {let x=inner.x+cx-scroll_x+0.5;let y=if area{inner.y+offset_y+cy-scroll_y}else{inner.center().y-ch/2.0};self.c.line(Point::new(x,y+2.0),Point::new(x,y+ch-2.0),Color::white(0.9),1.5);}
        self.c.unclip();
        out
    }

    /// Horizontal progress bar. When `seekable` it can be clicked or dragged:
    /// while held it shows (and reports) the fraction under the pointer, and
    /// the release reports where to seek, even if the pointer slid off the bar.
    pub fn progress_bar(&mut self, id: WidgetId, r: Rect, progress: f32, fill: Color, seekable: bool) -> Scrub {
        let hit = Rect::new(r.x, r.y - 6.0, r.w, r.h + 12.0);
        let resp = if seekable { self.button(id, hit) } else { Resp::default() };
        let frac = |x: f32| if r.w > 0.0 { ((x - r.x) / r.w).clamp(0.0, 1.0) } else { 0.0 };
        let held = if resp.pressed { self.input.mouse.map(|m| frac(m.x)) } else { None };
        let began = resp.pressed && self.input.just_pressed;
        let released = self.input.released.filter(|(rid, _)| seekable && *rid == id).map(|(_, p)| frac(p.x));
        let thick = if (resp.hovered || held.is_some()) && seekable { r.h + 2.0 } else { r.h };
        let bar = Rect::new(r.x, r.center().y - thick / 2.0, r.w, thick);
        self.c.fill_rrect(bar, thick / 2.0, palette::TRACK);
        let p = held.or(released).unwrap_or(progress).clamp(0.0, 1.0);
        if p > 0.0 {
            self.c.fill_rrect(Rect::new(bar.x, bar.y, (bar.w * p).max(thick), bar.h), thick / 2.0, fill);
        }
        if released.is_some() {
            self.repaint = true;
        }
        Scrub { held, released, began }
    }

    /// Release keyboard focus (e.g. after submitting).
    pub fn blur(&mut self) {
        if self.input.focused.is_some() {
            self.input.focused = None;
            self.wants_focus = Some(false);
        }
    }
}

#[cfg(test)]
mod caret_tests {
    use super::*;
    #[test]
    fn caret_blinks_on_boundaries_restarts_after_edit_and_rests_with_selection_or_no_focus(){
        let id=wid("ai.prompt");let mut input=Input::default();input.focused=Some(id);input.caret_key=Some((id,0,None));input.caret_period=Some(0.5);
        assert!(input.caret_visible(0.0,false,true));assert_eq!(input.caret_wake,Some(0.5));
        assert!(!input.caret_visible(0.51,false,true));assert_eq!(input.caret_wake,Some(1.0));
        assert!(input.caret_visible(0.6,true,true));assert_eq!(input.caret_wake,Some(1.1));
        input.caret=2;input.selection_anchor=Some(0);assert!(!input.caret_visible(0.7,false,true));assert!(input.caret_wake.is_none());
        input.focused=None;assert!(!input.caret_visible(0.8,false,true));assert!(input.caret_wake.is_none());
        input.focused=Some(id);input.caret_key=Some((id,2,Some(0)));input.selection_anchor=None;input.caret_period=None;
        assert!(input.caret_visible(100.0,false,true));assert!(input.caret_wake.is_none(),"disabled Windows blinking stays static without a wake");
        assert!(!input.caret_visible(101.0,false,false));
        assert_eq!(input.focused,Some(id));assert!(input.caret_key.is_none());assert!(input.caret_wake.is_none());
        assert!(input.caret_visible(102.0,true,true));assert_eq!(input.caret_epoch,102.0);
    }
}
