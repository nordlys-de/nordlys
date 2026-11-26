//! The ambient rail on the right edge: things that matter on every
//! workspace. Now playing, quick settings and notifications live here; any
//! tile opens a pull-out panel with the controls.

use crate::bar::{INTER, INTER_SEMIBOLD, color};
use crate::icons::{self, icon};
use crate::system::{self, Notices, NowPlaying, Snapshot, Volume};
use crate::ui::{self, Rail};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{Space, button, column, container, mouse_area, row, slider, stack, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Subscription, Task};
use iced_layershell::to_layer_message;
use nordlys_config::{Config, Palette};
use std::time::Duration;

use crate::shelf::{RAIL, ZONE};
const TILE: f32 = 56.0;
const PANEL: f32 = 368.0;

pub struct Ambient {
    palette: Palette,
    playing: Option<NowPlaying>,
    notices: Option<Notices>,
    /// Newest notification id the user has seen in the panel.
    seen: u32,
    volume: Option<Volume>,
    wifi: Option<bool>,
    bluetooth: Option<bool>,
    /// Volume while the slider is held, applied on release.
    dragging: Option<f32>,
    open: bool,
    preview: bool,
    reads: system::Reads,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Read(Option<Snapshot>),
    /// A control finished; read the new state.
    Done,
    /// nordlys.toml was saved.
    Config(Config),
    Toggle,
    PlayPause,
    Next,
    Volume(f32),
    VolumeDone,
    Mute,
    Wifi,
    Bluetooth,
    Dnd,
    ShowLast,
    Clear,
}

impl Ambient {
    pub fn new(config: &Config, preview: bool, open: bool) -> (Self, Task<Message>) {
        let mut me = Self {
            palette: config.palette(),
            playing: None,
            notices: None,
            seen: 0,
            volume: None,
            wifi: None,
            bluetooth: None,
            dragging: None,
            open,
            preview,
            reads: system::Reads::default(),
        };
        let first = me.refresh();
        (me, first)
    }

    /// Read the tools in the background. The rail only needs media and
    /// notices; the quick settings are read while the panel is open.
    fn refresh(&mut self) -> Task<Message> {
        if !self.reads.start() {
            return Task::none();
        }
        let controls = self.open;
        Task::perform(
            system::background(move || Snapshot::read(controls)),
            Message::Read,
        )
    }

    fn apply(&mut self, snap: Snapshot) {
        self.playing = snap.playing;
        self.notices = snap.notices;
        if self.open
            && let Some(newest) = self.notices.as_ref().and_then(|n| n.recent.first())
        {
            self.seen = self.seen.max(newest.id);
        }
        if let Some(c) = snap.controls {
            self.volume = c.volume;
            self.wifi = c.wifi;
            self.bluetooth = c.bluetooth;
        }
    }

    /// Run a control in the background, then read the result.
    fn act(&mut self, work: impl FnOnce() + Send + 'static) -> Task<Message> {
        Task::perform(system::background(work), |_| Message::Done)
    }

