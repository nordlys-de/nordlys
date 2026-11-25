//! The top bar: workspaces and what's on stage on the left, the clock in
//! the centre, layout and battery on the right. Matches the canvas.
//!
//! The same state, update and view run as a layer-shell surface on Wayland
//! and as a plain preview window anywhere else.

use crate::icons::{self, icon};
use crate::sources::{self, Battery, Compositor, Desk};
use iced::alignment::Horizontal;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{button, container, row, text};
use iced::{
    Alignment, Background, Border, Color, Element, Font, Length, Radians, Shadow, Subscription,
    Task, Vector,
};
use iced_layershell::to_layer_message;
use nordlys_config::{Config, Layout, Palette, Rgb};
use std::time::Duration;

pub const INTER: Font = Font::with_name("Inter");
pub(crate) const INTER_SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..INTER
};
pub(crate) const MONO: Font = Font::with_name("JetBrains Mono");

/// Bar height on the canvas, plus the gap above it.
pub const BAR_HEIGHT: u32 = 40;
pub const BAR_GAP: u32 = 10;

pub struct Bar {
    palette: Palette,
    layout: Layout,
    compositor: Compositor,
    desk: Desk,
    battery: Option<Battery>,
    clock: String,
    /// Draw a stand-in wallpaper behind the bar (preview window only).
    preview: bool,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Switch(usize),
}

impl Bar {
    pub fn new(config: &Config, preview: bool) -> Self {
        let compositor = Compositor::detect();
        let mut bar = Self {
            palette: config.palette(),
            layout: config.stage.layout,
            desk: Desk::default(),
            battery: None,
            clock: String::new(),
            compositor,
            preview,
        };
        bar.refresh();
        bar
    }

    fn refresh(&mut self) {
        self.desk = self.compositor.desk();
        self.battery = sources::battery();
        self.clock = sources::clock_text(chrono::Local::now());
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.refresh(),
            Message::Switch(i) => {
                if let Some(ws) = self.desk.workspaces.get(i) {
                    self.compositor.switch_to(ws);
                }
                self.refresh();
            }
            // Layer-shell control messages added by `to_layer_message`.
            _ => {}
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let p = &self.palette;
        let glass = Glass::of(p);

        let workspaces = row(self.desk.workspaces.iter().enumerate().map(|(i, ws)| {
            let (bg, fg) = if ws.active {
                (color(p.accent), color(p.on_accent))
            } else {
                (glass.chip, color(p.muted))
            };
            let label = if ws.name.is_empty() || ws.name == ws.id.to_string() {
                row![text(ws.id.to_string()).font(MONO).size(12)]
            } else {
                row![
                    text(ws.id.to_string()).font(MONO).size(12),
                    text(ws.name.clone())
                        .font(if ws.active { INTER_SEMIBOLD } else { INTER })
                        .size(12)
                ]
                .spacing(5)
            };
            button(label.align_y(Alignment::Center))
                .padding([5, 10])
                .on_press(Message::Switch(i))
                .style(move |_, status| button::Style {
                    background: Some(Background::Color(match status {
                        button::Status::Hovered if !ws.active => glass.chip_hover,
                        _ => bg,
                    })),
                    text_color: fg,
                    border: Border {
                        radius: 8.0.into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                })
                .into()
        }))
        .spacing(4);

        let stage = row![
            icon(icons::WINDOW, 14).color(color(p.dim)),
            text(
                self.desk
                    .stage
                    .clone()
                    .unwrap_or_else(|| "Empty stage".into())
            )
            .font(INTER)
            .size(13)
            .color(color(p.muted)),
        ]
        .spacing(7)
        .align_y(Alignment::Center);

        let clock = text(self.clock.clone())
            .font(INTER_SEMIBOLD)
            .size(13)
            .color(color(p.text));

        let layout_label = match (self.layout, &self.compositor) {
            (_, Compositor::Demo) => "preview · no compositor".to_owned(),
            (Layout::Auto, c) => format!("auto · {}", c.label()),
            (l, c) => format!("{} · {}", layout_name(l), c.label()),
        };
        let chip_icon = match self.compositor {
            Compositor::Demo => icons::MONITOR,
            _ => layout_icon(self.layout),
        };
        let chip = container(
            row![
                icon(chip_icon, 12).color(color(p.muted)),
                text(layout_label).font(MONO).size(11).color(color(p.muted)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([4, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(glass.chip)),
            border: Border {
                radius: 6.0.into(),
                width: 1.0,
                color: glass.chip_border,
            },
            ..container::Style::default()
        });

        let mut right = row![chip].spacing(12).align_y(Alignment::Center);
        if let Some(b) = self.battery {
            right = right.push(battery(p, b));
        }

        let bar = container(
            row![
                container(
                    row![workspaces, stage]
                        .spacing(12)
                        .align_y(Alignment::Center)
                )
                .width(Length::Fill),
                clock,
                container(right)
                    .width(Length::Fill)
                    .align_x(Horizontal::Right),
            ]
            .align_y(Alignment::Center),
        )
        .padding([0, 10])
        .width(Length::Fill)
        .height(BAR_HEIGHT as f32)
        .align_y(Alignment::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(glass.bar)),
            border: Border {
                radius: 12.0.into(),
                width: 1.0,
                color: glass.bar_border,
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.18),
                offset: Vector::new(0.0, 6.0),
                blur_radius: 24.0,
            },
            ..container::Style::default()
        });

        let framed = container(bar)
            .padding([BAR_GAP as f32, 12.0])
            .width(Length::Fill);
        if !self.preview {
            return framed.into();
        }
        // A stand-in for the blurred wallpaper so the glass reads in a plain window.
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
        let ground = color(p.ground);
        container(framed)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Gradient(
                    Linear::new(Radians(1.2))
                        .add_stop(0.0, a)
                        .add_stop(0.55, ground)
                        .add_stop(1.0, b)
                        .into(),
                )),
                ..container::Style::default()
            })
            .into()
    }

    pub fn style(&self, _theme: &iced::Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: if self.preview {
                color(self.palette.ground)
            } else {
                Color::TRANSPARENT
            },
            text_color: color(self.palette.text),
        }
    }
}

