//! Small drawing pieces shared by the rails.

use crate::bar::{INTER_SEMIBOLD, MONO, color};
use iced::gradient::Linear;
use iced::widget::{container, text};
use iced::{Background, Border, Color, Element, Radians};
use nordlys_config::{Palette, Rgb};

/// Tile tints from the canvas, picked by name so an app keeps its colour.
const TINTS: [u32; 7] = [
    0xC9A7FF, 0xA8D8A0, 0xF4B183, 0x8AB4FF, 0xF2A7C3, 0x8EE0D4, 0xF6C177,
];

pub fn tint(name: &str) -> Color {
    let h = name
        .bytes()
        .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
    color(Rgb::hex_u32(TINTS[h % TINTS.len()]))
}

/// Glass colours for rails and pull-out panels.
#[derive(Clone, Copy)]
pub struct Rail {
    pub rail: Color,
    pub rail_border: Color,
    pub panel: Color,
    pub chip: Color,
    pub chip_border: Color,
    pub badge: Color,
    pub text: Color,
    pub muted: Color,
    pub dim: Color,
    pub accent: Color,
    pub on_accent: Color,
}

impl Rail {
    pub fn of(p: &Palette) -> Self {
        let base = Rail {
            rail: Color::from_rgba8(18, 22, 30, 0.50),
            rail_border: Color::from_rgba8(255, 255, 255, 0.09),
            panel: Color::from_rgba8(24, 28, 38, 0.82),
            chip: Color::from_rgba8(255, 255, 255, 0.07),
            chip_border: Color::from_rgba8(255, 255, 255, 0.10),
            badge: Color::from_rgb8(0x1B, 0x20, 0x29),
            text: color(p.text),
            muted: color(p.muted),
            dim: color(p.dim),
            accent: color(p.accent),
            on_accent: color(p.on_accent),
        };
        if p.dark {
            base
        } else {
            Rail {
                rail: Color::from_rgba8(255, 255, 255, 0.42),
                rail_border: Color::from_rgba8(255, 255, 255, 0.85),
                panel: Color::from_rgba8(255, 255, 255, 0.82),
                chip: Color::from_rgba8(20, 24, 33, 0.06),
                chip_border: Color::from_rgba8(20, 24, 33, 0.12),
                badge: Color::WHITE,
                ..base
            }
        }
    }

    pub fn rail_style(self) -> impl Fn(&iced::Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(self.rail)),
            border: Border {
                radius: 16.0.into(),
                width: 1.0,
                color: self.rail_border,
            },
            ..container::Style::default()
        }
    }

    pub fn panel_style(self) -> impl Fn(&iced::Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(self.panel)),
            border: Border {
                radius: 18.0.into(),
                width: 1.0,
                color: self.rail_border,
            },
            shadow: iced::Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.40),
                offset: iced::Vector::new(0.0, 24.0),
                blur_radius: 70.0,
            },
            ..container::Style::default()
        }
    }
}

pub fn kbd<'a, M: 'a>(label: &str, r: Rail) -> Element<'a, M> {
    container(text(label.to_owned()).font(MONO).size(11).color(r.muted))
        .padding([3, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(r.chip)),
            border: Border {
                radius: 5.0.into(),
                width: 1.0,
                color: r.chip_border,
            },
            ..container::Style::default()
        })
        .into()
}

pub fn section_label<'a, M: 'a>(label: &str, r: Rail) -> Element<'a, M> {
    text(label.to_uppercase())
        .font(INTER_SEMIBOLD)
        .size(10)
        .color(r.dim)
        .into()
}

/// Stand-in for the blurred wallpaper in preview windows.
pub fn wallpaper(p: &Palette) -> Background {
    let (a, b) = if p.dark {
        (
            Color::from_rgb8(0x2F, 0x4F, 0xC9),
            Color::from_rgb8(0x6D, 0x3F, 0xD9),
        )
    } else {
        (
            Color::from_rgb8(0x8F, 0xAE, 0xFF),
            Color::from_rgb8(0xC7, 0xB2, 0xFF),
        )
    };
    Background::Gradient(
        Linear::new(Radians(2.2))
            .add_stop(0.0, a)
            .add_stop(0.55, color(p.ground))
            .add_stop(1.0, b)
            .into(),
    )
}
