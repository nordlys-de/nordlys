//! The notification daemon and its toasts. Notifications pop in as glass
//! cards, at the top right under the bar unless `[notifications]` says
//! otherwise.
//! Hovering one holds it; clicking it runs its default action or dismisses
//! it. Everything that leaves the screen goes to history, which the bar's
//! bell shows.

mod battery;
mod server;
pub mod store;

use crate::bar::{INTER, INTER_SEMIBOLD, color};
use crate::icons::{self, icon};
use crate::ui::{self, Rail};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::text::Wrapping;
use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Subscription, Task};
use iced_layershell::reexport::Anchor;
use iced_layershell::to_layer_message;
use nordlys_config::{Config, Notifications, Palette, ToastPosition};
use server::{Event, Shared};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use store::{Closed, Note, Request, Store, Urgency};
use zbus::Connection;

pub use server::{CONTROL, Entry, NAME, PATH};

/// The screen edges the toasts hang from. A centred stack names one edge, so
/// the compositor centres it along that edge.
pub fn anchor(position: ToastPosition) -> Anchor {
    match position {
        ToastPosition::TopRight => Anchor::Top | Anchor::Right,
        ToastPosition::TopCenter => Anchor::Top,
        ToastPosition::TopLeft => Anchor::Top | Anchor::Left,
        ToastPosition::BottomRight => Anchor::Bottom | Anchor::Right,
        ToastPosition::BottomCenter => Anchor::Bottom,
        ToastPosition::BottomLeft => Anchor::Bottom | Anchor::Left,
    }
}

const CARD: f32 = 360.0;
const GAP: f32 = 10.0;
/// Room around the cards for their shadow.
const MARGIN: f32 = 12.0;
/// Card heights are fixed so the layer surface can be sized exactly.
const BASE: f32 = 70.0;
/// One line of 13 px body text; bodies get one or two.
const LINE: f32 = 17.0;
/// About this many characters fit on a body line.
const LINE_CHARS: usize = 54;
const ACTIONS: f32 = 44.0;

pub struct Toasts {
    palette: Palette,
    /// Where the toasts show and for how long.
    cfg: Notifications,
    conn: Option<Connection>,
    store: Option<Shared>,
    shown: Vec<Note>,
    hovered: Option<u32>,
    preview: bool,
    /// Warn when the battery falls to this percentage; 0 is off.
    low_battery: u8,
    warning: battery::Warning,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Bus(Event),
    Tick,
    /// Time to look at the battery.
    Battery,
    /// nordlys.toml was saved.
    Config(nordlys_config::Config),
    Hover(u32),
    Unhover(u32),
    Dismiss(u32),
    Invoke(u32, String),
}

/// Lines of body shown: none, one or two (the rest is clipped).
fn body_lines(n: &Note) -> f32 {
    match n.body.chars().count() {
        0 => 0.0,
        c if c <= LINE_CHARS && !n.body.contains('\n') => 1.0,
        _ => 2.0,
    }
}

/// The body and the gap above it.
fn body_height(n: &Note) -> f32 {
    if n.body.is_empty() {
        0.0
    } else {
        6.0 + LINE * body_lines(n)
    }
}

fn card_height(n: &Note) -> f32 {
    BASE + body_height(n)
        + if buttons(n).next().is_some() {
            ACTIONS
        } else {
            0.0
        }
}

/// Actions that get a button; "default" is the click on the card itself.
fn buttons(n: &Note) -> impl Iterator<Item = &(String, String)> {
    n.actions.iter().filter(|(k, _)| k != "default")
}

impl Toasts {
    pub fn new(config: &Config, preview: bool) -> Self {
        Self {
            palette: config.palette(),
            cfg: config.notifications.clone(),
            conn: None,
            store: None,
            shown: vec![],
            hovered: None,
            preview,
            low_battery: config.power.low_battery,
            warning: battery::Warning::default(),
        }
    }

    /// Copy what's on screen out of the store and fit the surface to it.
    fn sync(&mut self) -> Task<Message> {
        if let Some(store) = &self.store {
            self.shown = server::lock(store).shown.clone();
        }
        if self.preview {
            return Task::none();
        }
        let size = if self.shown.is_empty() {
            (1, 1)
        } else {
            let h: f32 = self.shown.iter().map(card_height).sum::<f32>()
                + GAP * (self.shown.len() - 1) as f32
                + MARGIN * 2.0;
            ((CARD + MARGIN * 2.0) as u32, h.ceil() as u32)
        };
        Task::done(Message::SizeChange(size))
    }

    /// Hand the store the time a toast stays when its app doesn't say.
    fn apply_timeout(&self) {
        if let Some(store) = &self.store {
            server::lock(store).default_timeout =
                Some(Duration::from_secs(u64::from(self.cfg.seconds)));
        }
    }

