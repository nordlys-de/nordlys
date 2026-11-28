//! The left shelf as a 76 px rail (the 27-inch "rails" layout on the
//! canvas): windows waiting on this workspace, most recent first, with slot
//! numbers. Hover one for its name and title; click to bring it on stage.
//!
//! On nordlys-comp the flyout shows a live picture of the window: the
//! compositor keeps drawing shelved windows small (at `stage.shelf_fps`)
//! into files beside its socket, and the flyout re-reads the hovered one's
//! a few times a second while it's open. Elsewhere it's name and title.

use crate::apps;
use crate::bar::{INTER, INTER_SEMIBOLD, color};
use crate::events;
use crate::icons::{self, icon};
use crate::sources::{Compositor, ShelfWindow};
use crate::system;
use crate::ui::{self, Rail};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{Space, button, column, container, image, mouse_area, row, stack, text};
use iced::{Alignment, Background, Border, Color, ContentFit, Element, Length, Subscription, Task};
use iced_layershell::to_layer_message;
use nordlys_config::{BarConfig, Config, Gaps, Palette};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

pub const RAIL: f32 = 76.0;
/// Space the rail reserves at the screen edge: rail plus its margin.
pub const ZONE: u32 = 88;

/// Where the rail sits, as layer margins (top, right, bottom, left) and the
/// space it reserves. It starts `bar.gap` below the top bar and is indented
/// `gaps.shelf` from the screen's left edge.
pub fn placement(bar: &BarConfig, gaps: &Gaps) -> ((i32, i32, i32, i32), i32) {
    let (below, side) = (bar.gap as i32, gaps.shelf as i32);
    ((below, 0, 0, side), ZONE as i32 + side)
}
const TILE: f32 = 56.0;
const GAP: f32 = 10.0;
const FLYOUT: f32 = 300.0;
const MAX_SLOTS: usize = 8;

pub struct Shelf {
    palette: Palette,
    bar: BarConfig,
    gaps: Gaps,
    compositor: Compositor,
    windows: Vec<ShelfWindow>,
    hover: Option<usize>,
    preview: bool,
    reads: system::Reads,
    /// The screen this shelf is on, when it was given one.
    screen: Option<String>,
    /// The hovered window's picture: its id, the file's age, the pixels.
    picture: Option<(String, SystemTime, image::Handle)>,
}

/// What reading a preview file found.
#[derive(Debug, Clone)]
pub enum Picture {
    New(SystemTime, image::Handle),
    Unchanged,
    Missing,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    /// The compositor reported a change (or the slow fallback timer fired).
    Changed,
    Windows(Option<Vec<ShelfWindow>>),
    /// nordlys.toml was saved.
    Config(Config),
    Hover(usize),
    Unhover,
    Pick(usize),
    Close(usize),
    /// Time to re-read the hovered window's picture.
    Look,
    Picture(String, Picture),
}

impl Shelf {
    pub fn new(config: &Config, preview: bool, screen: Option<String>) -> Self {
        let compositor = Compositor::detect();
        let windows = compositor.shelf(screen.as_deref());
        Self {
            palette: config.palette(),
            bar: config.bar.clone(),
            gaps: config.gaps.clone(),
            compositor,
            // The preview window opens with slot 1's flyout showing.
            hover: (preview && !windows.is_empty()).then_some(0),
            windows,
            preview,
            reads: system::Reads::default(),
            screen,
            picture: None,
        }
    }

    /// Read the hovered window's picture, unless the file hasn't changed.
    fn look(&self) -> Task<Message> {
        let Some(w) = self.hover.and_then(|i| self.windows.get(i)) else {
            return Task::none();
        };
        let Some(path) = w.preview.clone() else {
            return Task::none();
        };
        let seen = self
            .picture
            .as_ref()
            .filter(|(id, ..)| *id == w.id)
            .map(|(_, t, _)| *t);
        let id = w.id.clone();
        Task::perform(
            system::background(move || read_picture(&path, seen)),
            move |p| Message::Picture(id.clone(), p.unwrap_or(Picture::Missing)),
        )
    }

