//! The lock screen, from the canvas's Lock board: a large clock, your
//! avatar and a glass password card, with suspend, restart and shut down
//! in the corner. On Wayland it is an `ext-session-lock-v1` client, so the
//! compositor keeps everything hidden until PAM says the password is right,
//! even if this program crashes.
//!
//! Keys go straight to the password, wherever the pointer is: type, Enter
//! to unlock, Backspace, and Esc or Ctrl+U to clear.

pub mod pam;

use crate::bar::{INTER, INTER_SEMIBOLD, MONO, color};
use crate::icons::{self, icon};
use crate::sources::{self, Battery};
use crate::system;
use crate::ui;
use iced::alignment::{Horizontal, Vertical};
use iced::event::{self, Event};
use iced::font::Weight;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Space, button, column, container, row, stack, text};
use iced::{
    Alignment, Background, Border, Color, Element, Font, Length, Padding, Shadow, Subscription,
    Task, Vector,
};
use iced_sessionlock::to_session_message;
use nordlys_config::{Config, Palette};
use pam::Secret;
use std::time::{Duration, Instant};

const CARD: f32 = 420.0;
const SHAKE: Duration = Duration::from_millis(450);
/// The preview never asks PAM; this is its password.
pub const PREVIEW_PASSWORD: &str = "preview";

/// Typed text. Its Debug output hides what was typed.
#[derive(Clone)]
pub struct Typed(String);

impl std::fmt::Debug for Typed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Typed(<{} chars>)", self.0.chars().count())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Suspend,
    Restart,
    ShutDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Idle,
    Empty,
    Checking,
    Wrong,
    Unlocked,
}

#[to_session_message]
#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Frame,
    /// Text typed, and whether it says Caps Lock is on.
    Type(Typed, Option<bool>),
    Backspace,
    Clear,
    Submit,
    Checked(bool),
    ToggleShow,
    Power(Power),
}

pub struct Lock {
    palette: Palette,
    login: String,
    name: String,
    password: String,
    show: bool,
    status: Status,
    shake: Option<Instant>,
    caps: bool,
    since: String,
    clock: String,
    date: String,
    battery: Option<Battery>,
    hidden: u32,
    /// Do not disturb was off before locking, so switch it back on unlock.
    restore_dnd: bool,
    confirm: Option<(Power, Instant)>,
    ticks: u32,
    preview: bool,
    /// `reduce_motion`: a wrong password doesn't shake the card.
    still: bool,
}

/// Room for any password without the String growing, since growing would
/// leave copies of what was typed in freed memory.
const ROOM: usize = 256;

impl Lock {
    pub fn new(config: &Config, preview: bool) -> Self {
        let (login, name) = pam::current_user();
        // Hold notifications while locked, so nothing shows over the lock.
        let mut restore_dnd = false;
        let mut hidden = 0;
        if !preview && let Some(n) = system::notices() {
            if !n.dnd {
                system::set_dnd(true);
                restore_dnd = true;
            }
            hidden = n.waiting;
        }
        let mut me = Self {
            palette: config.palette(),
            login,
            name,
            password: String::with_capacity(ROOM),
            show: false,
            status: Status::Idle,
            shake: None,
            caps: false,
            since: chrono::Local::now().format("%H:%M").to_string(),
            clock: String::new(),
            date: String::new(),
            battery: None,
            hidden,
            restore_dnd,
            confirm: None,
            ticks: 0,
            preview,
            still: config.theme.reduce_motion,
        };
        me.tick();
        me
    }

