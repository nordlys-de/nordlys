//! The launcher (Super+Space): one frosted panel that finds apps, Nordlys
//! commands and files. Matches the Launcher board on the design canvas.
//!
//! It runs once and exits: the compositor binds Super+Space to
//! `nordlys-shell launcher`.

use crate::bar::{INTER, INTER_SEMIBOLD, MONO, color};
use crate::icons::{self, icon};
use crate::search::{Catalog, Item, Kind, Mode};
use iced::event::{self, Event};
use iced::gradient::Linear;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::text::Wrapping;
use iced::widget::{Space, column, container, mouse_area, operation, row, text, text_input};
use iced::{
    Alignment, Background, Border, Color, Element, Length, Radians, Shadow, Subscription, Task,
    Vector,
};
use iced_layershell::to_layer_message;
use nordlys_config::{Config, Palette};

const QUERY: &str = "launcher-query";
const PANEL_WIDTH: f32 = 840.0;
const DETAIL_WIDTH: f32 = 288.0;
const BODY_HEIGHT: f32 = 530.0;

pub struct Launcher {
    palette: Palette,
    catalog: Catalog,
    query: String,
    results: Vec<Item>,
    selected: usize,
    ctrl: bool,
    status: Option<String>,
    /// A command that asks before it runs (restart, shut down…), armed by
    /// the first Enter; the second runs it.
    armed: Option<String>,
    preview: bool,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Query(String),
    Move(i32),
    Hover(usize),
    Run(usize),
    Submit,
    Ctrl(bool),
    Prefix(&'static str),
    FilesIndexed(Vec<std::path::PathBuf>),
    ClipsLoaded(Vec<(u64, String)>),
    Close,
}

impl Launcher {
    pub fn new(config: &Config, preview: bool, query: String) -> (Self, Task<Message>) {
        let catalog = Catalog::load(config);
        let results = catalog.search(&query);
        let me = Self {
            palette: config.palette(),
            catalog,
            query,
            results,
            selected: 0,
            ctrl: false,
            status: None,
            armed: None,
            preview,
        };
        let index = Task::perform(async { crate::files::index_home() }, Message::FilesIndexed);
        let clips = Task::perform(
            async { crate::search::clip_history() },
            Message::ClipsLoaded,
        );
        (me, Task::batch([operation::focus(QUERY), index, clips]))
    }

    fn refresh(&mut self) {
        self.results = self.catalog.search(&self.query);
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Query(q) => {
                self.query = q;
                self.selected = 0;
                self.status = None;
                self.armed = None;
                self.refresh();
            }
            Message::Prefix(p) => {
                let (_, rest) = Mode::split(&self.query);
                self.query = format!("{p}{rest}");
                self.selected = 0;
                self.refresh();
                return Task::batch([
                    operation::focus(QUERY),
                    operation::move_cursor_to_end(QUERY),
                ]);
            }
            Message::Move(d) if !self.results.is_empty() => {
                let n = self.results.len() as i32;
                self.selected = (self.selected as i32 + d).rem_euclid(n) as usize;
                self.disarm();
            }
            Message::Hover(i) => self.selected = i,
            Message::Ctrl(down) => self.ctrl = down,
            Message::Run(i) => {
                self.selected = i;
                return self.run();
            }
            Message::Submit => return self.run(),
            Message::FilesIndexed(files) => {
                self.catalog.set_files(files);
                self.refresh();
            }
            Message::ClipsLoaded(clips) => {
                self.catalog.set_clips(clips);
                self.refresh();
            }
            Message::Close => return iced::exit(),
            _ => {}
        }
        Task::none()
    }

    /// Moving off an armed command takes the question back.
    fn disarm(&mut self) {
        if self.armed.take().is_some() {
            self.status = None;
        }
    }

