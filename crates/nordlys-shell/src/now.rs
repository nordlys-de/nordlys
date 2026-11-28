//! Now playing and notifications, in the bar's top right. The chips live in
//! `bar.rs`; this file holds what they open: the media panel and the
//! notifications panel, plus the numbers the bar needs to size them.

use crate::bar::{INTER, INTER_SEMIBOLD};
use crate::icons::{self, icon};
use crate::system::{self, Notices, NowPlaying};
use crate::ui::{self, Rail};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Background, Border, Color, Element, Length};

/// Width of both panels.
pub const WIDTH: f32 = 340.0;
const MEDIA_HEIGHT: u32 = 130;
const NOTES_HEIGHT: u32 = 230;
const NOTES_EMPTY_HEIGHT: u32 = 180;
/// Newest notifications listed in the panel.
const LISTED: usize = 3;

/// Which of the two panels is open under the chips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pop {
    Media,
    Notices,
}

/// What the panels can do. The bar turns these into system calls.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    PlayPause,
    Next,
    Dnd,
    ShowLast,
    Clear,
}

/// The height the bar reserves for `pop`, so the surface fits the panel.
pub fn height(pop: Pop, notices: Option<&Notices>) -> u32 {
    match pop {
        Pop::Media => MEDIA_HEIGHT,
        Pop::Notices => match notices {
            Some(n) if !n.recent.is_empty() => NOTES_HEIGHT,
            _ => NOTES_EMPTY_HEIGHT,
        },
    }
}

/// Notifications the bell's badge counts: what arrived since the panel was
/// last open with the Nordlys daemon, or what dunst holds without it.
pub fn unread(n: &Notices, seen: u32) -> u32 {
    if n.recent.is_empty() {
        n.waiting + n.history
    } else {
        n.recent.iter().filter(|r| r.id > seen).count() as u32
    }
}

/// The track's title, shortened for the chip in the bar.
pub fn chip_title(np: &NowPlaying) -> String {
    const MAX: usize = 26;
    let title = np.title.trim();
    if title.chars().count() <= MAX {
        title.to_owned()
    } else {
        let cut: String = title.chars().take(MAX - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// The now-playing panel: the track, and play/pause and next.
pub fn media_panel(
    np: Option<&NowPlaying>,
    p: &nordlys_config::Palette,
) -> Element<'static, Action> {
    let r = Rail::of(p);
    let amber = Color::from_rgb8(0xF6, 0xC1, 0x77);
    match np {
        Some(np) => row![
            container(icon(icons::MUSIC, 22).color(amber))
                .width(48)
                .height(48)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| container::Style {
                    background: Some(Background::Color(Color { a: 0.35, ..amber })),
                    border: Border {
                        radius: 12.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
            column![
                text(np.title.clone())
                    .font(INTER_SEMIBOLD)
                    .size(14)
                    .color(r.text)
                    .wrapping(iced::widget::text::Wrapping::None),
                text(np.artist.clone())
                    .font(INTER)
                    .size(12)
                    .color(r.muted)
                    .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(4)
            .width(Length::Fill)
            .clip(true),
            round(
                if np.playing {
                    icons::PAUSE
                } else {
                    icons::PLAY
                },
                Action::PlayPause,
                true,
                r
            ),
            round(icons::SKIP_FORWARD, Action::Next, false, r),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into(),
        None => row![
            icon(icons::MUSIC, 16).color(r.dim),
            text("Nothing playing").font(INTER).size(13).color(r.muted),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into(),
    }
}

/// The notifications panel: the summary, Show last, Clear, Do not disturb,
/// and the newest few notifications.
pub fn notes_panel(
    notices: Option<&Notices>,
    seen: u32,
    p: &nordlys_config::Palette,
) -> Element<'static, Action> {
    let r = Rail::of(p);
    let Some(n) = notices else {
        return text("No notification daemon")
            .font(INTER)
            .size(12)
            .color(r.dim)
            .into();
    };
    let summary = match (n.waiting, n.history, n.recent.len()) {
        (0, 0, 0) => "All caught up".to_owned(),
        (0, h, _) => format!("{h} earlier"),
        (w, h, _) => format!("{w} waiting · {h} earlier"),
    };
    let unread = unread(n, seen);
    let head = row![
        container(icon(if n.dnd { icons::BELL_OFF } else { icons::BELL }, 18).color(r.muted))
            .width(28)
            .align_x(Horizontal::Center),
        column![
            text("Notifications")
                .font(INTER_SEMIBOLD)
                .size(13)
                .color(r.text),
            text(if unread > 0 {
                format!("{unread} new · {summary}")
            } else {
                summary
            })
            .font(INTER)
            .size(12)
            .color(r.muted),
        ]
        .spacing(3)
        .width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    let actions = row![
        pill("Show last", Action::ShowLast, n.history > 0, r),
        pill("Clear", Action::Clear, !n.recent.is_empty(), r),
        pill(
            if n.dnd {
                "Do not disturb · On"
            } else {
                "Do not disturb · Off"
            },
            Action::Dnd,
            true,
            r
        ),
    ]
    .spacing(8);

    let mut list = column![head, actions].spacing(12);
    for item in n.recent.iter().take(LISTED) {
        list = list.push(recent_row(item, r));
    }
    list.into()
}

/// One line per notification, as on the canvas: dot, summary, then the body.
fn recent_row(item: &system::Recent, r: Rail) -> Element<'static, Action> {
    let tint = ui::tint(&item.app);
    let detail = if item.body.is_empty() {
        item.app.clone()
    } else {
        item.body.replace('\n', " ")
    };
    row![
        container(
            container(Space::new().width(8).height(8)).style(move |_| container::Style {
                background: Some(Background::Color(tint)),
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
        )
        .width(28)
        .align_x(Horizontal::Center),
        container(
            row![
                text(item.summary.clone())
                    .font(INTER_SEMIBOLD)
                    .size(13)
                    .color(r.text)
                    .wrapping(iced::widget::text::Wrapping::None),
                text(format!("· {detail}"))
                    .font(INTER)
                    .size(13)
                    .color(r.muted)
                    .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(6)
        )
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

fn pill(label: &str, action: Action, enabled: bool, r: Rail) -> Element<'static, Action> {
    let b = button(
        container(text(label.to_owned()).font(INTER).size(12))
            .align_x(Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([7, 10])
    .width(Length::Fill)
    .style(move |_, status| button::Style {
        background: Some(Background::Color(if status == button::Status::Hovered {
            r.chip
        } else {
            Color::TRANSPARENT
        })),
        text_color: if enabled { r.muted } else { r.dim },
        border: Border {
            radius: 10.0.into(),
            width: 1.0,
            color: r.chip_border,
        },
        ..button::Style::default()
    });
    if enabled { b.on_press(action) } else { b }.into()
}

fn round(glyph: char, action: Action, primary: bool, r: Rail) -> Element<'static, Action> {
    button(
        container(icon(glyph, 15))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center),
    )
    .width(36)
    .height(36)
    .padding(0)
    .on_press(action)
    .style(move |_, _| button::Style {
        background: Some(Background::Color(if primary { r.accent } else { r.chip })),
        text_color: if primary { r.on_accent } else { r.text },
        border: Border {
            radius: 18.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    })
    .into()
}