    fn tick(&mut self) {
        let now = chrono::Local::now();
        self.clock = now.format("%H:%M").to_string();
        self.date = now.format("%A, %-d %B").to_string();
        self.battery = sources::battery();
        if !self.preview && self.ticks.is_multiple_of(5) {
            self.hidden = system::notices().map(|n| n.waiting).unwrap_or(0);
        }
        self.ticks = self.ticks.wrapping_add(1);
        if self
            .confirm
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(4))
        {
            self.confirm = None;
        }
    }

    fn wipe(&mut self) {
        // SAFETY: zero bytes are valid UTF-8.
        unsafe { self.password.as_mut_vec().fill(0) };
        self.password.clear();
    }

    fn fail(&mut self, status: Status) {
        self.status = status;
        if !self.still {
            self.shake = Some(Instant::now());
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.tick(),
            Message::Frame => {
                if self.shake.is_some_and(|t| t.elapsed() > SHAKE) {
                    self.shake = None;
                }
            }
            Message::Type(Typed(t), caps) => {
                if self.status != Status::Checking {
                    if self.password.len() + t.len() <= ROOM {
                        self.password.push_str(&t);
                    }
                    self.status = Status::Idle;
                }
                if let Some(c) = caps {
                    self.caps = c;
                }
            }
            Message::Backspace => {
                if self.status != Status::Checking {
                    self.password.pop();
                }
            }
            Message::Clear => {
                if self.status != Status::Checking {
                    self.wipe();
                    self.status = Status::Idle;
                }
            }
            Message::ToggleShow => self.show = !self.show,
            Message::Submit => {
                if self.status == Status::Checking {
                    return Task::none();
                }
                if self.password.is_empty() {
                    self.fail(Status::Empty);
                    return Task::none();
                }
                self.status = Status::Checking;
                let secret = Secret::new(std::mem::replace(
                    &mut self.password,
                    String::with_capacity(ROOM),
                ));
                return Task::perform(
                    check(self.login.clone(), secret, self.preview),
                    Message::Checked,
                );
            }
            Message::Checked(true) => {
                self.status = Status::Unlocked;
                if self.restore_dnd {
                    system::set_dnd(false);
                }
                return if self.preview {
                    iced::exit()
                } else {
                    Task::done(Message::UnLock)
                };
            }
            Message::Checked(false) => self.fail(Status::Wrong),
            Message::Power(p) => self.power(p),
            // UnLock only reaches here in the preview window.
            Message::UnLock => return iced::exit(),
        }
        Task::none()
    }

    /// Suspend right away; restart and shut down want a second click.
    fn power(&mut self, p: Power) {
        let confirmed = p == Power::Suspend || self.confirm.is_some_and(|(q, _)| q == p);
        if !confirmed {
            self.confirm = Some((p, Instant::now()));
            return;
        }
        self.confirm = None;
        if self.preview {
            eprintln!("nordlys-shell lock: preview, not running {p:?}");
            return;
        }
        let verb = match p {
            Power::Suspend => "suspend",
            Power::Restart => "reboot",
            Power::ShutDown => "poweroff",
        };
        let _ = std::process::Command::new("systemctl").arg(verb).spawn();
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let keys = event::listen_with(on_key);
        let clock = iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick);
        if self.shake.is_some() {
            Subscription::batch([
                keys,
                clock,
                iced::time::every(Duration::from_millis(16)).map(|_| Message::Frame),
            ])
        } else {
            Subscription::batch([keys, clock])
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let p = &self.palette;
        let look = Look::of(p);

        // Top right: battery.
        let mut top = row![Space::new().width(Length::Fill)]
            .align_y(Alignment::Center)
            .spacing(16);
        if let Some(b) = self.battery {
            top = top.push(
                row![
                    icon(icons::battery(b.percent, b.charging), 16).color(look.muted),
                    text(format!("{}%", b.percent))
                        .font(INTER)
                        .size(13)
                        .color(look.muted),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        let top = container(top).padding([24, 32]);

        // Clock
        let mut clock = column![
            text(self.clock.clone())
                .font(Font {
                    weight: Weight::ExtraLight,
                    ..INTER
                })
                .size(148)
                .color(look.text),
            text(self.date.clone())
                .font(INTER)
                .size(20)
                .color(look.muted),
        ]
        .spacing(4)
        .align_x(Alignment::Center);
        if self.hidden > 0 {
            let label = if self.hidden == 1 {
                "1 notification hidden until you unlock".to_owned()
            } else {
                format!("{} notifications hidden until you unlock", self.hidden)
            };
            clock = clock.push(Space::new().height(16)).push(
                container(
                    row![
                        icon(icons::BELL_OFF, 14).color(look.muted),
                        text(label).font(INTER).size(13).color(look.muted)
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([7, 14])
                .style(move |_| container::Style {
                    background: Some(Background::Color(look.chip)),
                    border: Border {
                        radius: 16.0.into(),
                        width: 1.0,
                        color: look.border,
                    },
                    ..container::Style::default()
                }),
            );
        }

        // Avatar
        let initial = self
            .name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        let ring = if self.status == Status::Checking {
            look.accent
        } else {
            Color {
                a: 0.55,
                ..look.accent
            }
        };
        let avatar = column![
            container(text(initial).font(INTER_SEMIBOLD).size(34).color(look.text))
                .width(96)
                .height(96)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| container::Style {
                    background: Some(Background::Color(Color {
                        a: 0.22,
                        ..look.accent
                    })),
                    border: Border {
                        radius: 48.0.into(),
                        width: 2.0,
                        color: ring,
                    },
                    ..container::Style::default()
                }),
            text(self.name.clone())
                .font(INTER_SEMIBOLD)
                .size(15)
                .color(look.text),
        ]
        .spacing(10)
        .align_x(Alignment::Center);

        let card = self.card(look);
        // Shake the card after a wrong password.
        let dx = self.shake.map_or(0.0, |t| {
            let f = t.elapsed().as_secs_f32() / SHAKE.as_secs_f32();
            (f * std::f32::consts::TAU * 3.0).sin() * 12.0 * (1.0 - f).max(0.0)
        });
        let card = container(card).padding(Padding {
            left: (dx * 2.0).max(0.0),
            right: (-dx * 2.0).max(0.0),
            ..Padding::ZERO
        });

        let middle = column![
            Space::new().height(Length::FillPortion(2)),
            clock,
            Space::new().height(Length::FillPortion(2)),
            avatar,
            Space::new().height(18),
            card,
            Space::new().height(Length::FillPortion(3)),
        ]
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fill);

        // Bottom: hints in the middle, power on the right.
        let hint = |k: &'static str, what: &'static str| {
            row![
                kbd(k, look),
                text(what).font(INTER).size(12).color(look.dim)
            ]
            .spacing(6)
            .align_y(Alignment::Center)
        };
        let pending = self.confirm.map(|(p, _)| p);
        let bottom = row![
            container(
                text(format!("Locked since {}", self.since))
                    .font(INTER)
                    .size(12)
                    .color(look.dim)
            )
            .width(Length::Fill),
            row![hint("Enter", "unlock"), hint("Esc", "clear")].spacing(16),
            container(
                row![
                    power(icons::MOON, Power::Suspend, pending, look),
                    power(icons::RESTART, Power::Restart, pending, look),
                    power(icons::POWER, Power::ShutDown, pending, look),
                ]
                .spacing(8)
            )
            .width(Length::Fill)
            .align_x(Horizontal::Right),
        ]
        .align_y(Alignment::Center)
        .padding([24, 32]);

        let content = column![top, middle, bottom]
            .width(Length::Fill)
            .height(Length::Fill);

        let bg = ui::wallpaper(p);
        let dim = look.dim_overlay;
        stack![
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(bg),
                    ..container::Style::default()
                }),
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(Background::Color(dim)),
                    ..container::Style::default()
                }),
            content,
        ]
        .into()
    }

    /// The session lock asks for a view per output; they all look the same.
    pub fn view_output(&self, _output: iced::window::Id) -> Element<'_, Message> {
        self.view()
    }

    fn card(&self, look: Look) -> Element<'_, Message> {
        let bad = matches!(self.status, Status::Empty | Status::Wrong);

        let mut head = row![
            text(format!("Unlock as {}", self.login))
                .font(INTER)
                .size(12)
                .color(look.muted),
            Space::new().width(Length::Fill),
        ]
        .height(22)
        .align_y(Alignment::Center);
        if self.caps {
            head = head.push(
                container(
                    row![
                        icon(icons::CAPS_LOCK, 12).color(look.warn),
                        text("Caps Lock is on")
                            .font(INTER_SEMIBOLD)
                            .size(12)
                            .color(look.warn)
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .padding([3, 8])
                .style(move |_| container::Style {
                    background: Some(Background::Color(Color {
                        a: 0.16,
                        ..look.warn
                    })),
                    border: Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
            );
        }

        // The field: what's typed is drawn as dots unless shown.
        let typed: Element<'_, Message> = if self.password.is_empty() {
            text(if self.status == Status::Checking {
                ""
            } else {
                "Password"
            })
            .font(INTER)
            .size(15)
            .color(look.dim)
            .into()
        } else if self.show {
            text(self.password.clone())
                .font(MONO)
                .size(15)
                .color(look.text)
                .into()
        } else {
            text("•".repeat(self.password.chars().count().min(32)))
                .font(INTER)
                .size(15)
                .color(look.text)
                .into()
        };
        let caret = container(Space::new().width(2).height(20)).style(move |_| container::Style {
            background: Some(Background::Color(look.accent)),
            ..container::Style::default()
        });
        let eye = button(
            container(icon(
                if self.show {
                    icons::EYE_OFF
                } else {
                    icons::EYE
                },
                16,
            ))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center),
        )
        .width(36)
        .height(36)
        .padding(0)
        .on_press(Message::ToggleShow)
        .style(move |_, status| button::Style {
            background: (status == button::Status::Hovered).then_some(Background::Color(look.chip)),
            text_color: look.muted,
            border: Border {
                radius: 9.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        });
        let go = button(
            container(icon(icons::ARROW_RIGHT, 18))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        )
        .width(38)
        .height(38)
        .padding(0)
        .on_press(Message::Submit)
        .style(move |_, _| button::Style {
            background: Some(Background::Color(look.accent)),
            text_color: look.on_accent,
            border: Border {
                radius: 10.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        });
        let field_border = if bad {
            look.err
        } else {
            Color {
                a: 0.7,
                ..look.accent
            }
        };
        let field = container(
            row![
                icon(icons::LOCK, 16).color(look.muted),
                row![typed, caret].spacing(2).align_y(Alignment::Center),
                Space::new().width(Length::Fill),
                eye,
                go,
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .height(48)
        .padding(Padding {
            left: 16.0,
            right: 5.0,
            top: 0.0,
            bottom: 0.0,
        })
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(look.field)),
            border: Border {
                radius: 12.0.into(),
                width: 1.5,
                color: field_border,
            },
            ..container::Style::default()
        });

        let (msg, msg_color) = match (self.confirm, self.status) {
            (Some((Power::Restart, _)), _) => ("Click restart again to restart", look.warn),
            (Some((Power::ShutDown, _)), _) => ("Click shut down again to turn off", look.warn),
            (_, Status::Idle) | (Some(_), _) => ("Type your password to unlock", look.muted),
            (_, Status::Empty) => ("Type your password first", look.err),
            (_, Status::Checking) => ("Checking…", look.muted),
            (_, Status::Wrong) => ("That password didn’t work. Try again.", look.err),
            (_, Status::Unlocked) => ("Welcome back", look.ok),
        };

        container(
            column![head, field, text(msg).font(INTER).size(13).color(msg_color),].spacing(10),
        )
        .width(CARD)
        .padding(Padding {
            top: 16.0,
            right: 18.0,
            bottom: 14.0,
            left: 18.0,
        })
        .style(move |_| container::Style {
            background: Some(Background::Color(look.card)),
            border: Border {
                radius: 20.0.into(),
                width: 1.0,
                color: look.border,
            },
            shadow: Shadow {
                color: look.shadow,
                offset: Vector::new(0.0, 30.0),
                blur_radius: 80.0,
            },
            ..container::Style::default()
        })
        .into()
    }

    pub fn style(&self, _theme: &iced::Theme) -> iced::theme::Style {
        // Opaque: nothing of the session may show through.
        iced::theme::Style {
            background_color: color(self.palette.ground),
            text_color: color(self.palette.text),
        }
    }
}

/// Check off the UI thread; PAM can take a few seconds on a wrong password.
async fn check(user: String, secret: Secret, preview: bool) -> bool {
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let ok = if preview {
            std::thread::sleep(Duration::from_millis(600));
            secret.is(PREVIEW_PASSWORD)
        } else {
            pam::authenticate(&user, secret).is_ok()
        };
        let _ = tx.send(ok);
    });
    rx.await.unwrap_or(false)
}