    fn run(&mut self) -> Task<Message> {
        let Some(item) = self.results.get(self.selected).cloned() else {
            return Task::none();
        };
        if let crate::search::Action::Confirm { verb, .. } = &item.action
            && self.armed.as_deref() != Some(item.name.as_str())
        {
            self.armed = Some(item.name.clone());
            self.status = Some(format!("Press Enter again to {verb}"));
            return Task::none();
        }
        match self.catalog.run(&item, self.ctrl) {
            Ok(()) => iced::exit(),
            Err(e) => {
                self.status = Some(format!("{}: {e}", item.name));
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        event::listen_with(on_event)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let p = &self.palette;
        let g = Look::of(p);
        let (mode, _) = Mode::split(&self.query);

        // Search row
        let input = text_input(
            "Type to search · > commands · / files · # clipboard",
            &self.query,
        )
        .id(QUERY)
        .on_input(Message::Query)
        .size(19)
        .font(INTER)
        .padding([10, 0])
        .style(move |_, _| text_input::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            icon: g.muted,
            placeholder: g.placeholder,
            value: g.text,
            selection: g.accent_soft,
        });
        let search = row![icon(icons::SEARCH, 20).color(g.muted), input, kbd("Esc", g)]
            .spacing(14)
            .align_y(Alignment::Center)
            .padding([12, 20]);

        // Mode chips
        let chip = |label: &'static str, prefix: &'static str, on: bool| {
            mouse_area(
                container(
                    text(label)
                        .font(if on { INTER_SEMIBOLD } else { INTER })
                        .size(12)
                        .color(if on { g.text } else { g.muted }),
                )
                .padding([5, 10])
                .style(move |_| container::Style {
                    background: Some(Background::Color(if on { g.accent_soft } else { g.chip })),
                    border: Border {
                        radius: 7.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
            )
            .on_press(Message::Prefix(prefix))
        };
        let modes = row![
            chip("All", "", mode == Mode::All),
            chip("> Commands", "> ", mode == Mode::Commands),
            chip("/ Files", "/ ", mode == Mode::Files),
            chip("# Clipboard", "# ", mode == Mode::Clipboard),
        ]
        .spacing(6)
        .padding([10, 20]);

        // Results, grouped under headers
        let mut list = column![].spacing(2).padding([8, 10]).width(Length::Fill);
        let mut last: Option<Kind> = None;
        for (i, it) in self.results.iter().enumerate() {
            if last != Some(it.kind) {
                list = list.push(
                    container(
                        text(it.kind.label().to_uppercase())
                            .font(INTER_SEMIBOLD)
                            .size(11)
                            .color(g.dim),
                    )
                    .padding([8, 12]),
                );
                last = Some(it.kind);
            }
            list = list.push(result_row(i, it, i == self.selected, g));
        }
        if self.results.is_empty() {
            let hint = if mode == Mode::Files && !self.catalog.files_ready() {
                "Indexing your home folder…"
            } else if mode == Mode::Clipboard {
                "Nothing copied yet, or cliphist isn't running."
            } else {
                "No matches. Start with > for commands or / for files."
            };
            list = list
                .push(container(text(hint).font(INTER).size(13).color(g.muted)).padding([24, 12]));
        }

        let body = row![
            container(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .clip(true),
            self.detail(g),
        ]
        .height(BODY_HEIGHT);

        // Footer
        let mut footer = row![
            row![kbd("↑", g), kbd("↓", g), hint("move", g)]
                .spacing(4)
                .align_y(Alignment::Center),
            row![kbd("Enter", g), hint("open", g)]
                .spacing(6)
                .align_y(Alignment::Center),
            row![kbd("Esc", g), hint("close", g)]
                .spacing(6)
                .align_y(Alignment::Center),
        ]
        .spacing(18)
        .align_y(Alignment::Center);
        if let Some(s) = &self.status {
            footer = footer.push(
                text(s.clone())
                    .font(INTER)
                    .size(12)
                    .color(g.error)
                    .wrapping(Wrapping::None),
            );
        }
        let footer = footer
            .push(Space::new().width(Length::Fill))
            .push(hint(&format!("{} results", self.results.len()), g))
            .padding([12, 20]);

        let panel = container(column![
            search,
            divider(g),
            modes,
            divider(g),
            body,
            divider(g),
            footer
        ])
        .width(PANEL_WIDTH)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(Background::Color(g.panel)),
            border: Border {
                radius: 18.0.into(),
                width: 1.0,
                color: g.panel_border,
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.55),
                offset: Vector::new(0.0, 30.0),
                blur_radius: 90.0,
            },
            ..container::Style::default()
        });

        // Clicking anywhere outside the panel closes the launcher.
        let outside = |w: Length, h: Length| {
            mouse_area(Space::new().width(w).height(h)).on_press(Message::Close)
        };
        let page = column![
            outside(Length::Fill, Length::Fixed(112.0)),
            row![
                outside(Length::Fill, Length::Shrink),
                panel,
                outside(Length::Fill, Length::Shrink)
            ],
            outside(Length::Fill, Length::Fill),
        ];

        let scrim = g.scrim;
        let preview = self.preview;
        let dark = p.dark;
        let ground = color(p.ground);
        container(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(if preview {
                    // Stand-in for the blurred desktop behind the scrim.
                    let (a, b) = if dark {
                        (
                            Color::from_rgb8(0x24, 0x36, 0x8C),
                            Color::from_rgb8(0x4B, 0x2D, 0x96),
                        )
                    } else {
                        (
                            Color::from_rgb8(0xB8, 0xC9, 0xFF),
                            Color::from_rgb8(0xD9, 0xCC, 0xFF),
                        )
                    };
                    Background::Gradient(
                        Linear::new(Radians(0.9))
                            .add_stop(0.0, a)
                            .add_stop(0.5, ground)
                            .add_stop(1.0, b)
                            .into(),
                    )
                } else {
                    Background::Color(scrim)
                }),
                ..container::Style::default()
            })
            .into()
    }

    fn detail(&self, g: Look) -> Element<'_, Message> {
        let Some(it) = self.results.get(self.selected) else {
            return container(Space::new()).width(DETAIL_WIDTH).into();
        };
        let (open, extra): (&str, Option<(&str, &str)>) = match it.kind {
            Kind::App => ("Open", None),
            Kind::Command => ("Run", None),
            Kind::File => ("Open", Some(("Open folder", "Ctrl Enter"))),
            Kind::Clip => ("Copy", None),
        };
        let mut actions = column![action_row(open, "Enter", true, g)].spacing(4);
        if let Some((label, key)) = extra {
            actions = actions.push(action_row(label, key, false, g));
        }
        if let Some(key) = &it.key {
            actions = actions.push(action_row("Shortcut", key, false, g));
        }
        let what = match it.kind {
            Kind::App => "Application",
            Kind::Command => "Nordlys command",
            Kind::File => "File",
            Kind::Clip => "Copied earlier",
        };
        container(
            column![
                tile(it, 56.0, g),
                column![
                    text(it.name.clone())
                        .font(INTER_SEMIBOLD)
                        .size(18)
                        .color(g.text),
                    text(what).font(INTER).size(12).color(g.dim),
                ]
                .spacing(4),
                text(it.desc.clone()).font(INTER).size(13).color(g.muted),
                actions,
            ]
            .spacing(16),
        )
        .width(DETAIL_WIDTH)
        .height(Length::Fill)
        .padding([24, 20])
        .style(move |_| container::Style {
            background: Some(Background::Color(g.detail)),
            border: Border {
                width: 0.0,
                ..Border::default()
            },
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

fn on_event(event: Event, _status: event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => match key {
            Key::Named(Named::ArrowDown) => Some(Message::Move(1)),
            Key::Named(Named::ArrowUp) => Some(Message::Move(-1)),
            Key::Named(Named::Tab) => Some(Message::Move(if modifiers.shift() { -1 } else { 1 })),
            Key::Named(Named::Enter) => Some(Message::Submit),
            Key::Named(Named::Escape) => Some(Message::Close),
            Key::Named(Named::Control) => Some(Message::Ctrl(true)),
            _ => None,
        },
        Event::Keyboard(keyboard::Event::KeyReleased {
            key: Key::Named(Named::Control),
            ..
        }) => Some(Message::Ctrl(false)),
        Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Message::Ctrl(m.control())),
        _ => None,
    }
}

fn result_row(i: usize, it: &Item, selected: bool, g: Look) -> Element<'static, Message> {
    let mut line = row![
        tile(it, 28.0, g),
        text(it.name.clone())
            .font(INTER_SEMIBOLD)
            .size(14)
            .color(g.text)
            .wrapping(Wrapping::None),
        container(
            text(it.desc.clone())
                .font(INTER)
                .size(13)
                .color(g.dim)
                .wrapping(Wrapping::None)
        )
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    if let Some(k) = &it.key {
        line = line.push(kbd(k, g));
    }
    let row_box = container(line)
        .height(40)
        .padding([0, 12])
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(if selected {
                g.selected
            } else {
                Color::TRANSPARENT
            })),
            border: Border {
                radius: 10.0.into(),
                width: if selected { 1.0 } else { 0.0 },
                color: g.selected_border,
            },
            ..container::Style::default()
        });
    mouse_area(row_box)
        .on_enter(Message::Hover(i))
        .on_press(Message::Run(i))
        .into()
}

