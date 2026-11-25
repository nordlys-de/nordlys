//! The left shelf as a 76 px rail (the 27-inch "rails" layout on the
//! canvas): windows waiting on this workspace, most recent first, with slot
//! numbers. Hover one for its name and title; click to bring it on stage.
//!
//! Live thumbnails need the compositor to share window pictures, so they
//! come with nordlys-comp. Until then the flyout shows name and title.

use crate::bar::{INTER, INTER_SEMIBOLD, color};
use crate::sources::{Compositor, ShelfWindow};
use crate::ui::{self, Rail};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{Space, column, container, mouse_area, row, stack, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Subscription, Task};
use iced_layershell::to_layer_message;
use nordlys_config::{Config, Palette};
use std::time::Duration;

pub const RAIL: f32 = 76.0;
/// Space the rail reserves at the screen edge: rail plus its margin.
pub const ZONE: u32 = 88;
const TILE: f32 = 56.0;
const GAP: f32 = 10.0;
const FLYOUT: f32 = 300.0;
const MAX_SLOTS: usize = 8;

pub struct Shelf {
    palette: Palette,
    compositor: Compositor,
    windows: Vec<ShelfWindow>,
    hover: Option<usize>,
    preview: bool,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Hover(usize),
    Unhover,
    Pick(usize),
}

impl Shelf {
    pub fn new(config: &Config, preview: bool) -> Self {
        let compositor = Compositor::detect();
        let windows = compositor.shelf();
        Self {
            palette: config.palette(),
            compositor,
            // The preview window opens with slot 1's flyout showing.
            hover: (preview && !windows.is_empty()).then_some(0),
            windows,
            preview,
        }
    }

    /// The layer surface widens while a flyout is open, and shrinks back after.
    fn resize(&self) -> Task<Message> {
        if self.preview {
            return Task::none();
        }
        let width = if self.hover.is_some() {
            ZONE + FLYOUT as u32 + 12
        } else {
            ZONE
        };
        Task::done(Message::SizeChange((width, 0)))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.windows = self.compositor.shelf();
                if self.hover.is_some_and(|i| i >= self.windows.len()) {
                    self.hover = None;
                    return self.resize();
                }
            }
            Message::Hover(i) => {
                let was = self.hover.is_some();
                self.hover = Some(i);
                if !was {
                    return self.resize();
                }
            }
            Message::Unhover => {
                self.hover = None;
                return self.resize();
            }
            Message::Pick(i) => {
                if let Some(w) = self.windows.get(i) {
                    self.compositor.bring(w);
                }
                self.hover = None;
                self.windows = self.compositor.shelf();
                return self.resize();
            }
            _ => {}
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(750)).map(|_| Message::Tick)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let r = Rail::of(&self.palette);

        let mut tiles = column![ui::section_label("Shelf", r)]
            .spacing(GAP)
            .align_x(Alignment::Center);
        for (i, w) in self.windows.iter().take(MAX_SLOTS).enumerate() {
            tiles = tiles.push(slot(i, w, self.hover == Some(i), r));
        }
        if self.windows.is_empty() {
            tiles = tiles.push(
                container(Space::new().width(TILE - 4.0).height(TILE - 4.0)).style(move |_| {
                    container::Style {
                        border: Border {
                            radius: 14.0.into(),
                            width: 1.5,
                            color: r.chip_border,
                        },
                        ..container::Style::default()
                    }
                }),
            );
        }
        let rail = container(tiles)
            .width(RAIL)
            .height(Length::Fill)
            .padding([12, 0])
            .align_x(Horizontal::Center)
            .style(r.rail_style());

        let mut layout = row![rail].spacing(12);
        if let Some(w) = self.hover.and_then(|i| self.windows.get(i).map(|w| (i, w))) {
            layout = layout.push(flyout(w.0, w.1, r));
        }
        let page = container(layout)
            .padding(iced::Padding {
                left: 12.0,
                top: 12.0,
                bottom: 12.0,
                right: 0.0,
            })
            .height(Length::Fill);

        if self.preview {
            let bg = ui::wallpaper(&self.palette);
            return container(page)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(bg),
                    ..container::Style::default()
                })
                .into();
        }
        page.into()
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

fn slot(i: usize, w: &ShelfWindow, hovered: bool, r: Rail) -> Element<'static, Message> {
    let tint = ui::tint(&w.app);
    let initial = w
        .app
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let tile = container(text(initial).font(INTER_SEMIBOLD).size(20).color(r.text))
        .width(TILE)
        .height(TILE)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(Color { a: 0.24, ..tint })),
            border: Border {
                radius: 14.0.into(),
                width: 1.5,
                color: if hovered { tint } else { Color::TRANSPARENT },
            },
            ..container::Style::default()
        });
    let badge = container(
        container(
            text((i + 1).to_string())
                .font(crate::bar::MONO)
                .size(10)
                .color(r.muted),
        )
        .width(18)
        .height(18)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(r.badge)),
            border: Border {
                radius: 9.0.into(),
                width: 1.0,
                color: r.chip_border,
            },
            ..container::Style::default()
        }),
    )
    .width(TILE + 6.0)
    .height(TILE + 6.0)
    .align_x(Horizontal::Right)
    .align_y(Vertical::Bottom);
    mouse_area(stack![container(tile).padding(3), badge])
        .on_enter(Message::Hover(i))
        .on_exit(Message::Unhover)
        .on_press(Message::Pick(i))
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

fn flyout(i: usize, w: &ShelfWindow, r: Rail) -> Element<'static, Message> {
    // Line the flyout up with its tile: rail padding, header, then one tile pitch per slot.
    let top = 12.0 + 12.0 + GAP + i as f32 * (TILE + 6.0 + GAP) - 4.0;
    let card = container(
        column![
            row![
                container(Space::new().width(8).height(8)).style({
                    let t = ui::tint(&w.app);
                    move |_| container::Style {
                        background: Some(Background::Color(t)),
                        border: Border {
                            radius: 4.0.into(),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    }
                }),
                text(w.app.clone())
                    .font(INTER_SEMIBOLD)
                    .size(13)
                    .color(r.text),
                Space::new().width(Length::Fill),
                ui::kbd(&format!("Super Alt {}", i + 1), r),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(w.title.clone()).font(INTER).size(12).color(r.muted),
            text("Click to bring it on stage")
                .font(INTER)
                .size(11)
                .color(r.dim),
        ]
        .spacing(8),
    )
    .width(FLYOUT)
    .padding([12, 14])
    .style(r.panel_style());
    column![Space::new().height(top.max(0.0)), card].into()
}