    fn refresh(&mut self) -> Task<Message> {
        if !self.reads.start() {
            return Task::none();
        }
        let (c, screen) = (self.compositor.clone(), self.screen.clone());
        Task::perform(
            system::background(move || c.shelf(screen.as_deref())),
            Message::Windows,
        )
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
            Message::Changed => return self.refresh(),
            Message::Config(c) => {
                self.palette = c.palette();
                self.bar = c.bar.clone();
                self.gaps = c.gaps.clone();
                if !self.preview {
                    let (margin, exclusive) = placement(&self.bar, &self.gaps);
                    return Task::batch([
                        Task::done(Message::MarginChange(margin)),
                        Task::done(Message::ExclusiveZoneChange(exclusive)),
                    ]);
                }
            }
            Message::Windows(windows) => {
                let again = self.reads.done();
                if let Some(w) = windows {
                    self.windows = w;
                }
                let mut tasks = vec![];
                if self.hover.is_some_and(|i| i >= self.windows.len()) {
                    self.hover = None;
                    tasks.push(self.resize());
                }
                if again {
                    tasks.push(self.refresh());
                }
                return Task::batch(tasks);
            }
            Message::Hover(i) => {
                let was = self.hover.is_some();
                self.hover = Some(i);
                let look = self.look();
                if !was {
                    return Task::batch([self.resize(), look]);
                }
                return look;
            }
            Message::Look => return self.look(),
            Message::Picture(id, p) => match p {
                Picture::New(t, h) => self.picture = Some((id, t, h)),
                Picture::Missing => {
                    if self.picture.as_ref().is_some_and(|(seen, ..)| *seen == id) {
                        self.picture = None;
                    }
                }
                Picture::Unchanged => {}
            },
            Message::Unhover => {
                self.hover = None;
                return self.resize();
            }
            Message::Close(i) => {
                if let Some(w) = self.windows.get(i) {
                    self.compositor.close(&w.id);
                }
                // The window takes a moment to go; drop its tile now.
                if i < self.windows.len() {
                    self.windows.remove(i);
                }
                self.hover = None;
                return self.resize();
            }
            Message::Pick(i) => {
                if let Some(w) = self.windows.get(i) {
                    self.compositor.bring(w);
                }
                self.hover = None;
                return Task::batch([self.resize(), self.refresh()]);
            }
            _ => {}
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            events::changes(&self.compositor).map(|_| Message::Changed),
            events::config_changes().map(Message::Config),
            // In case an event is ever missed.
            iced::time::every(Duration::from_secs(15)).map(|_| Message::Changed),
            if self.hover.is_some() && matches!(self.compositor, Compositor::Nordlys(_)) {
                iced::time::every(Duration::from_millis(250)).map(|_| Message::Look)
            } else {
                Subscription::none()
            },
        ])
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
                            radius: r.corner(14.0).into(),
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
            let picture = self
                .picture
                .as_ref()
                .filter(|(id, ..)| *id == w.1.id)
                .map(|(.., h)| h.clone());
            layout = layout.push(flyout(w.0, w.1, picture, r));
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
    let art = apps::icon_for(&w.app);
    let initial = w
        .app
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let face: Element<'static, Message> = match &art {
        Some(path) => ui::app_art(path, TILE * 0.8),
        None => text(initial).font(INTER_SEMIBOLD).size(20).color(r.text).into(),
    };
    let tile = container(face)
        .width(TILE)
        .height(TILE)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(if art.is_some() {
                Color::TRANSPARENT
            } else {
                Color { a: 0.24, ..tint }
            })),
            border: Border {
                radius: r.corner(14.0).into(),
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
                radius: r.corner(9.0).into(),
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
    // The tile picks; the × on top closes. Hover is tracked around both, so
    // moving onto the × doesn't count as leaving.
    let pick = mouse_area(stack![container(tile).padding(3), badge])
        .on_press(Message::Pick(i))
        .interaction(iced::mouse::Interaction::Pointer);
    let mut layers = stack![pick];
    if hovered {
        layers = layers.push(
            container(close_badge(i, r))
                .width(TILE + 6.0)
                .align_x(Horizontal::Right),
        );
    }
    mouse_area(layers)
        .on_enter(Message::Hover(i))
        .on_exit(Message::Unhover)
        .into()
}

fn close_badge(i: usize, r: Rail) -> Element<'static, Message> {
    button(
        container(icon(icons::CLOSE, 12))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center),
    )
    .width(20)
    .height(20)
    .padding(0)
    .on_press(Message::Close(i))
    .style(move |_, status| button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => Color::from_rgb8(0xE0, 0x5A, 0x6D),
            _ => r.badge,
        })),
        text_color: r.text,
        border: Border {
            radius: r.corner(10.0).into(),
            width: 1.0,
            color: r.chip_border,
        },
        ..button::Style::default()
    })
    .into()
}

fn flyout(
    i: usize,
    w: &ShelfWindow,
    picture: Option<image::Handle>,
    r: Rail,
) -> Element<'static, Message> {
    // Line the flyout up with its tile: rail padding, header, then one tile pitch per slot.
    let top = 12.0 + 12.0 + GAP + i as f32 * (TILE + 6.0 + GAP) - 4.0;
    let shot = picture.map(|h| {
        container(
            image(h)
                .width(Length::Fill)
                .height(Length::Shrink)
                .content_fit(ContentFit::Contain),
        )
        .width(Length::Fill)
        .max_height(FLYOUT * 0.75)
        .align_x(Horizontal::Center)
    });
    let card = container(
        column![
            row![
                container(Space::new().width(8).height(8)).style({
                    let t = ui::tint(&w.app);
                    move |_| container::Style {
                        background: Some(Background::Color(t)),
                        border: Border {
                            radius: r.corner(4.0).into(),
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
        .push(shot)
        .spacing(8),
    )
    .width(FLYOUT)
    .padding([12, 14])
    .style(r.panel_style());
    column![Space::new().height(top.max(0.0)), card].into()
}

/// The picture at `path` as pixels, if it changed since `seen`.
fn read_picture(path: &PathBuf, seen: Option<SystemTime>) -> Picture {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return Picture::Missing;
    };
    if seen == Some(modified) {
        return Picture::Unchanged;
    }
    match std::fs::read(path)
        .ok()
        .and_then(|b| nordlys_ipc::decode_preview(&b))
    {
        Some((w, h, px)) => Picture::New(modified, image::Handle::from_rgba(w, h, px)),
        None => Picture::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_read_once_until_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("nordlys-shelf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("term.rgba");
        assert!(matches!(read_picture(&path, None), Picture::Missing));

        std::fs::write(&path, nordlys_ipc::encode_preview(2, 1, &[9; 8])).unwrap();
        let Picture::New(t, _) = read_picture(&path, None) else {
            panic!("a fresh picture should be read");
        };
        assert!(matches!(read_picture(&path, Some(t)), Picture::Unchanged));

        std::fs::write(&path, b"junk").unwrap();
        assert!(matches!(read_picture(&path, None), Picture::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