/// The coloured letter tile used for apps; commands and files get an icon.
fn tile(it: &Item, size: f32, g: Look) -> Element<'static, Message> {
    let (bg, fg) = match it.kind {
        Kind::App => (
            Color {
                a: 0.24,
                ..crate::ui::tint(&it.name)
            },
            g.text,
        ),
        Kind::Command => (g.accent_soft, g.accent),
        Kind::File | Kind::Clip => (g.chip, g.muted),
    };
    let face: Element<'static, Message> = match it.icon {
        Some(glyph) => icon(glyph, size * 0.46).color(fg).into(),
        None => text(
            it.name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default(),
        )
        .font(INTER_SEMIBOLD)
        .size(size * 0.42)
        .color(fg)
        .into(),
    };
    container(face)
        .width(size)
        .height(size)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                radius: (size * 0.28).into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn action_row(label: &str, key: &str, first: bool, g: Look) -> Element<'static, Message> {
    container(
        row![
            text(label.to_owned()).font(INTER).size(13).color(g.text),
            Space::new().width(Length::Fill),
            kbd(key, g)
        ]
        .align_y(Alignment::Center),
    )
    .padding([7, 10])
    .style(move |_| container::Style {
        background: Some(Background::Color(if first {
            g.chip
        } else {
            Color::TRANSPARENT
        })),
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

fn kbd(label: &str, g: Look) -> Element<'static, Message> {
    container(text(label.to_owned()).font(MONO).size(11).color(g.kbd_text))
        .padding([3, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(g.chip)),
            border: Border {
                radius: 5.0.into(),
                width: 1.0,
                color: g.chip_border,
            },
            ..container::Style::default()
        })
        .into()
}

