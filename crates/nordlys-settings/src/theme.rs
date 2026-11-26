//! Colours and widget styles from the Nordlys palette, so Settings looks
//! like the rest of the shell and follows the theme live.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector};
use nordlys_config::{Config, Rgb};

pub const INTER: Font = Font::with_name("Inter");
pub const SEMIBOLD: Font = Font {
    weight: iced::font::Weight::Semibold,
    ..INTER
};
pub const BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..INTER
};
pub const MONO: Font = Font::with_name("JetBrains Mono");

pub fn color(c: Rgb) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// The palette as iced colours, plus the glass setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct C {
    pub dark: bool,
    pub ground: Color,
    pub surface: Color,
    pub raised: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub dim: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    pub glass: bool,
}

impl C {
    pub fn of(cfg: &Config) -> Self {
        let p = cfg.palette();
        Self {
            dark: p.dark,
            ground: color(p.ground),
            surface: color(p.surface),
            raised: color(p.raised),
            border: color(p.border),
            text: color(p.text),
            muted: color(p.muted),
            dim: color(p.dim),
            accent: color(p.accent),
            on_accent: color(p.on_accent),
            good: color(p.ansi[2]),
            warn: color(p.ansi[3]),
            bad: color(p.ansi[1]),
            glass: !cfg.theme.reduce_transparency,
        }
    }

    /// The iced theme the stock widgets (toggles, sliders, lists) draw with.
    pub fn theme(&self) -> Theme {
        Theme::custom(
            "Nordlys",
            iced::theme::Palette {
                background: self.surface,
                text: self.text,
                primary: self.accent,
                success: self.good,
                warning: self.warn,
                danger: self.bad,
            },
        )
    }

    pub fn window(&self) -> Color {
        if self.glass {
            alpha(self.ground, if self.dark { 0.86 } else { 0.90 })
        } else {
            self.ground
        }
    }

    /// Stand-in for the aurora wallpaper in previews.
    pub fn wallpaper(&self) -> Background {
        let (a, b) = if self.dark {
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
            iced::gradient::Linear::new(iced::Radians(2.2))
                .add_stop(0.0, a)
                .add_stop(0.55, self.ground)
                .add_stop(1.0, b)
                .into(),
        )
    }

    // ---- Containers ----

