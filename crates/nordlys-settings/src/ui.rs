//! The pieces every page is built from: sections of rows on cards, each
//! row a title, a line of explanation and a control on the right.

use crate::icons::{Icon, icon};
use crate::theme::{BOLD, C, Kind, MONO, SEMIBOLD, alpha};
use iced::widget::{
    Space, button, column, container, row, rule, slider, text, text_input, toggler,
};
use iced::{Alignment, Color, Element, Length, Padding};

pub type El<'a, M> = Element<'a, M>;

/// The page title and what the page is for.
pub fn header<'a, M: 'a>(c: C, title: &str, detail: &str) -> El<'a, M> {
    column![
        text(title.to_owned()).font(BOLD).size(26).color(c.text),
        text(detail.to_owned()).size(14).color(c.muted),
    ]
    .spacing(6)
    .into()
}

/// A labelled card of rows, with hairlines between them.
pub fn section<'a, M: 'a>(c: C, title: &str, rows: Vec<El<'a, M>>) -> El<'a, M> {
    let mut list = column![].spacing(0);
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        list = list.push(r);
        if i + 1 < n {
            list = list.push(
                container(rule::horizontal(1).style(move |_| rule::Style {
                    color: alpha(c.border, 0.7),
                    radius: 0.0.into(),
                    fill_mode: rule::FillMode::Full,
                    snap: true,
                }))
                .padding(Padding::ZERO.left(18).right(18)),
            );
        }
    }
    let card = container(list).style(c.card()).width(Length::Fill);
    if title.is_empty() {
        return card.into();
    }
    column![label(c, title), card].spacing(10).into()
}

pub fn label<'a, M: 'a>(c: C, s: &str) -> El<'a, M> {
    container(text(s.to_uppercase()).font(SEMIBOLD).size(11).color(c.dim))
        .padding(Padding::ZERO.left(4))
        .into()
}

/// One row: title and detail on the left, `control` on the right.
pub fn setting<'a, M: 'a>(
    c: C,
    title: impl Into<String>,
    detail: impl Into<String>,
    control: impl Into<El<'a, M>>,
) -> El<'a, M> {
    let title = title.into();
    let detail = detail.into();
    let mut words = column![text(title).size(14).color(c.text)].spacing(3);
    if !detail.is_empty() {
        words = words.push(text(detail).size(12).color(c.muted));
    }
    row![container(words).width(Length::Fill), control.into()]
        .spacing(18)
        .align_y(Alignment::Center)
        .padding([14, 18])
        .into()
}

/// A row with an icon in front.
pub fn icon_setting<'a, M: 'a>(
    c: C,
    ic: Icon,
    tint: Color,
    title: impl Into<String>,
    detail: impl Into<String>,
    control: impl Into<El<'a, M>>,
) -> El<'a, M> {
    let title = title.into();
    let detail = detail.into();
    let mut words = column![text(title).size(14).color(c.text)].spacing(3);
    if !detail.is_empty() {
        words = words.push(text(detail).size(12).color(c.muted));
    }
    row![
        badge_icon(c, ic, tint),
        container(words).width(Length::Fill),
        control.into()
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .padding([12, 18])
    .into()
}

pub fn badge_icon<'a, M: 'a>(_c: C, ic: Icon, tint: Color) -> El<'a, M> {
    container(icon(ic, 18.0, tint))
        .center(34)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(alpha(tint, 0.14))),
            border: iced::Border {
                radius: 10.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

pub fn toggle<'a, M: 'a>(on: bool, f: impl Fn(bool) -> M + 'a) -> El<'a, M> {
    toggler(on).on_toggle(f).size(22).into()
}

pub fn toggle_maybe<'a, M: 'a>(on: bool, f: Option<impl Fn(bool) -> M + 'a>) -> El<'a, M> {
    toggler(on).on_toggle_maybe(f).size(22).into()
}

pub fn toggle_row<'a, M: 'a>(
    c: C,
    title: impl Into<String>,
    detail: impl Into<String>,
    on: bool,
    f: impl Fn(bool) -> M + 'a,
) -> El<'a, M> {
    setting(c, title, detail, toggle(on, f))
}

pub fn btn<'a, M: Clone + 'a>(
    c: C,
    label: impl Into<String>,
    kind: Kind,
    msg: Option<M>,
) -> El<'a, M> {
    button(text(label.into()).size(13).font(SEMIBOLD))
        .padding([8, 14])
        .style(c.button(kind))
        .on_press_maybe(msg)
        .into()
}

pub fn icon_btn<'a, M: Clone + 'a>(c: C, ic: Icon, kind: Kind, msg: Option<M>) -> El<'a, M> {
    let fg = match kind {
        Kind::Primary => c.on_accent,
        Kind::Danger => c.bad,
        _ => c.muted,
    };
    button(icon(ic, 16.0, fg))
        .padding(8)
        .style(c.button(kind))
        .on_press_maybe(msg)
        .into()
}