/// Translucent fills from the canvas. The compositor supplies the blur.
#[derive(Clone, Copy)]
struct Glass {
    bar: Color,
    bar_border: Color,
    chip: Color,
    chip_hover: Color,
    chip_border: Color,
}

impl Glass {
    fn of(p: &Palette) -> Self {
        if p.dark {
            Glass {
                bar: Color::from_rgba8(18, 22, 30, 0.50),
                bar_border: Color::from_rgba8(255, 255, 255, 0.08),
                chip: Color::from_rgba8(255, 255, 255, 0.07),
                chip_hover: Color::from_rgba8(255, 255, 255, 0.13),
                chip_border: Color::from_rgba8(255, 255, 255, 0.10),
            }
        } else {
            Glass {
                bar: Color::from_rgba8(255, 255, 255, 0.50),
                bar_border: Color::from_rgba8(255, 255, 255, 0.75),
                chip: Color::from_rgba8(20, 24, 33, 0.06),
                chip_hover: Color::from_rgba8(20, 24, 33, 0.11),
                chip_border: Color::from_rgba8(20, 24, 33, 0.12),
            }
        }
    }
}

fn battery<'a>(p: &Palette, b: Battery) -> Element<'a, Message> {
    let tone = if b.charging {
        color(p.accent)
    } else if b.percent <= 15 {
        color(p.ansi[1])
    } else {
        color(p.text)
    };
    row![
        icon(icons::battery(b.percent, b.charging), 16).color(tone),
        text(format!("{}%", b.percent))
            .font(INTER)
            .size(12)
            .color(color(p.muted)),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

fn layout_icon(l: Layout) -> char {
    match l {
        Layout::Auto => icons::GRID,
        Layout::Ultrawide => icons::MONITOR,
        Layout::Rails => icons::TWO_UP,
        Layout::Peek => icons::WINDOW,
    }
}

fn layout_name(l: Layout) -> &'static str {
    match l {
        Layout::Auto => "auto",
        Layout::Ultrawide => "ultrawide",
        Layout::Rails => "rails",
        Layout::Peek => "peek",
    }
}

pub(crate) fn color(c: Rgb) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
}
