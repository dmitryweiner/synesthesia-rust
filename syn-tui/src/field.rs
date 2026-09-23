//! The picture in the terminal: two pixels a cell, one above the other, as
//! the foreground and background colours of `▀` (GRAPHICS.md decision 1).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use syn_core::sim::Image;

/// How colours reach the terminal. Truecolor is the picture as computed;
/// 256 colours is the fallback for a terminal that does not say it can do
/// better (decision 2: fewer colours do not make a terminal cheaper, so they
/// are never chosen for speed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    TrueColor,
    Indexed,
}

impl ColorMode {
    /// `setting` is the config's `viz_color`: "truecolor", "256", or anything
    /// else to ask the terminal (`COLORTERM`).
    pub fn pick(setting: &str, colorterm: Option<&str>) -> Self {
        match setting {
            "truecolor" => Self::TrueColor,
            "256" => Self::Indexed,
            _ => match colorterm {
                Some(c) if c.contains("truecolor") || c.contains("24bit") => Self::TrueColor,
                _ => Self::Indexed,
            },
        }
    }

    fn color(self, [r, g, b]: [u8; 3]) -> Color {
        match self {
            Self::TrueColor => Color::Rgb(r, g, b),
            Self::Indexed => Color::Indexed(xterm256(r, g, b)),
        }
    }
}

/// The xterm levels of the 6×6×6 cube.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn cube_index(c: u8) -> usize {
    CUBE.iter().enumerate().min_by_key(|(_, &l)| (i32::from(l) - i32::from(c)).abs()).map_or(0, |(i, _)| i)
}

/// The nearest xterm-256 colour: the closer of the cube entry and the grey
/// ramp (232–255, 8 + 10k).
pub fn xterm256(r: u8, g: u8, b: u8) -> u8 {
    let (ri, gi, bi) = (cube_index(r), cube_index(g), cube_index(b));
    let dist = |x: [u8; 3]| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(x[0], r) + d(x[1], g) + d(x[2], b)
    };
    let cube = [CUBE[ri], CUBE[gi], CUBE[bi]];
    let mean = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
    let k = ((mean.saturating_sub(8) + 5) / 10).min(23) as u8;
    let grey = 8 + 10 * k;
    if dist([grey; 3]) < dist(cube) {
        232 + k
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

pub struct FieldView<'a> {
    pub image: &'a Image,
    pub mode: ColorMode,
}

impl Widget for FieldView<'_> {
    /// Fills `area` with the image; if their sizes differ (a resize not yet
    /// caught up with), the image is stretched by nearest neighbour.
    fn render(self, area: Rect, buf: &mut Buffer) {
        let img = self.image;
        if img.w == 0 || img.h == 0 || area.width == 0 || area.height == 0 {
            return;
        }
        let rows = usize::from(area.height) * 2;
        for cy in 0..area.height {
            for cx in 0..area.width {
                let x = usize::from(cx) * img.w / usize::from(area.width);
                let top = (usize::from(cy) * 2) * img.h / rows;
                let bottom = (usize::from(cy) * 2 + 1) * img.h / rows;
                buf[(area.x + cx, area.y + cy)]
                    .set_char('▀')
                    .set_fg(self.mode.color(img.at(x, top)))
                    .set_bg(self.mode.color(img.at(x, bottom)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_mode_follows_the_setting_then_the_terminal() {
        assert_eq!(ColorMode::pick("256", Some("truecolor")), ColorMode::Indexed);
        assert_eq!(ColorMode::pick("truecolor", None), ColorMode::TrueColor);
        assert_eq!(ColorMode::pick("auto", Some("truecolor")), ColorMode::TrueColor);
        assert_eq!(ColorMode::pick("auto", Some("24bit")), ColorMode::TrueColor);
        assert_eq!(ColorMode::pick("auto", None), ColorMode::Indexed);
    }

    #[test]
    fn xterm256_picks_cube_colours_and_greys() {
        assert_eq!(xterm256(0, 0, 0), 16);
        assert_eq!(xterm256(255, 255, 255), 231);
        assert_eq!(xterm256(255, 0, 0), 196);
        assert_eq!(xterm256(128, 128, 128), 244); // grey 128 sits on the ramp
        assert_eq!(xterm256(95, 135, 175), 16 + 36 + 12 + 3);
    }

    #[test]
    fn each_cell_carries_two_pixels() {
        let mut img = Image::new(2, 4);
        img.rgb =
            vec![[1, 0, 0], [2, 0, 0], [3, 0, 0], [4, 0, 0], [5, 0, 0], [6, 0, 0], [7, 0, 0], [8, 0, 0]];
        let area = Rect::new(0, 0, 2, 2);
        let mut buf = Buffer::empty(area);
        FieldView { image: &img, mode: ColorMode::TrueColor }.render(area, &mut buf);
        let cell = &buf[(1, 1)];
        assert_eq!(cell.symbol(), "▀");
        assert_eq!(cell.fg, Color::Rgb(6, 0, 0));
        assert_eq!(cell.bg, Color::Rgb(8, 0, 0));
    }
}