    fn signal_closed(&self, ids: Vec<u32>, reason: Closed) -> Task<Message> {
        let Some(conn) = self.conn.clone() else {
            return Task::none();
        };
        Task::batch(
            ids.into_iter()
                .map(|id| Task::future(server::closed(conn.clone(), id, reason)).discard()),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Bus(Event::Ready { conn, store }) => {
                self.conn = Some(conn);
                self.store = Some(store);
                self.apply_timeout();
                self.sync()
            }
            Message::Bus(Event::Changed) => self.sync(),
            Message::Bus(Event::Invoke(id, key)) => self.update(Message::Invoke(id, key)),
            Message::Bus(Event::Gone(ids)) => {
                let signals = self.signal_closed(ids, Closed::Dismissed);
                Task::batch([signals, self.sync()])
            }
            Message::Config(c) => {
                self.palette = c.palette();
                self.low_battery = c.power.low_battery;
                let moved = self.cfg.position != c.notifications.position;
                self.cfg = c.notifications;
                self.apply_timeout();
                if moved && !self.preview {
                    return Task::done(Message::AnchorChange(anchor(self.cfg.position)));
                }
                Task::none()
            }
            Message::Bus(Event::Failed(e)) => {
                eprintln!("nordlys-shell notify: {e}");
                if self.preview {
                    eprintln!("nordlys-shell notify: showing sample notifications instead");
                    self.store = Some(Arc::new(Mutex::new(demo())));
                    self.sync()
                } else {
                    iced::exit()
                }
            }
            Message::Tick => {
                let Some(store) = &self.store else {
                    return Task::none();
                };
                let now = Instant::now();
                let (gone, woke) = {
                    let mut s = server::lock(store);
                    (s.expire(now, self.hovered), s.wake(now))
                };
                if gone.is_empty() && !woke {
                    return Task::none();
                }
                let signals = self.signal_closed(gone, Closed::Expired);
                Task::batch([signals, self.sync()])
            }
            Message::Battery => self.check_battery(),
            Message::Hover(id) => {
                self.hovered = Some(id);
                Task::none()
            }
            Message::Unhover(id) => {
                if self.hovered == Some(id) {
                    self.hovered = None;
                }
                if let Some(store) = &self.store {
                    server::lock(store).extend(id, Instant::now(), Duration::from_secs(2));
                }
                Task::none()
            }
            Message::Dismiss(id) => self.dismiss(id),
            Message::Invoke(id, key) => {
                let signal = match self.conn.clone() {
                    Some(conn) => Task::future(server::invoked(conn, id, key)).discard(),
                    None => Task::none(),
                };
                Task::batch([signal, self.dismiss(id)])
            }
            _ => Task::none(),
        }
    }

    /// Put the low-battery warning up, or take it down once on charge.
    fn check_battery(&mut self) -> Task<Message> {
        let (Some(store), Some(now)) = (&self.store, crate::sources::battery()) else {
            return Task::none();
        };
        match self.warning.check(&now, self.low_battery) {
            battery::Step::Warn => {
                let request = battery::request(now.percent, self.warning.id);
                self.warning.id = server::lock(store).notify(request, Instant::now());
                self.sync()
            }
            battery::Step::Clear(id) => {
                if !server::lock(store).close(id, Closed::Called) {
                    return Task::none();
                }
                let signal = self.signal_closed(vec![id], Closed::Called);
                Task::batch([signal, self.sync()])
            }
            battery::Step::Nothing => Task::none(),
        }
    }