    fn resize(&self) -> Task<Message> {
        if self.preview {
            return Task::none();
        }
        let width = if self.open {
            ZONE + PANEL as u32 + 12
        } else {
            ZONE
        };
        Task::done(Message::SizeChange((width, 0)))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick | Message::Done => self.refresh(),
            Message::Config(c) => {
                self.palette = c.palette();
                Task::none()
            }
            Message::Read(snap) => {
                let again = self.reads.done();
                if let Some(snap) = snap {
                    self.apply(snap);
                }
                if again {
                    return self.refresh();
                }
                Task::none()
            }
            Message::Toggle => {
                self.open = !self.open;
                // Opening wants the quick settings now, not after a read
                // that started while the panel was shut.
                if self.open {
                    self.reads.invalidate();
                }
                Task::batch([self.refresh(), self.resize()])
            }
            Message::PlayPause => {
                if let Some(p) = &mut self.playing {
                    p.playing = !p.playing;
                }
                self.act(system::play_pause)
            }
            Message::Next => self.act(system::next_track),
            Message::Volume(v) => {
                self.dragging = Some(v);
                Task::none()
            }
            Message::VolumeDone => match self.dragging.take() {
                Some(v) => {
                    if let Some(vol) = &mut self.volume {
                        vol.level = v;
                    }
                    self.act(move || system::set_volume(v))
                }
                None => Task::none(),
            },
            Message::Mute => {
                if let Some(vol) = &mut self.volume {
                    vol.muted = !vol.muted;
                }
                self.act(system::toggle_mute)
            }
            Message::Wifi => match self.wifi {
                Some(on) => {
                    self.wifi = Some(!on);
                    self.act(move || system::set_wifi(!on))
                }
                None => Task::none(),
            },
            Message::Bluetooth => match self.bluetooth {
                Some(on) => {
                    self.bluetooth = Some(!on);
                    self.act(move || system::set_bluetooth(!on))
                }
                None => Task::none(),
            },
            Message::Dnd => match &mut self.notices {
                Some(n) => {
                    n.dnd = !n.dnd;
                    let on = n.dnd;
                    self.act(move || system::set_dnd(on))
                }
                None => Task::none(),
            },
            Message::ShowLast => self.act(system::pop_notification),
            Message::Clear => self.act(system::clear_notifications),
            _ => Task::none(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let every = if self.open { 1000 } else { 2500 };
        Subscription::batch([
            iced::time::every(Duration::from_millis(every)).map(|_| Message::Tick),
            crate::events::config_changes().map(Message::Config),
        ])
    }

    pub fn view(&self) -> Element<'_, Message> {
        let r = Rail::of(&self.palette);
        let amber = Color::from_rgb8(0xF6, 0xC1, 0x77);

        let media_tile = tile(
            icons::MUSIC,
            Color { a: 0.20, ..amber },
            false,
            None,
            self.playing.as_ref().is_some_and(|p| p.playing),
            r,
        );
        let settings_tile = tile(icons::SLIDERS, r.chip, self.open, None, false, r);
        // With the Nordlys daemon the badge counts what arrived since the
        // panel was last open; with dunst, what it holds.
        let unread = match &self.notices {
            Some(n) if !n.recent.is_empty() => {
                n.recent.iter().filter(|r| r.id > self.seen).count() as u32
            }
            Some(n) => n.waiting + n.history,
            None => 0,
        };
        let dnd = self.notices.as_ref().is_some_and(|n| n.dnd);
        let notes_tile = tile(
            if dnd { icons::BELL_OFF } else { icons::BELL },
            r.chip,
            false,
            (unread > 0).then_some(unread),
            false,
            r,
        );

        let rail = container(
            column![
                ui::section_label("Ambient", r),
                media_tile,
                settings_tile,
                notes_tile
            ]
            .spacing(10)
            .align_x(Alignment::Center),
        )
        .width(RAIL)
        .height(Length::Fill)
        .padding([12, 0])
        .align_x(Horizontal::Center)
        .style(r.rail_style());

        let mut layout = row![].spacing(12);
        if self.open {
            layout = layout.push(self.panel(r));
        }
        layout = layout.push(rail);
        let page = container(layout)
            .padding(iced::Padding {
                left: 0.0,
                top: 12.0,
                bottom: 12.0,
                right: 12.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Right);

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

    fn panel(&self, r: Rail) -> Element<'_, Message> {
        let amber = Color::from_rgb8(0xF6, 0xC1, 0x77);

        // Now playing
        let media: Element<'_, Message> = match &self.playing {
            Some(np) => row![
                container(icon(icons::MUSIC, 22).color(amber))
                    .width(56)
                    .height(56)
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
                        .color(r.text),
                    text(np.artist.clone()).font(INTER).size(12).color(r.muted),
                ]
                .spacing(4)
                .width(Length::Fill),
                round(
                    if np.playing {
                        icons::PAUSE
                    } else {
                        icons::PLAY
                    },
                    Message::PlayPause,
                    true,
                    r
                ),
                round(icons::SKIP_FORWARD, Message::Next, false, r),
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
        };

        // Toggles
        let toggles = row![
            toggle(
                (icons::WIFI, icons::WIFI_OFF),
                "Wi-Fi",
                self.wifi,
                Message::Wifi,
                r
            ),
            toggle(
                (icons::BLUETOOTH, icons::BLUETOOTH_OFF),
                "Bluetooth",
                self.bluetooth,
                Message::Bluetooth,
                r
            ),
            toggle(
                (icons::MOON, icons::MOON),
                "Do not disturb",
                self.notices.as_ref().map(|n| n.dnd),
                Message::Dnd,
                r
            ),
        ]
        .spacing(8);

        // Volume
        let volume: Element<'_, Message> = match self.volume {
            Some(v) => {
                let level = self.dragging.unwrap_or(v.level).min(1.0);
                row![
                    mouse_area(
                        container(icon(icons::volume(level, v.muted), 18).color(if v.muted {
                            r.dim
                        } else {
                            r.text
                        }))
                        .width(28)
                        .align_x(Horizontal::Center)
                    )
                    .on_press(Message::Mute)
                    .interaction(iced::mouse::Interaction::Pointer),
                    slider(0.0..=1.0, level, Message::Volume)
                        .step(0.01_f32)
                        .on_release(Message::VolumeDone)
                        .style(move |_, _| slider::Style {
                            rail: slider::Rail {
                                backgrounds: (
                                    Background::Color(r.accent),
                                    Background::Color(r.chip_border)
                                ),
                                width: 6.0,
                                border: Border {
                                    radius: 3.0.into(),
                                    ..Border::default()
                                },
                            },
                            handle: slider::Handle {
                                shape: slider::HandleShape::Circle { radius: 8.0 },
                                background: Background::Color(r.text),
                                border_width: 0.0,
                                border_color: Color::TRANSPARENT,
                            },
                        }),
                    text(if v.muted {
                        "Muted".to_owned()
                    } else {
                        format!("{:.0}%", level * 100.0)
                    })
                    .font(INTER_SEMIBOLD)
                    .size(12)
                    .width(40)
                    .align_x(Horizontal::Right)
                    .color(if v.muted { r.dim } else { r.text }),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
            }
            None => text("No audio output found")
                .font(INTER)
                .size(12)
                .color(r.dim)
                .into(),
        };

        // Notifications
        let notes: Element<'_, Message> = match &self.notices {
            Some(n) => {
                let summary = match (n.waiting, n.history, n.recent.len()) {
                    (0, 0, 0) => "All caught up".to_owned(),
                    (0, h, _) => format!("{h} earlier"),
                    (w, h, _) => format!("{w} waiting · {h} earlier"),
                };
                let mut head = row![
                    container(
                        icon(if n.dnd { icons::BELL_OFF } else { icons::BELL }, 18).color(r.muted)
                    )
                    .width(28)
                    .align_x(Horizontal::Center),
                    column![
                        text("Notifications")
                            .font(INTER_SEMIBOLD)
                            .size(13)
                            .color(r.text),
                        text(summary).font(INTER).size(12).color(r.muted),
                    ]
                    .spacing(3)
                    .width(Length::Fill),
                    pill("Show last", Message::ShowLast, n.history > 0, r),
                ]
                .spacing(10)
                .align_y(Alignment::Center);
                if !n.recent.is_empty() {
                    head = head.push(pill("Clear", Message::Clear, n.history > 0, r));
                }
                let mut list = column![head].spacing(10);
                for item in n.recent.iter().take(3) {
                    list = list.push(recent_row(item, r));
                }
                list.into()
            }
            None => text("No notification daemon")
                .font(INTER)
                .size(12)
                .color(r.dim)
                .into(),
        };