fn on_key(event: Event, _status: event::Status, _window: iced::window::Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        modifiers,
        text,
        ..
    }) = event
    else {
        return None;
    };
    match key.as_ref() {
        Key::Named(Named::Enter) => Some(Message::Submit),
        Key::Named(Named::Escape) => Some(Message::Clear),
        Key::Named(Named::Backspace) if modifiers.control() => Some(Message::Clear),
        Key::Named(Named::Backspace) => Some(Message::Backspace),
        Key::Character("u") if modifiers.control() => Some(Message::Clear),
        _ if modifiers.control() || modifiers.alt() || modifiers.logo() => None,
        _ => {
            let t: String = text?.chars().filter(|c| !c.is_control()).collect();
            if t.is_empty() {
                return None;
            }
            let caps = t
                .chars()
                .next()
                .and_then(|c| caps_lock(c, modifiers.shift()));
            Some(Message::Type(Typed(t), caps))
        }
    }
}

/// A letter in the "wrong" case for Shift means Caps Lock is on.
fn caps_lock(c: char, shift: bool) -> Option<bool> {
    (c.is_lowercase() || c.is_uppercase()).then(|| c.is_uppercase() != shift)
}

fn kbd<'a>(label: &'static str, look: Look) -> Element<'a, Message> {
    container(text(label).font(MONO).size(11).color(look.muted))
        .padding([3, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(look.chip)),
            border: Border {
                radius: 5.0.into(),
                width: 1.0,
                color: look.border,
            },
            ..container::Style::default()
        })
        .into()
}

