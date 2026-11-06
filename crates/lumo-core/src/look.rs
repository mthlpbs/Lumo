//! Painting vocabulary shared by the Classic and Pill themes.

use crate::geom::{palette, Color};
use crate::settings::Theme;

#[derive(Debug, PartialEq)]
pub struct Look {
    pub ground: Color,
    pub ink: Color,
    pub accent: Option<Color>,
    pub border: Color,
    pub font: Option<&'static str>,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Notch,
    Pill,
}

pub const CLASSIC: Look = Look {
    ground: Color::black(1.0), ink: Color::white(1.0), accent: None,
    border: palette::BORDER, font: None, shape: Shape::Notch,
};

pub const PILL: Look = Look {
    ground: Color::black(1.0), ink: Color::white(1.0), accent: None,
    border: Color::white(0.10), font: None, shape: Shape::Pill,
};

pub fn of(theme: Theme) -> &'static Look {
    match theme { Theme::Classic => &CLASSIC, Theme::Pill => &PILL }
}

impl Look {
    pub fn paint(&self, c: Color) -> Color { c }
    pub fn paint_text(&self, c: Color) -> Color { c }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_paints_what_it_is_given() {
        for c in [Color::white(0.62), Color::black(1.0), palette::GREEN, Color::rgba(0.16, 0.16, 0.16, 1.0), Color::hex(0x123456)] {
            assert_eq!(CLASSIC.paint(c), c);
            assert_eq!(CLASSIC.paint_text(c), c);
        }
    }

    #[test]
    fn pill_floats_and_paints_like_classic() {
        assert_eq!(PILL.shape, Shape::Pill);
        assert_eq!(*of(Theme::Pill), PILL);
        assert_eq!(CLASSIC.shape, Shape::Notch);
        for c in [Color::white(0.62), Color::black(1.0), palette::GREEN, Color::hex(0x123456)] {
            assert_eq!(PILL.paint(c), c);
            assert_eq!(PILL.paint_text(c), c);
        }
    }
}
