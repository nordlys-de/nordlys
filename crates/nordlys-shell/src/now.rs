//! Now playing and notifications, in the bar's top right. The chips live in
//! `bar.rs`; this file holds what they open: the media panel and the
//! notifications panel, plus the numbers the bar needs to size them.

use crate::apps;
use crate::bar::{INTER, INTER_SEMIBOLD};
use crate::icons::{self, Symbol, icon};
use crate::notify::Entry;
use crate::system::{Notices, NowPlaying};
use crate::ui::{self, Rail};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::text::Wrapping;
use iced::widget::{Space, button, column, container, mouse_area, row, scrollable, text};
use iced::{Alignment, Background, Border, Color, Element, Length};
use std::collections::BTreeSet;

/// Width of both panels.
pub const WIDTH: f32 = 340.0;
const MEDIA_HEIGHT: u32 = 130;
const NOTES_HEIGHT: u32 = 440;
const NOTES_EMPTY_HEIGHT: u32 = 180;
/// Notifications the panel reads from the daemon.
const LISTED: usize = 40;
/// Snooze choices, in minutes.
const SNOOZE: [(u32, &str); 3] = [(15, "15 min"), (60, "1 hour"), (180, "3 hours")];

/// Which of the two panels is open under the chips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pop {
    Media,
    Notices,
}

/// What the panels can do. The bar turns most of these into system calls;
/// `Expand` and `Menu` only change what the notifications panel shows open.
#[derive(Debug, Clone)]
pub enum Action {
    PlayPause,
    Next,
    Dnd,
    ShowLast,
    Clear,
    /// Click one of a notification's own buttons (id, key).
    Invoke(u32, String),
    Dismiss(u32),
    /// Snooze a notification for this many minutes.
    Snooze(u32, u32),
    /// Stop (true) or resume (false) an app's notifications.
    Mute(String, bool),
    ClearApp(String),
    /// Open or close an app's group.
    Expand(String),
    /// Open or close a notification's snooze choices.
    Menu(u32),
    /// The pointer moved onto, or off, a card or a notification.
    Hover(Hot),
    Leave(Hot),
}

/// What the pointer is over in the notifications panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hot {
    App(String),
    Note(u32),
}

/// What's open in the notifications panel.
#[derive(Debug, Clone, Default)]
pub struct NotesView {
    /// Apps whose groups show every notification, not just the newest.
    pub expanded: BTreeSet<String>,
    /// The notification whose snooze choices are showing.
    pub menu: Option<u32>,
    /// The app card and the notification under the pointer.
    pub hover_app: Option<String>,
    pub hover_note: Option<u32>,
}

impl NotesView {
    /// Note what the pointer moved onto or off. Leaving only clears what it
    /// left, so moving between two notifications never drops the new one.
    pub fn hover(&mut self, hot: Hot, on: bool) {
        match (hot, on) {
            (Hot::App(a), true) => self.hover_app = Some(a),
            (Hot::Note(id), true) => self.hover_note = Some(id),
            (Hot::App(a), false) if self.hover_app.as_ref() == Some(&a) => self.hover_app = None,
            (Hot::Note(id), false) if self.hover_note == Some(id) => {
                self.hover_note = None;
                if self.menu == Some(id) {
                    self.menu = None;
                }
            }
            _ => {}
        }
    }
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
                    .wrapping(Wrapping::None),
                text(np.artist.clone())
                    .font(INTER)
                    .size(12)
                    .color(r.muted)
                    .wrapping(Wrapping::None),
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

/// The notifications panel: a header with the bulk actions, then one card
/// per app, newest app first.
pub fn notes_panel(
    notices: Option<&Notices>,
    seen: u32,
    view: &NotesView,
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
        icons::symbol(
            if n.dnd { Symbol::BellOff } else { Symbol::Bell },
            16.0,
            r.muted,
        ),
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
            .size(11)
            .color(r.muted),
        ]
        .spacing(2)
        .width(Length::Fill),
        icon_button(Symbol::Clock, Action::ShowLast, n.history > 0, r),
        icon_button(Symbol::Trash, Action::Clear, !n.recent.is_empty(), r),
        icon_button(
            if n.dnd { Symbol::BellOff } else { Symbol::Bell },
            Action::Dnd,
            true,
            r
        ),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let mut list = column![].spacing(10);
    if n.recent.is_empty() {
        list = list.push(text("Nothing here yet").font(INTER).size(12).color(r.dim));
    }
    // Groups in the order of their newest notification.
    let mut groups: Vec<(&str, Vec<&Entry>)> = vec![];
    for item in n.recent.iter().take(LISTED) {
        match groups.iter_mut().find(|(app, _)| *app == item.app) {
            Some((_, items)) => items.push(item),
            None => groups.push((item.app.as_str(), vec![item])),
        }
    }
    for (app, items) in groups {
        let muted = n.muted.iter().any(|m| m == app);
        let open = view.expanded.contains(app);
        list = list.push(group(app, &items, muted, open, view, r));
    }