    fn dismiss(&mut self, id: u32) -> Task<Message> {
        let Some(store) = &self.store else {
            return Task::none();
        };
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        let was_shown = server::lock(store).close(id, Closed::Dismissed);
        let signal = if was_shown {
            self.signal_closed(vec![id], Closed::Dismissed)
        } else {
            Task::none()
        };
        Task::batch([signal, self.sync()])
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut all = vec![
            Subscription::run(server::serve).map(Message::Bus),
            crate::events::config_changes().map(Message::Config),
        ];
        // Toasts need a quick tick to time out; snoozed notes only need a slow
        // one to wake, so that's all there is when nothing is on screen.
        let tick = if self.shown.is_empty() { 1000 } else { 250 };
        all.push(iced::time::every(Duration::from_millis(tick)).map(|_| Message::Tick));
        // Once a minute is soon enough for a battery, and costs next to nothing.
        if self.low_battery > 0 && self.store.is_some() && !self.preview {
            all.push(iced::time::every(Duration::from_secs(60)).map(|_| Message::Battery));
        }
        Subscription::batch(all)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let r = Rail::of(&self.palette);
        let critical = crate::bar::color(self.palette.ansi[1]);
        let now = Instant::now();

        // The newest is nearest the screen's edge: on top, or at the bottom
        // when the toasts hang from the bottom.
        let bottom = self.cfg.position.bottom();
        let mut notes: Vec<&Note> = self.shown.iter().collect();
        if !bottom {
            notes.reverse();
        }
        let cards = column(notes.into_iter().map(|n| toast(n, now, critical, r))).spacing(GAP);
        let side = match self.cfg.position {
            ToastPosition::TopLeft | ToastPosition::BottomLeft => Horizontal::Left,
            ToastPosition::TopCenter | ToastPosition::BottomCenter => Horizontal::Center,
            ToastPosition::TopRight | ToastPosition::BottomRight => Horizontal::Right,
        };
        let page = container(cards)
            .padding(MARGIN)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(side)
            .align_y(if bottom {
                Vertical::Bottom
            } else {
                Vertical::Top
            });

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

fn ago(received: Instant, now: Instant) -> String {
    let s = now.saturating_duration_since(received).as_secs();
    match s {
        0..60 => "now".into(),
        60..3600 => format!("{} min", s / 60),
        _ => format!("{} h", s / 3600),
    }
}

fn toast<'a>(n: &Note, now: Instant, critical: Color, r: Rail) -> Element<'a, Message> {
    let id = n.id;
    let urgent = n.urgency == Urgency::Critical;
    let dot = if urgent { critical } else { ui::tint(&n.app) };

    let close = button(icon(icons::CLOSE, 12))
        .padding(4)
        .on_press(Message::Dismiss(id))
        .style(move |_, status| button::Style {
            background: (status == button::Status::Hovered).then_some(Background::Color(r.chip)),
            text_color: r.muted,
            border: Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        });
    let header = row![
        container(Space::new().width(8).height(8)).style(move |_| container::Style {
            background: Some(Background::Color(dot)),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        }),
        text(n.app.clone())
            .font(INTER_SEMIBOLD)
            .size(12)
            .color(r.muted)
            .wrapping(Wrapping::None),
        Space::new().width(Length::Fill),
        text(ago(n.received, now)).font(INTER).size(11).color(r.dim),
        close,
    ]
    .spacing(8)
    .height(20)
    .align_y(Alignment::Center);

    let mut body = column![
        header,
        text(n.summary.clone())
            .font(INTER_SEMIBOLD)
            .size(14)
            .color(r.text)
            .wrapping(Wrapping::None),
    ]
    .spacing(6);
    if !n.body.is_empty() {
        body = body.push(
            container(text(n.body.clone()).font(INTER).size(13).color(r.muted))
                .height(LINE * body_lines(n) + 1.0)
                .clip(true),
        );
    }
    let actions: Vec<Element<'a, Message>> = buttons(n)
        .map(|(key, label)| {
            button(text(label.clone()).font(INTER).size(12))
                .padding([6, 12])
                .on_press(Message::Invoke(id, key.clone()))
                .style(move |_, status| button::Style {
                    background: Some(Background::Color(if status == button::Status::Hovered {
                        Color {
                            a: r.chip.a * 1.8,
                            ..r.chip
                        }
                    } else {
                        r.chip
                    })),
                    text_color: r.text,
                    border: Border {
                        radius: 8.0.into(),
                        width: 1.0,
                        color: r.chip_border,
                    },
                    ..button::Style::default()
                })
                .into()
        })
        .collect();
    if !actions.is_empty() {
        body = body
            .push(Space::new().height(4))
            .push(row(actions).spacing(8));
    }

    let panel = r.panel_style();
    let card = container(body)
        .width(CARD)
        .height(card_height(n))
        .padding([12, 14])
        .clip(true)
        .align_y(Vertical::Top)
        .style(move |theme| {
            let mut s = panel(theme);
            if urgent {
                s.border.color = critical;
            }
            s
        });
    let default = if n.actions.iter().any(|(k, _)| k == "default") {
        Message::Invoke(id, "default".into())
    } else {
        Message::Dismiss(id)
    };
    mouse_area(card)
        .on_enter(Message::Hover(id))
        .on_exit(Message::Unhover(id))
        .on_press(default)
        .into()
}

/// Sample notifications for the preview when the bus name isn't free.
fn demo() -> Store {
    let mut s = Store::default();
    let t = Instant::now();
    let note = |app: &str, summary: &str, body: &str| Request {
        app: app.into(),
        summary: summary.into(),
        body: body.into(),
        timeout: 0,
        ..Request::default()
    };
    s.notify(
        note(
            "cargo",
            "Tests passed",
            "nordlys-comp · stage module, 10 tests",
        ),
        t,
    );
    s.notify(
        Request {
            actions: vec![
                ("default".into(), "Open".into()),
                ("open".into(), "Open folder".into()),
                ("later".into(), "Later".into()),
            ],
            ..note(
                "Firefox",
                "Download complete",
                "nordlys-wallpapers.zip finished downloading (24 MB)",
            )
        },
        t,
    );
    s.notify(
        Request {
            urgency: Some(Urgency::Critical),
            ..note("Power", "Battery low", "8% left. Plug in soon.")
        },
        t,
    );
    s
}
