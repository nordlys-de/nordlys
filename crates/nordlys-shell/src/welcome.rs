//! The welcome: what the Stage is and the keys that move it, shown once at
//! the first login (`nordlys-shell welcome --first-run`) and again with
//! Super+F1 or "Welcome and keys" in the launcher.
//!
//! Shown is remembered in `$XDG_STATE_HOME/nordlys/welcomed`.

use crate::bar::{INTER, INTER_SEMIBOLD, MONO, color};
use crate::icons::{self, icon};
use iced::alignment::{Horizontal, Vertical};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Length, Shadow, Subscription, Task,
    Vector, event,
};
use iced_layershell::to_layer_message;
use nordlys_config::{Config, Palette};
use std::path::PathBuf;

pub struct Welcome {
    palette: Palette,
    preview: bool,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Close,
    /// A click on the card, which mustn't close it.
    Keep,
}

/// The stage and shelf, in three steps.
const IDEAS: [(char, &str, &str); 3] = [
    (
        icons::WINDOW,
        "The stage",
        "The window you open takes the centre of the screen. One at a time, \
         full attention.",
    ),
    (
        icons::SHELF,
        "The shelf",
        "The window it replaces waits at the front of the shelf beside it. \
         Click one there, or press Super K, to bring it back.",
    ),
    (
        icons::TWO_UP,
        "Side by side",
        "Super T shows the stage tiled, to work with two windows at once. \
         Press it again for the stage alone.",
    ),
];

/// The keys worth knowing on day one. Super is the Windows or ⌘ key.
const KEYS: [(&str, &str); 14] = [
    ("Super Space", "Launcher: apps, files, commands"),
    ("Super Enter", "Terminal"),
    ("Super K", "Previous window to the stage"),
    ("Super J", "Cycle through the shelf"),
    ("Super T", "Tiled view on and off"),
    ("Super Q", "Close the window"),
    ("Super 1–9", "Go to a workspace (Shift: take the window)"),
    ("Super N · B", "Files · browser"),
    ("Super V", "Clipboard history"),
    ("Super Shift S", "Screenshot a region"),
    ("Super R", "Record a region (again to stop)"),
    ("Super ,", "Settings"),
    ("Super L", "Lock"),
    ("Super F1", "This page"),
];

fn marker() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("nordlys/welcomed"))
}

/// Has the welcome been shown before?
pub fn seen() -> bool {
    marker().is_some_and(|m| m.exists())
}

/// Remember it was shown, so the first-run start skips it from now on.
pub fn mark_seen() {
    if let Some(m) = marker() {
        if let Some(dir) = m.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(m, b"");
    }
}

impl Welcome {
    pub fn new(config: &Config, preview: bool) -> (Self, Task<Message>) {
        mark_seen();
        (
            Self {
                palette: config.palette(),
                preview,
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Close => iced::exit(),
            _ => Task::none(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        event::listen_with(|event, _, _| match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::Escape | Named::Enter | Named::Space),
                ..
            }) => Some(Message::Close),
            _ => None,
        })
    }

    pub fn view(&self) -> Element<'_, Message> {
        let p = &self.palette;
        let accent = color(p.accent);
        let (ink, muted, dim) = (color(p.text), color(p.muted), color(p.dim));
        let (panel, line, chip, scrim) = if p.dark {
            (
                Color::from_rgba8(22, 26, 36, 0.86),
                Color::from_rgba8(255, 255, 255, 0.10),
                Color::from_rgba8(255, 255, 255, 0.07),
                Color::from_rgba8(5, 7, 11, 0.45),
            )
        } else {
            (
                Color::from_rgba8(250, 251, 255, 0.90),
                Color::from_rgba8(20, 24, 33, 0.10),
                Color::from_rgba8(20, 24, 33, 0.06),
                Color::from_rgba8(20, 24, 33, 0.25),
            )
        };

        let heading = column![
            text("Welcome to Nordlys")
                .font(INTER_SEMIBOLD)
                .size(26)
                .color(ink),
            text("A keyboard-first desktop where the newest window always takes the stage.")
                .font(INTER)
                .size(14)
                .color(muted),
        ]
        .spacing(6);

        let ideas = row(IDEAS.iter().map(|&(glyph, title, body)| {
            container(
                column![
                    icon(glyph, 22).color(accent),
                    text(title).font(INTER_SEMIBOLD).size(15).color(ink),
                    text(body).font(INTER).size(13).color(muted),
                ]
                .spacing(8),
            )
            .width(Length::Fill)
            .padding(16)
            .style(move |_| container::Style {
                background: Some(Background::Color(chip)),
                border: Border {
                    radius: 12.0.into(),
                    width: 1.0,
                    color: line,
                },
                ..container::Style::default()
            })
            .into()
        }))
        .spacing(12);

        let key_row = |(keys, what): (&'static str, &'static str)| -> Element<'_, Message> {
            row![
                container(text(keys).font(MONO).size(12).color(ink))
                    .width(130)
                    .padding([3, 0]),
                text(what).font(INTER).size(13).color(muted),
            ]
            .align_y(Alignment::Center)
            .into()
        };
        let half = KEYS.len().div_ceil(2);
        let keys = row![
            column(KEYS[..half].iter().copied().map(key_row))
                .spacing(6)
                .width(Length::Fill),
            column(KEYS[half..].iter().copied().map(key_row))
                .spacing(6)
                .width(Length::Fill),
        ]
        .spacing(24);

        let on_accent = color(p.on_accent);
        let start = button(
            text("Get started")
                .font(INTER_SEMIBOLD)
                .size(14)
                .color(on_accent),
        )
        .padding([9, 20])
        .on_press(Message::Close)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                Color { a: 0.85, ..accent }
            } else {
                accent
            })),
            border: Border {
                radius: 9.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        });
        let footer = row![
            text(
                "With the mouse: Apps on the bar opens the launcher, the clock a calendar, \
                  and the × near a window's corner closes it. Super F1 brings this back."
            )
            .font(INTER)
            .size(12)
            .color(dim)
            .width(Length::Fill),
            start,
        ]
        .spacing(20)
        .align_y(Alignment::Center);

        let divider = move || {
            container(Space::new().width(Length::Fill).height(1)).style(move |_| container::Style {
                background: Some(Background::Color(line)),
                ..container::Style::default()
            })
        };
        let card = container(
            column![
                heading,
                ideas,
                divider(),
                text("KEYS").font(INTER_SEMIBOLD).size(11).color(dim),
                keys,
                divider(),
                footer,
            ]
            .spacing(18),
        )
        .width(820)
        .padding(28)
        .style(move |_| container::Style {
            background: Some(Background::Color(panel)),
            border: Border {
                radius: 18.0.into(),
                width: 1.0,
                color: line,
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.5),
                offset: Vector::new(0.0, 30.0),
                blur_radius: 90.0,
            },
            ..container::Style::default()
        });

        let ground = color(p.ground);
        let preview = self.preview;
        // A click beside the card closes it, like the launcher.
        mouse_area(
            container(mouse_area(card).on_press(Message::Keep))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| container::Style {
                    background: Some(Background::Color(if preview { ground } else { scrim })),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::Close)
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