    // Right padding keeps the cards clear of the scrollbar.
    column![head, scrollable(list.padding([0, 8])).height(Length::Fill)]
        .spacing(10)
        .into()
}

/// One app's notifications as one rounded card, the way a phone stacks
/// them: the app's icon, name and age on top, then each notification.
/// Hovering the card shows the app's mute and clear buttons; hovering a
/// notification shows its own buttons, snooze and dismiss. A closed group
/// shows only the newest, with a sliver under it for the rest.
fn group(
    app: &str,
    items: &[&Entry],
    muted: bool,
    open: bool,
    view: &NotesView,
    r: Rail,
) -> Element<'static, Action> {
    let count = items.len();
    let hot = view.hover_app.as_deref() == Some(app);
    let mut header = row![
        avatar(app, r),
        text(app.to_owned())
            .font(INTER_SEMIBOLD)
            .size(11)
            .color(r.muted)
            .wrapping(Wrapping::None),
        text(format!("· {}", age(items[0].age)))
            .font(INTER)
            .size(11)
            .color(r.dim)
            .wrapping(Wrapping::None),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .height(22);
    if muted {
        header = header.push(icons::symbol(Symbol::BellOff, 11.0, r.dim));
    }
    header = header.push(Space::new().width(Length::Fill));
    if hot {
        header = header.push(icon_button(
            if muted { Symbol::Bell } else { Symbol::BellOff },
            Action::Mute(app.to_owned(), !muted),
            true,
            r,
        ));
        header = header.push(icon_button(
            Symbol::Trash,
            Action::ClearApp(app.to_owned()),
            true,
            r,
        ));
    }
    if count > 1 {
        header = header.push(expander(app, count, open, r));
    }

    let shown = if open { count } else { 1 };
    let mut body = column![header].spacing(8);
    for (i, item) in items.iter().take(shown).enumerate() {
        if i > 0 {
            body = body.push(rule(r));
        }
        body = body.push(card(item, view, r));
    }

    let face = container(body)
        .padding([10, 12])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(r.chip)),
            border: Border {
                radius: r.corner(16.0).into(),
                ..Border::default()
            },
            ..container::Style::default()
        });

    let mut stack = column![face];
    if count > 1 && !open {
        // The rest of the group peeking out from under the newest.
        stack = stack.push(
            container(
                container(Space::new().width(Length::Fill).height(6)).style(move |_| {
                    container::Style {
                        background: Some(Background::Color(r.chip_border)),
                        border: Border {
                            radius: iced::border::bottom(r.corner(10.0)),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    }
                }),
            )
            .padding([0, 14]),
        );
    }
    mouse_area(stack)
        .on_enter(Action::Hover(Hot::App(app.to_owned())))
        .on_exit(Action::Leave(Hot::App(app.to_owned())))
        .into()
}

/// One notification inside its app's card: the summary, the body (all of
/// it while hovered), and on hover the app's own buttons, snooze and dismiss.
fn card(e: &Entry, view: &NotesView, r: Rail) -> Element<'static, Action> {
    let hot = view.hover_note == Some(e.id) || view.menu == Some(e.id);
    let mut lines = column![
        text(if e.summary.is_empty() {
            e.app.clone()
        } else {
            e.summary.clone()
        })
        .font(INTER_SEMIBOLD)
        .size(13)
        .color(r.text)
        .wrapping(Wrapping::None),
    ]
    .spacing(3);
    if !e.body.is_empty() {
        let (body, lines_tall) = if hot {
            (e.body.clone(), 4.0)
        } else {
            (e.body.replace('\n', " "), 2.0)
        };
        lines = lines.push(
            container(
                text(body)
                    .font(INTER)
                    .size(12)
                    .line_height(1.3)
                    .color(r.muted)
                    .wrapping(Wrapping::WordOrGlyph),
            )
            .max_height(12.0 * 1.3 * lines_tall)
            .clip(true),
        );
    }
    if e.snoozed > 0 {
        lines = lines.push(
            row![
                icons::symbol(Symbol::Clock, 11.0, r.muted),
                text(format!("Snoozed · back in {}m", e.snoozed.div_ceil(60)))
                    .font(INTER)
                    .size(11)
                    .color(r.muted),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }

    if hot {
        let mut actions = row![].spacing(2).align_y(Alignment::Center);
        if view.menu == Some(e.id) {
            actions = actions.push(icons::symbol(Symbol::Clock, 12.0, r.muted));
            for (minutes, label) in SNOOZE {
                actions = actions.push(text_action(label, Action::Snooze(e.id, minutes), r));
            }
            actions = actions.push(Space::new().width(Length::Fill));
            actions = actions.push(icon_button(Symbol::X, Action::Menu(e.id), true, r));
        } else {
            for (key, label) in e.actions.iter().filter(|(k, _)| k != "default").take(3) {
                actions = actions.push(text_action(label, Action::Invoke(e.id, key.clone()), r));
            }
            actions = actions.push(Space::new().width(Length::Fill));
            actions = actions.push(icon_button(Symbol::Clock, Action::Menu(e.id), true, r));
            actions = actions.push(icon_button(Symbol::X, Action::Dismiss(e.id), true, r));
        }
        lines = lines.push(actions);
    }

    let id = e.id;
    let face = container(lines).width(Length::Fill).padding([0, 2]);
    let face: Element<'static, Action> = match e.actions.iter().find(|(k, _)| k == "default") {
        // Clicking the notification itself does what the app asked for.
        Some((key, _)) => mouse_area(face)
            .on_press(Action::Invoke(id, key.clone()))
            .interaction(iced::mouse::Interaction::Pointer)
            .into(),
        None => face.into(),
    };
    mouse_area(face)
        .on_enter(Action::Hover(Hot::Note(id)))
        .on_exit(Action::Leave(Hot::Note(id)))
        .into()
}