    pub fn sidebar(self) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(alpha(
                self.surface,
                if self.glass { 0.55 } else { 1.0 },
            ))),
            border: Border {
                width: 0.0,
                ..Border::default()
            },
            ..container::Style::default()
        }
    }

    pub fn card(self) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(alpha(
                self.surface,
                if self.glass { 0.78 } else { 1.0 },
            ))),
            border: Border {
                radius: 14.0.into(),
                width: 1.0,
                color: self.border,
            },
            ..container::Style::default()
        }
    }

    /// A tinted note: errors, warnings, tips.
    pub fn note(self, tint: Color) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(alpha(tint, 0.12))),
            border: Border {
                radius: 12.0.into(),
                width: 1.0,
                color: alpha(tint, 0.35),
            },
            text_color: Some(self.text),
            ..container::Style::default()
        }
    }

    pub fn chip(self, tint: Color) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(alpha(tint, 0.16))),
            border: Border {
                radius: 99.0.into(),
                ..Border::default()
            },
            text_color: Some(tint),
            ..container::Style::default()
        }
    }

    pub fn swatch(self, fill: Color, ring: bool) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(fill)),
            border: Border {
                radius: 99.0.into(),
                width: if ring { 3.0 } else { 1.0 },
                color: if ring { self.text } else { self.border },
            },
            ..container::Style::default()
        }
    }

    pub fn toast(self, tint: Color) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(self.raised)),
            border: Border {
                radius: 12.0.into(),
                width: 1.0,
                color: alpha(tint, 0.6),
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.35),
                offset: Vector::new(0.0, 10.0),
                blur_radius: 30.0,
            },
            text_color: Some(self.text),
            ..container::Style::default()
        }
    }

    pub fn scrim(self) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(Color::from_rgba8(0, 0, 0, 0.45))),
            ..container::Style::default()
        }
    }

    pub fn dialog(self) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(self.surface)),
            border: Border {
                radius: 18.0.into(),
                width: 1.0,
                color: self.border,
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.45),
                offset: Vector::new(0.0, 24.0),
                blur_radius: 60.0,
            },
            text_color: Some(self.text),
            ..container::Style::default()
        }
    }

    // ---- Buttons ----

    pub fn button(self, kind: Kind) -> impl Fn(&Theme, button::Status) -> button::Style {
        move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let disabled = status == button::Status::Disabled;
            let (bg, fg, border) = match kind {
                Kind::Primary => (
                    if hovered {
                        mix(self.accent, self.text, 0.15)
                    } else {
                        self.accent
                    },
                    self.on_accent,
                    Color::TRANSPARENT,
                ),
                Kind::Secondary => (
                    if hovered {
                        mix(self.raised, self.text, 0.08)
                    } else {
                        self.raised
                    },
                    self.text,
                    self.border,
                ),
                Kind::Danger => (
                    alpha(self.bad, if hovered { 0.30 } else { 0.18 }),
                    self.bad,
                    alpha(self.bad, 0.35),
                ),
                Kind::Ghost => (
                    if hovered {
                        alpha(self.text, 0.07)
                    } else {
                        Color::TRANSPARENT
                    },
                    self.muted,
                    Color::TRANSPARENT,
                ),
                Kind::Nav(selected) => (
                    if selected {
                        alpha(self.accent, 0.16)
                    } else if hovered {
                        alpha(self.text, 0.06)
                    } else {
                        Color::TRANSPARENT
                    },
                    if selected { self.accent } else { self.text },
                    Color::TRANSPARENT,
                ),
                Kind::Segment(selected) => (
                    if selected {
                        self.accent
                    } else if hovered {
                        alpha(self.text, 0.07)
                    } else {
                        Color::TRANSPARENT
                    },
                    if selected { self.on_accent } else { self.muted },
                    Color::TRANSPARENT,
                ),
            };
            let radius = match kind {
                Kind::Segment(_) => 8.0,
                _ => 10.0,
            };
            button::Style {
                background: Some(Background::Color(if disabled {
                    alpha(bg, bg.a * 0.4)
                } else {
                    bg
                })),
                text_color: if disabled { alpha(fg, 0.45) } else { fg },
                border: Border {
                    radius: radius.into(),
                    width: if border.a > 0.0 { 1.0 } else { 0.0 },
                    color: border,
                },
                shadow: Shadow::default(),
                snap: true,
            }
        }
    }

    pub fn segments(self) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            background: Some(Background::Color(self.raised)),
            border: Border {
                radius: 10.0.into(),
                width: 1.0,
                color: self.border,
            },
            ..container::Style::default()
        }
    }

    pub fn input(self) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
        move |_, status| {
            let focused = matches!(status, text_input::Status::Focused { .. });
            text_input::Style {
                background: Background::Color(self.raised),
                border: Border {
                    radius: 10.0.into(),
                    width: 1.0,
                    color: if focused { self.accent } else { self.border },
                },
                icon: self.dim,
                placeholder: self.dim,
                value: self.text,
                selection: alpha(self.accent, 0.35),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
    Ghost,
    Nav(bool),
    Segment(bool),
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a,
    }
}

/// Accent choices: the defaults of both modes and the palette's tints.
pub const ACCENTS: [(&str, u32); 8] = [
    ("Aurora", 0x8AB4FF),
    ("Cobalt", 0x2F5BD8),
    ("Violet", 0xC9A7FF),
    ("Fjord", 0x8EE0D4),
    ("Moss", 0xA8D8A0),
    ("Amber", 0xF6C177),
    ("Ember", 0xF4B183),
    ("Rose", 0xF2A7C3),
];