pub fn icon_text_btn<'a, M: Clone + 'a>(
    c: C,
    ic: Icon,
    label: impl Into<String>,
    kind: Kind,
    msg: Option<M>,
) -> El<'a, M> {
    let fg = match kind {
        Kind::Primary => c.on_accent,
        Kind::Danger => c.bad,
        Kind::Ghost => c.muted,
        _ => c.text,
    };
    button(
        row![
            icon(ic, 15.0, fg),
            text(label.into()).size(13).font(SEMIBOLD)
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([8, 14])
    .style(c.button(kind))
    .on_press_maybe(msg)
    .into()
}

/// A row of mutually exclusive choices.
pub fn segmented<'a, T, M>(
    c: C,
    options: &[(T, &str)],
    selected: T,
    f: impl Fn(T) -> M + 'a,
) -> El<'a, M>
where
    T: Copy + PartialEq + 'a,
    M: Clone + 'a,
{
    let mut r = row![].spacing(2);
    for (value, name) in options {
        let on = *value == selected;
        r = r.push(
            button(text((*name).to_owned()).size(12).font(SEMIBOLD))
                .padding([6, 12])
                .style(c.button(Kind::Segment(on)))
                .on_press(f(*value)),
        );
    }
    container(r).padding(3).style(c.segments()).into()
}

/// A slider with its value printed beside it.
#[allow(clippy::too_many_arguments)]
pub fn slider_row<'a, M: Clone + 'a>(
    c: C,
    title: impl Into<String>,
    detail: impl Into<String>,
    range: std::ops::RangeInclusive<f32>,
    step: f32,
    value: f32,
    shown: String,
    on_change: impl Fn(f32) -> M + 'a,
    on_release: Option<M>,
) -> El<'a, M> {
    let mut s = slider(range, value, on_change).step(step).width(220);
    if let Some(m) = on_release {
        s = s.on_release(m);
    }
    setting(
        c,
        title,
        detail,
        row![
            s,
            container(text(shown).size(13).font(MONO).color(c.muted))
                .width(58)
                .align_x(Alignment::End)
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
}

pub fn input<'a, M: Clone + 'a>(
    c: C,
    placeholder: &str,
    value: &str,
) -> text_input::TextInput<'a, M> {
    text_input(placeholder, value)
        .padding([8, 12])
        .size(14)
        .style(c.input())
}

pub fn chip<'a, M: 'a>(c: C, label: impl Into<String>, tint: Color) -> El<'a, M> {
    container(text(label.into()).size(11).font(SEMIBOLD))
        .padding([3, 10])
        .style(c.chip(tint))
        .into()
}

/// A tinted message across the page.
pub fn note<'a, M: 'a>(c: C, ic: Icon, tint: Color, message: impl Into<String>) -> El<'a, M> {
    container(
        row![
            icon(ic, 18.0, tint),
            text(message.into())
                .size(13)
                .color(c.text)
                .width(Length::Fill)
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([12, 16])
    .width(Length::Fill)
    .style(c.note(tint))
    .into()
}

pub fn note_with<'a, M: 'a>(
    c: C,
    ic: Icon,
    tint: Color,
    message: impl Into<String>,
    action: El<'a, M>,
) -> El<'a, M> {
    container(
        row![
            icon(ic, 18.0, tint),
            text(message.into())
                .size(13)
                .color(c.text)
                .width(Length::Fill),
            action
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([10, 16])
    .width(Length::Fill)
    .style(c.note(tint))
    .into()
}

/// A plain fact: name on the left, value on the right.
pub fn fact<'a, M: 'a>(c: C, name: impl Into<String>, value: impl Into<String>) -> El<'a, M> {
    row![
        text(name.into())
            .size(14)
            .color(c.muted)
            .width(Length::Fill),
        text(value.into()).size(14).color(c.text)
    ]
    .spacing(18)
    .padding([12, 18])
    .align_y(Alignment::Center)
    .into()
}

pub fn mono_fact<'a, M: 'a>(c: C, name: impl Into<String>, value: impl Into<String>) -> El<'a, M> {
    row![
        text(name.into())
            .size(14)
            .color(c.muted)
            .width(Length::Fill),
        text(value.into()).size(13).font(MONO).color(c.text)
    ]
    .spacing(18)
    .padding([12, 18])
    .align_y(Alignment::Center)
    .into()
}

/// Shown in a card that has nothing to list yet.
pub fn empty<'a, M: 'a>(c: C, message: impl Into<String>) -> El<'a, M> {
    container(text(message.into()).size(13).color(c.dim))
        .padding([18, 18])
        .width(Length::Fill)
        .into()
}

pub fn loading<'a, M: 'a>(c: C) -> El<'a, M> {
    section(c, "", vec![empty(c, "Reading…")])
}

pub fn gap<'a, M: 'a>(h: f32) -> El<'a, M> {
    Space::new().height(h).into()
}

pub fn fill<'a, M: 'a>() -> El<'a, M> {
    Space::new().width(Length::Fill).into()
}

/// A simple meter, 0.0–1.0, for memory, disk and battery.
pub fn meter<'a, M: 'a>(c: C, value: f32, tint: Color) -> El<'a, M> {
    let v = value.clamp(0.0, 1.0);
    let bar = container(Space::new().width(Length::Fill).height(6))
        .width(Length::FillPortion(((v * 1000.0) as u16).max(1)))
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(tint)),
            border: iced::Border {
                radius: 3.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        });
    let rest = Space::new()
        .width(Length::FillPortion((((1.0 - v) * 1000.0) as u16).max(1)))
        .height(6);
    container(row![bar, rest])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(alpha(c.text, 0.08))),
            border: iced::Border {
                radius: 3.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// "4 min", "1 h 30 min", "Never".
pub fn minutes(secs: Option<u32>) -> String {
    match secs {
        None => "Never".into(),
        Some(s) if s < 60 => format!("{s} s"),
        Some(s) if s < 3600 => format!("{} min", s / 60),
        Some(s) if s % 3600 == 0 => format!("{} h", s / 3600),
        Some(s) => format!("{} h {} min", s / 3600, s / 60 % 60),
    }
}