/// The app's icon, or its initial on its own colour when it has none.
fn avatar(app: &str, r: Rail) -> Element<'static, Action> {
    const SIZE: f32 = 18.0;
    match apps::icon_for(app) {
        Some(path) => ui::app_art(&path, SIZE),
        None => {
            let tint = ui::tint(app);
            container(
                text(app.chars().next().unwrap_or('?').to_uppercase().to_string())
                    .font(INTER_SEMIBOLD)
                    .size(10)
                    .color(r.on_accent),
            )
            .width(SIZE)
            .height(SIZE)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_| container::Style {
                background: Some(Background::Color(tint)),
                border: Border {
                    radius: (SIZE / 2.0).into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
        }
    }
}

/// The count of a group's notifications and the arrow that opens it.
fn expander(app: &str, count: usize, open: bool, r: Rail) -> Element<'static, Action> {
    button(
        row![
            text(count.to_string())
                .font(INTER_SEMIBOLD)
                .size(10)
                .color(r.text),
            icons::symbol(
                if open {
                    Symbol::ChevronUp
                } else {
                    Symbol::ChevronDown
                },
                12.0,
                r.text,
            ),
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    )
    .padding([2, 6])
    .on_press(Action::Expand(app.to_owned()))
    .style(move |_, status| button::Style {
        background: Some(Background::Color(if status == button::Status::Hovered {
            r.chip_border
        } else {
            r.panel
        })),
        text_color: r.text,
        border: Border {
            radius: 9.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    })
    .into()
}

/// A hairline between notifications in one card.
fn rule(r: Rail) -> Element<'static, Action> {
    container(Space::new().width(Length::Fill).height(1))
        .style(move |_| container::Style {
            background: Some(Background::Color(r.chip_border)),
            ..container::Style::default()
        })
        .into()
}

/// A small icon-only button. Disabled ones stay visible but don't press.
fn icon_button(symbol: Symbol, action: Action, enabled: bool, r: Rail) -> Element<'static, Action> {
    let tone = if enabled { r.muted } else { r.dim };
    let b = button(icons::symbol(symbol, 13.0, tone))
        .padding(4)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                r.chip_border
            } else {
                Color::TRANSPARENT
            })),
            text_color: tone,
            border: Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        });
    if enabled { b.on_press(action) } else { b }.into()
}

/// A flat text button, like a phone's notification actions.
fn text_action(label: &str, action: Action, r: Rail) -> Element<'static, Action> {
    button(
        text(label.to_owned())
            .font(INTER_SEMIBOLD)
            .size(11)
            .color(r.text),
    )
    .padding([4, 8])
    .on_press(action)
    .style(move |_, status| button::Style {
        background: Some(Background::Color(if status == button::Status::Hovered {
            r.chip_border
        } else {
            Color::TRANSPARENT
        })),
        text_color: r.text,
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    })
    .into()
}

/// How long ago, short: "now", "5m", "3h", "2d".
fn age(secs: u64) -> String {
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_short() {
        assert_eq!(age(5), "now");
        assert_eq!(age(90), "1m");
        assert_eq!(age(7300), "2h");
        assert_eq!(age(3 * 86_400), "3d");
    }

    #[test]
    fn leaving_one_note_keeps_the_next() {
        let mut v = NotesView::default();
        v.hover(Hot::Note(1), true);
        v.menu = Some(1);
        // The pointer enters the next note before leaving the first.
        v.hover(Hot::Note(2), true);
        v.hover(Hot::Note(1), false);
        assert_eq!(v.hover_note, Some(2));
        assert_eq!(
            v.menu,
            Some(1),
            "only leaving the note itself closes its menu"
        );
        v.hover(Hot::Note(2), false);
        assert_eq!(v.hover_note, None);
        v.hover(Hot::App("Mail".into()), true);
        v.hover(Hot::App("Music".into()), false);
        assert_eq!(v.hover_app.as_deref(), Some("Mail"));
    }

    #[test]
    fn unread_counts_what_is_newer_than_seen() {
        let mut n = Notices::default();
        n.recent = vec![
            Entry {
                id: 5,
                ..Entry::default()
            },
            Entry {
                id: 4,
                ..Entry::default()
            },
            Entry {
                id: 2,
                ..Entry::default()
            },
        ];
        assert_eq!(unread(&n, 3), 2);
    }
}
