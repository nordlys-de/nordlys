//! The top bar: workspaces and what's on stage on the left, the clock in
//! the centre, layout and battery on the right. Matches the canvas.
//!
//! The same state, update and view run as a layer-shell surface on Wayland
//! and as a plain preview window anywhere else.

use crate::apps;
use crate::calendar;
use crate::icons::{self, icon};
use crate::sources::{self, Battery, Compositor, Desk};
use chrono::{Datelike, NaiveDate};
use iced::alignment::Horizontal;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{button, column, container, mouse_area, row, text};
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

/// This same binary's launcher, so it works without anything on PATH.
fn launcher_command() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "nordlys-shell".into());
    format!("{} launcher", apps::shell_quote(&exe))
}

pub struct Bar {
    palette: Palette,
    layout: Layout,
    compositor: Compositor,
    desk: Desk,
    battery: Option<Battery>,
    clock: String,
    /// The month on show while the calendar is open.
    month: Option<NaiveDate>,
    /// Draw a stand-in wallpaper behind the bar (preview window only).
    preview: bool,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Switch(usize),
    Launcher,
    /// Open or close the calendar under the clock.
    Calendar,
    /// Move the calendar by this many months.
    Month(i32),
    Today,
    /// A click on the calendar itself, which mustn't close it.
    Keep,
}

/// The surface's height with the calendar closed and open.
pub const SURFACE: u32 = BAR_HEIGHT + BAR_GAP * 2;
const SURFACE_OPEN: u32 = SURFACE + CALENDAR_HEIGHT;
const CALENDAR_HEIGHT: u32 = 320;
const CALENDAR_WIDTH: f32 = 300.0;

impl Bar {
    pub fn new(config: &Config, preview: bool) -> Self {
        let compositor = Compositor::detect();
        let mut bar = Self {
            palette: config.palette(),
            layout: config.stage.layout,
            desk: Desk::default(),
            battery: None,
            clock: String::new(),
            month: None,
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
            Message::Launcher => {
                // The preview has no layer shell to open it in.
                if !self.preview
                    && let Err(e) = apps::spawn_shell(&launcher_command())
                {
                    eprintln!("nordlys-shell: can't open the launcher: {e}");
                }
            }
            Message::Switch(i) => {
                if let Some(ws) = self.desk.workspaces.get(i) {
                    self.compositor.switch_to(ws);
                }
                self.refresh();
            }
            Message::Calendar => {
                self.month = match self.month {
                    Some(_) => None,
                    None => Some(calendar::first_of(today())),
                };
                return self.resize();
            }
            Message::Month(by) => {
                self.month = self.month.map(|m| calendar::shift(m, by));
            }
            Message::Today => {
                self.month = self.month.map(|_| calendar::first_of(today()));
            }
            // Layer-shell control messages added by `to_layer_message`.
            _ => {}
        }
        Task::none()
    }

    /// The surface grows to hold the calendar. The space the bar reserves
    /// stays the same, so windows don't move.
    fn resize(&self) -> Task<Message> {
        if self.preview {
            return Task::none();
        }
        let height = if self.month.is_some() {
            SURFACE_OPEN
        } else {
            SURFACE
        };
        Task::done(Message::SizeChange((0, height)))
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

        let apps_button = button(
            row![
                icon(icons::GRID, 13),
                text("Apps").font(INTER_SEMIBOLD).size(12)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([5, 10])
        .on_press(Message::Launcher)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                glass.chip_hover
            } else {
                glass.chip
            })),
            text_color: color(p.text),
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        });

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

        let open = self.month.is_some();
        let clock = button(text(self.clock.clone()).font(INTER_SEMIBOLD).size(13))
            .padding([5, 10])
            .on_press(Message::Calendar)
            .style(move |_, status| button::Style {
                background: (open || status == button::Status::Hovered)
                    .then_some(Background::Color(glass.chip_hover)),
                text_color: color(p.text),
                border: Border {
                    radius: 8.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            });

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
                    row![apps_button, workspaces, stage]
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
        let framed: Element<'_, Message> = match self.month {
            Some(month) => column![
                framed,
                // A click beside the calendar closes it.
                mouse_area(
                    container(
                        mouse_area(calendar_panel(p, glass, month, today()))
                            .on_press(Message::Keep)
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Horizontal::Center)
                )
                .on_press(Message::Calendar),
            ]
            .into(),
            None => framed.into(),
        };
        if !self.preview {
            return framed;
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

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn calendar_panel<'a>(
    p: &Palette,
    glass: Glass,
    month: NaiveDate,
    today: NaiveDate,
) -> Element<'a, Message> {
    let (accent, on_accent) = (color(p.accent), color(p.on_accent));
    let (ink, dim) = (color(p.text), color(p.dim));
    let step = move |label: &'static str, message: Message| {
        button(text(label).font(INTER_SEMIBOLD).size(14))
            .padding([2, 10])
            .on_press(message)
            .style(move |_, status| button::Style {
                background: Some(Background::Color(if status == button::Status::Hovered {
                    glass.chip_hover
                } else {
                    glass.chip
                })),
                text_color: ink,
                border: Border {
                    radius: 7.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            })
    };
    let title = button(
        text(month.format("%B %Y").to_string())
            .font(INTER_SEMIBOLD)
            .size(14),
    )
    .padding(0)
    .on_press(Message::Today)
    .style(move |_, _| button::Style {
        text_color: ink,
        ..button::Style::default()
    });
    let cell = |content: Element<'a, Message>| {
        container(content)
            .width(Length::Fill)
            .height(30)
            .align_x(Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center)
    };
    let names = row(["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
        .map(|d| cell(text(d).font(INTER).size(11).color(dim).into()).into()));
    let mut grid = column![
        row![
            container(title).width(Length::Fill),
            step("‹", Message::Month(-1)),
            step("›", Message::Month(1)),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        names,
    ]
    .spacing(6);
    for week in calendar::weeks(month) {
        grid = grid.push(row(week.map(|day| {
            let is_today = day == today;
            let ink = if is_today {
                on_accent
            } else if day.month() == month.month() {
                ink
            } else {
                dim
            };
            let number = container(text(day.day().to_string()).font(INTER).size(12).color(ink))
                .width(28)
                .height(28)
                .align_x(Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center)
                .style(move |_| container::Style {
                    background: is_today.then_some(Background::Color(accent)),
                    border: Border {
                        radius: 14.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                });
            cell(number.into()).into()
        })));
    }
    let ground = color(p.ground);
    container(grid)
        .width(CALENDAR_WIDTH)
        .padding(14)
        .style(move |_| container::Style {
            // Darker than the bar: it sits over windows, not just wallpaper.
            background: Some(Background::Color(Color { a: 0.94, ..ground })),
            border: Border {
                radius: 14.0.into(),
                width: 1.0,
                color: glass.bar_border,
            },
            ..container::Style::default()
        })
        .into()
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