        let line = || {
            container(Space::new().width(Length::Fill).height(1)).style(move |_| container::Style {
                background: Some(Background::Color(r.chip_border)),
                ..container::Style::default()
            })
        };

        container(
            column![
                row![
                    ui::section_label("Quick settings", r),
                    Space::new().width(Length::Fill),
                    button(icon(icons::CLOSE, 14))
                        .padding(6)
                        .on_press(Message::Toggle)
                        .style(move |_, status| button::Style {
                            background: (status == button::Status::Hovered)
                                .then_some(Background::Color(r.chip)),
                            text_color: r.muted,
                            border: Border {
                                radius: 8.0.into(),
                                ..Border::default()
                            },
                            ..button::Style::default()
                        }),
                ]
                .align_y(Alignment::Center),
                media,
                toggles,
                volume,
                line(),
                notes,
            ]
            .spacing(16),
        )
        .width(PANEL)
        .padding(16)
        .style(r.panel_style())
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

fn tile(
    glyph: char,
    bg: Color,
    open: bool,
    count: Option<u32>,
    live: bool,
    r: Rail,
) -> Element<'static, Message> {
    let face = container(icon(glyph, 22).color(r.text))
        .width(TILE)
        .height(TILE)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                radius: 14.0.into(),
                width: 1.5,
                color: if open { r.accent } else { Color::TRANSPARENT },
            },
            ..container::Style::default()
        });
    let mut layers = stack![container(face).padding(3)];
    if let Some(n) = count {
        layers = layers.push(
            container(
                container(
                    text(n.min(99).to_string())
                        .font(INTER_SEMIBOLD)
                        .size(10)
                        .color(r.on_accent),
                )
                .width(18)
                .height(18)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| container::Style {
                    background: Some(Background::Color(r.accent)),
                    border: Border {
                        radius: 9.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
            )
            .width(TILE + 6.0)
            .align_x(Horizontal::Right),
        );
    }
    if live {
        // A small dot while music is playing.
        layers = layers.push(
            container(container(Space::new().width(8).height(8)).style(move |_| {
                container::Style {
                    background: Some(Background::Color(r.accent)),
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            }))
            .width(TILE + 6.0)
            .height(TILE + 6.0)
            .padding(6)
            .align_x(Horizontal::Right)
            .align_y(Vertical::Bottom),
        );
    }
    mouse_area(layers)
        .on_press(Message::Toggle)
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// One line per notification, as on the canvas: dot, summary, then the body.
fn recent_row(item: &system::Recent, r: Rail) -> Element<'static, Message> {
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

fn toggle(
    (glyph_on, glyph_off): (char, char),
    label: &str,
    state: Option<bool>,
    msg: Message,
    r: Rail,
) -> Element<'static, Message> {
    let on = state == Some(true);
    let available = state.is_some();
    let state_line = match state {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Unavailable",
    };
    let b = button(
        container(
            column![
                icon(if on { glyph_on } else { glyph_off }, 18),
                Space::new().height(4),
                text(label.to_owned())
                    .font(if on { INTER_SEMIBOLD } else { INTER })
                    .size(12),
                text(state_line)
                    .font(INTER)
                    .size(10)
                    .color(if on { r.text } else { r.dim }),
            ]
            .spacing(2)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(80)
    .padding([0, 6])
    .style(move |_, status| button::Style {
        background: Some(Background::Color(match (on, status) {
            (true, _) => Color {
                a: 0.22,
                ..r.accent
            },
            (false, button::Status::Hovered) => Color {
                a: r.chip.a * 1.8,
                ..r.chip
            },
            _ => r.chip,
        })),
        text_color: if available { r.text } else { r.dim },
        border: Border {
            radius: 12.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    });
    if available { b.on_press(msg) } else { b }.into()
}

fn pill(label: &str, msg: Message, enabled: bool, r: Rail) -> Element<'static, Message> {
    let b = button(
        container(text(label.to_owned()).font(INTER).size(12))
            .align_x(Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([8, 14])
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
    if enabled { b.on_press(msg) } else { b }.into()
}

fn round(glyph: char, msg: Message, primary: bool, r: Rail) -> Element<'static, Message> {
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
    .on_press(msg)
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