fn power<'a>(glyph: char, p: Power, pending: Option<Power>, look: Look) -> Element<'a, Message> {
    let armed = pending == Some(p);
    let b = button(
        container(icon(glyph, 18))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center),
    )
    .width(40)
    .height(40)
    .padding(0)
    .on_press(Message::Power(p))
    .style(move |_, status| button::Style {
        background: Some(Background::Color(if armed {
            Color {
                a: 0.22,
                ..look.err
            }
        } else if status == button::Status::Hovered {
            Color {
                a: look.chip.a * 2.0,
                ..look.chip
            }
        } else {
            look.chip
        })),
        text_color: if armed { look.err } else { look.muted },
        border: Border {
            radius: 20.0.into(),
            width: 1.0,
            color: if armed { look.err } else { look.border },
        },
        ..button::Style::default()
    });
    b.into()
}

/// Colours from the Lock board on the canvas.
#[derive(Clone, Copy)]
struct Look {
    text: Color,
    muted: Color,
    dim: Color,
    accent: Color,
    on_accent: Color,
    card: Color,
    field: Color,
    chip: Color,
    border: Color,
    shadow: Color,
    dim_overlay: Color,
    err: Color,
    warn: Color,
    ok: Color,
}

impl Look {
    fn of(p: &Palette) -> Self {
        let base = Look {
            text: color(p.text),
            muted: color(p.muted),
            dim: color(p.dim),
            accent: color(p.accent),
            on_accent: color(p.on_accent),
            card: Color::from_rgba8(22, 26, 36, 0.62),
            field: Color::from_rgba8(0, 0, 0, 0.22),
            chip: Color::from_rgba8(255, 255, 255, 0.06),
            border: Color::from_rgba8(255, 255, 255, 0.12),
            shadow: Color::from_rgba8(0, 0, 0, 0.5),
            dim_overlay: Color::from_rgba8(5, 7, 11, 0.35),
            err: Color::from_rgb8(0xF4, 0x8B, 0x83),
            warn: Color::from_rgb8(0xF6, 0xC1, 0x77),
            ok: color(p.ansi[2]),
        };
        if p.dark {
            base
        } else {
            Look {
                card: Color::from_rgba8(255, 255, 255, 0.55),
                field: Color::from_rgba8(255, 255, 255, 0.7),
                chip: Color::from_rgba8(255, 255, 255, 0.45),
                border: Color::from_rgba8(255, 255, 255, 0.85),
                shadow: Color::from_rgba8(40, 50, 80, 0.18),
                dim_overlay: Color::from_rgba8(255, 255, 255, 0.10),
                err: Color::from_rgb8(0xB4, 0x23, 0x18),
                warn: Color::from_rgb8(0x9A, 0x5B, 0x00),
                ..base
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_lock_guess() {
        assert_eq!(caps_lock('a', false), Some(false));
        assert_eq!(caps_lock('A', true), Some(false));
        assert_eq!(caps_lock('A', false), Some(true));
        assert_eq!(caps_lock('a', true), Some(true));
        assert_eq!(caps_lock('7', false), None);
    }

    #[test]
    fn typed_text_stays_out_of_logs() {
        let m = Message::Type(Typed("hunter2".into()), None);
        assert!(!format!("{m:?}").contains("hunter2"));
    }
}