fn hint(label: &str, g: Look) -> Element<'static, Message> {
    text(label.to_owned())
        .font(INTER)
        .size(12)
        .color(g.dim)
        .into()
}

fn divider(g: Look) -> Element<'static, Message> {
    container(Space::new().width(Length::Fill).height(1))
        .style(move |_| container::Style {
            background: Some(Background::Color(g.line)),
            ..container::Style::default()
        })
        .into()
}

/// A magnifying glass drawn from two boxes, so it needs no icon font.
/// Panel colours from the canvas.
#[derive(Clone, Copy)]
struct Look {
    panel: Color,
    panel_border: Color,
    detail: Color,
    scrim: Color,
    line: Color,
    chip: Color,
    chip_border: Color,
    selected: Color,
    selected_border: Color,
    text: Color,
    muted: Color,
    dim: Color,
    placeholder: Color,
    kbd_text: Color,
    accent: Color,
    accent_soft: Color,
    error: Color,
}

impl Look {
    fn of(p: &Palette) -> Self {
        let accent = color(p.accent);
        let soft = |a| Color { a, ..accent };
        if p.dark {
            Look {
                panel: Color::from_rgba8(22, 26, 36, 0.78),
                panel_border: Color::from_rgba8(255, 255, 255, 0.12),
                detail: Color::from_rgba8(255, 255, 255, 0.025),
                scrim: Color::from_rgba8(5, 7, 11, 0.40),
                line: Color::from_rgba8(255, 255, 255, 0.07),
                chip: Color::from_rgba8(255, 255, 255, 0.07),
                chip_border: Color::from_rgba8(255, 255, 255, 0.10),
                selected: soft(0.14),
                selected_border: soft(0.45),
                text: color(p.text),
                muted: color(p.muted),
                dim: color(p.dim),
                placeholder: Color::from_rgb8(0x7F, 0x8A, 0x99),
                kbd_text: Color::from_rgb8(0xC8, 0xD0, 0xDC),
                accent,
                accent_soft: soft(0.22),
                error: color(p.ansi[9]),
            }
        } else {
            Look {
                panel: Color::from_rgba8(255, 255, 255, 0.80),
                panel_border: Color::from_rgba8(255, 255, 255, 0.85),
                detail: Color::from_rgba8(20, 24, 33, 0.025),
                scrim: Color::from_rgba8(238, 241, 246, 0.30),
                line: Color::from_rgba8(20, 24, 33, 0.08),
                chip: Color::from_rgba8(20, 24, 33, 0.06),
                chip_border: Color::from_rgba8(20, 24, 33, 0.12),
                selected: soft(0.12),
                selected_border: soft(0.40),
                text: color(p.text),
                muted: color(p.muted),
                dim: color(p.dim),
                placeholder: Color::from_rgb8(0x6B, 0x74, 0x82),
                kbd_text: color(p.muted),
                accent,
                accent_soft: soft(0.16),
                error: color(p.ansi[1]),
            }
        }
    }
}
