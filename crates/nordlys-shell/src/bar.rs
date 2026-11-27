//! The top bar: workspaces and what's on stage on the left, the clock in
//! the centre, the tray, layout and battery on the right. Matches the canvas.
//!
//! The same state, update and view run as a layer-shell surface on Wayland
//! and as a plain preview window anywhere else.

use crate::apps;
use crate::calendar;
use crate::controls;
use crate::events;
use crate::icons::{self, icon};
use crate::sources::{self, Battery, Compositor, Desk};
use crate::system;
use crate::tray;
use chrono::{Datelike, NaiveDate};
use iced::alignment::Horizontal;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{
    Space, button, column, container, image, mouse_area, row, scrollable, svg, text,
};
use iced::{
    Alignment, Background, Border, Color, Element, Font, Length, Radians, Shadow, Subscription,
    Task, Vector,
};
use iced_layershell::to_layer_message;
use nordlys_config::{Config, Layout, Palette, Rgb};
use std::time::Duration;

pub const INTER: Font = Font::with_name("Inter");
pub(crate) const INTER_SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..INTER
};
pub(crate) const MONO: Font = Font::with_name("JetBrains Mono");

/// Bar height on the canvas, plus the gap above it.
pub const BAR_HEIGHT: u32 = 40;
pub const BAR_GAP: u32 = 10;

/// This same binary's launcher, so it works without anything on PATH.
fn launcher_command() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "nordlys-shell".into());
    format!("{} launcher", apps::shell_quote(&exe))
}

pub struct Bar {
    palette: Palette,
    layout: Layout,
    compositor: Compositor,
    desk: Desk,
    battery: Option<Battery>,
    clock: String,
    /// The month on show while the calendar is open.
    month: Option<NaiveDate>,
    /// Draw a stand-in wallpaper behind the bar (preview window only).
    preview: bool,
    reads: system::Reads,
    /// The screen this bar is on, when it was given one.
    screen: Option<String>,
    /// Tray icons, newest registration last.
    tray: Vec<tray::Item>,
    /// An app's tray menu, open under the bar.
    menu: Option<Menu>,
    /// The network, Bluetooth or sound panel, open under its icon.
    panel: Option<controls::Panel>,
    /// How long the screen has been recording, while it is.
    recording: Option<Duration>,
    /// Network, Bluetooth and volume, read every few seconds.
    status: system::Status,
    status_reading: bool,
}

/// A tray menu on show. Submenus stack on the levels; Back pops one.
struct Menu {
    service: String,
    path: String,
    title: String,
    /// Empty while it loads.
    levels: Vec<Vec<tray::Entry>>,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    /// Once a second, for the clock and battery.
    Tick,
    /// nordlys.toml was saved.
    Config(Config),
    /// The compositor reported a change to workspaces or windows.
    Changed,
    Desk(Option<Desk>),
    Switch(usize),
    Launcher,
    /// Open or close the calendar under the clock.
    Calendar,
    /// Move the calendar by this many months.
    Month(i32),
    Today,
    /// A click on the calendar itself, which mustn't close it.
    Keep,
    /// The tray's icons changed.
    Tray(Vec<tray::Item>),
    TrayClick(usize),
    TrayMiddle(usize),
    /// Right click: the app's menu.
    TrayMenu(usize),
    TrayScroll(usize, i32),
    MenuLoaded(String, String, Option<Vec<tray::Entry>>),
    MenuPick(tray::Entry),
    MenuBack,
    /// A click beside an open menu.
    Dismiss,
    StopRecording,
    /// Time to read the indicators again.
    Poll,
    Status(Option<system::Status>),
    /// Open a Settings page ("network", "bluetooth", "sound").
    Settings(&'static str),
    /// An indicator was clicked: open its panel, or close it.
    Panel(controls::Kind),
    Control(controls::Msg),
    PanelData(controls::Kind, Option<Result<controls::Data, String>>),
    PanelDone(Option<Result<Option<String>, String>>),
    /// Scroll on the volume: up (+1) or down (-1).
    Volume(i32),
}

/// The surface's height with the calendar closed and open.
pub const SURFACE: u32 = BAR_HEIGHT + BAR_GAP * 2;
const SURFACE_OPEN: u32 = SURFACE + CALENDAR_HEIGHT;
const CALENDAR_HEIGHT: u32 = 320;
const CALENDAR_WIDTH: f32 = 300.0;
const MENU_WIDTH: f32 = 270.0;
const MENU_ROW: u32 = 30;
const MENU_MAX: u32 = 440;

impl Bar {
    pub fn new(config: &Config, preview: bool, screen: Option<String>) -> (Self, Task<Message>) {
        let compositor = Compositor::detect();
        let mut bar = Self {
            palette: config.palette(),
            layout: config.stage.layout,
            desk: Desk::default(),
            battery: None,
            clock: String::new(),
            month: None,
            compositor,
            preview,
            reads: system::Reads::default(),
            screen,
            tray: vec![],
            menu: None,
            recording: None,
            panel: None,
            status: system::Status::default(),
            status_reading: false,
        };
        bar.tick();
        // The preview can start with a panel open, to look at it:
        // NORDLYS_PREVIEW_PANEL=network|bluetooth|sound.
        let panel = match std::env::var("NORDLYS_PREVIEW_PANEL").as_deref() {
            Ok("network") if preview => Task::done(Message::Panel(controls::Kind::Network)),
            Ok("bluetooth") if preview => Task::done(Message::Panel(controls::Kind::Bluetooth)),
            Ok("sound") if preview => Task::done(Message::Panel(controls::Kind::Sound)),
            _ => Task::none(),
        };
        let first = Task::batch([bar.read_desk(), bar.read_status(), panel]);
        (bar, first)
    }

    fn tick(&mut self) {
        self.battery = sources::battery();
        self.clock = sources::clock_text(chrono::Local::now());
        self.recording = recording();
    }

    /// Read the indicators off the UI thread, one read at a time.
    fn read_status(&mut self) -> Task<Message> {
        if self.status_reading {
            return Task::none();
        }
        self.status_reading = true;
        Task::perform(system::background(system::Status::read), Message::Status)
    }

    /// Read the open panel again, unless something is changing it.
    fn read_panel(&self) -> Task<Message> {
        let Some(panel) = &self.panel else {
            return Task::none();
        };
        if !panel.idle() {
            return Task::none();
        }
        let kind = panel.kind;
        Task::perform(system::background(move || controls::read(kind)), move |r| {
            Message::PanelData(kind, r)
        })
    }

    fn control(&mut self, msg: controls::Msg) -> Task<Message> {
        let Some(panel) = &mut self.panel else {
            return Task::none();
        };
        match msg {
            controls::Msg::Slide(id, v) => panel.sliding = Some((id, v)),
            controls::Msg::Release => {
                if let Some((id, v)) = panel.sliding {
                    return self.control(controls::Msg::Do(controls::Action::Volume(id, v)));
                }
            }
            controls::Msg::Settings(page) => {
                self.panel = None;
                return Task::batch([self.update(Message::Settings(page)), self.resize()]);
            }
            controls::Msg::Do(action) => {
                if panel.busy {
                    return Task::none();
                }
                panel.busy = true;
                panel.status = controls::working(&action);
                let grow = self.resize();
                return Task::batch([
                    grow,
                    Task::perform(
                        system::background(move || controls::perform(action)),
                        Message::PanelDone,
                    ),
                ]);
            }
        }
        Task::none()
    }

    /// Open `item`'s menu: its dbusmenu when it has one, else the app's own.
    fn open_menu(&mut self, i: usize) -> Task<Message> {
        let Some(item) = self.tray.get(i) else {
            return Task::none();
        };
        let Some(path) = item.menu.clone() else {
            tray::context_menu(item);
            return Task::none();
        };
        let service = item.service.clone();
        self.month = None;
        self.panel = None;
        self.menu = Some(Menu {
            service: service.clone(),
            path: path.clone(),
            title: item.title.clone(),
            levels: vec![],
        });
        Task::batch([
            self.resize(),
            Task::perform(
                system::background(move || {
                    let entries = tray::read_menu(&service, &path);
                    (service, path, entries)
                }),
                |r| match r {
                    Some((s, p, e)) => Message::MenuLoaded(s, p, e),
                    None => Message::Dismiss,
                },
            ),
        ])
    }

    fn read_desk(&mut self) -> Task<Message> {
        if !self.reads.start() {
            return Task::none();
        }
        let (c, screen) = (self.compositor.clone(), self.screen.clone());
        Task::perform(
            system::background(move || c.desk(screen.as_deref())),
            Message::Desk,
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.tick(),
            Message::Config(c) => {
                self.palette = c.palette();
                self.layout = c.stage.layout;
            }
            Message::Changed => return self.read_desk(),
            Message::Desk(desk) => {
                let again = self.reads.done();
                if let Some(d) = desk {
                    self.desk = d;
                }
                if again {
                    return self.read_desk();
                }
            }
            Message::Launcher => {
                // The preview has no layer shell to open it in.
                if !self.preview
                    && let Err(e) = apps::spawn_shell(&launcher_command())
                {
                    eprintln!("nordlys-shell: can't open the launcher: {e}");
                }
            }
            Message::Switch(i) => {
                if let Some(ws) = self.desk.workspaces.get(i) {
                    self.compositor.switch_to(ws);
                }
                return self.read_desk();
            }
            Message::Calendar => {
                self.menu = None;
                self.panel = None;
                self.month = match self.month {
                    Some(_) => None,
                    None => Some(calendar::first_of(today())),
                };
                return self.resize();
            }
            Message::Month(by) => {
                self.month = self.month.map(|m| calendar::shift(m, by));
            }
            Message::Today => {
                self.month = self.month.map(|_| calendar::first_of(today()));
            }
            Message::Tray(items) => {
                self.tray = items;
                // The app whose menu is open went away.
                if let Some(m) = &self.menu
                    && !self.tray.iter().any(|t| t.service == m.service)
                {
                    self.menu = None;
                    return self.resize();
                }
            }
            Message::TrayClick(i) => {
                if let Some(item) = self.tray.get(i) {
                    if item.only_menu {
                        return self.open_menu(i);
                    }
                    tray::click(item, tray::Click::Primary);
                }
            }
            Message::TrayMiddle(i) => {
                if let Some(item) = self.tray.get(i) {
                    tray::click(item, tray::Click::Middle);
                }
            }
            Message::TrayMenu(i) => {
                let same = match (&self.menu, self.tray.get(i)) {
                    (Some(m), Some(t)) => m.service == t.service,
                    _ => false,
                };
                if same {
                    self.menu = None;
                    return self.resize();
                }
                return self.open_menu(i);
            }
            Message::TrayScroll(i, delta) => {
                if let Some(item) = self.tray.get(i) {
                    tray::scroll(item, delta);
                }
            }
            Message::MenuLoaded(service, path, entries) => {
                if let Some(m) = &mut self.menu
                    && m.service == service
                    && m.path == path
                {
                    match entries {
                        Some(e) if !e.is_empty() => {
                            m.levels = vec![e];
                        }
                        // No menu after all: let the app show its own.
                        _ => {
                            if let Some(item) = self.tray.iter().find(|t| t.service == service) {
                                tray::context_menu(item);
                            }
                            self.menu = None;
                        }
                    }
                    return self.resize();
                }
            }
            Message::MenuPick(entry) => {
                if let Some(m) = &mut self.menu {
                    if entry.children.is_empty() {
                        tray::menu_clicked(&m.service, &m.path, entry.id);
                        self.menu = None;
                    } else {
                        m.levels.push(entry.children);
                    }
                    return self.resize();
                }
            }
            Message::MenuBack => {
                if let Some(m) = &mut self.menu
                    && m.levels.len() > 1
                {
                    m.levels.pop();
                    return self.resize();
                }
            }
            Message::Dismiss => {
                self.menu = None;
                self.month = None;
                self.panel = None;
                return self.resize();
            }
            Message::Poll => return Task::batch([self.read_status(), self.read_panel()]),
            Message::Panel(kind) => {
                if self.panel.as_ref().is_some_and(|p| p.kind == kind) {
                    self.panel = None;
                    return self.resize();
                }
                self.menu = None;
                self.month = None;
                self.panel = Some(controls::Panel::new(kind));
                return Task::batch([self.resize(), self.read_panel()]);
            }
            Message::Control(msg) => return self.control(msg),
            Message::PanelData(kind, data) => {
                if let Some(panel) = &mut self.panel
                    && panel.kind == kind
                    && panel.idle()
                {
                    let data = data.unwrap_or_else(|| Err("Couldn't read it".into()));
                    if panel.data.as_ref() != Some(&data) {
                        panel.data = Some(data);
                        return self.resize();
                    }
                }
            }
            Message::PanelDone(result) => {
                if let Some(panel) = &mut self.panel {
                    panel.busy = false;
                    panel.sliding = None;
                    panel.status = match result {
                        Some(Ok(said)) => said,
                        Some(Err(e)) => Some(e),
                        None => Some("That didn't work".into()),
                    };
                }
                // Show the change, and the icons too.
                return Task::batch([self.read_panel(), self.read_status(), self.resize()]);
            }
            Message::Status(status) => {
                self.status_reading = false;
                if let Some(s) = status {
                    self.status = s;
                }
            }
            Message::Settings(page) => {
                let cmd = apps::settings_command(
                    page,
                    "echo 'nordlys-shell: nordlys-settings is not installed' >&2; exit 1",
                );
                if let Err(e) = apps::spawn_shell(&cmd) {
                    eprintln!("nordlys-shell: can't open Settings: {e}");
                }
            }
            Message::Volume(dir) => {
                let way = if dir > 0 { "up" } else { "down" };
                let exe = std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "nordlys-shell".into());
                let cmd = format!("{} osd volume {way}", apps::shell_quote(&exe));
                let _ = apps::spawn_shell(&cmd);
                // Show the new level soon, not at the next poll.
                return Task::perform(
                    system::background(|| std::thread::sleep(Duration::from_millis(250))),
                    |_| Message::Poll,
                );
            }
            Message::StopRecording => {
                if let Err(e) = apps::spawn_shell("nordlys-record stop") {
                    eprintln!("nordlys-shell: can't stop the recording: {e}");
                }
            }
            // Layer-shell control messages added by `to_layer_message`.
            _ => {}
        }
        Task::none()
    }

    /// The surface grows to hold the calendar. The space the bar reserves
    /// stays the same, so windows don't move.
    fn resize(&self) -> Task<Message> {
        if self.preview {
            return Task::none();
        }
        let height = if self.month.is_some() {
            SURFACE_OPEN
        } else if let Some(m) = &self.menu {
            SURFACE + menu_height(m) + BAR_GAP
        } else if let Some(panel) = &self.panel {
            SURFACE + panel.height() + BAR_GAP
        } else {
            SURFACE
        };
        Task::done(Message::SizeChange((0, height)))
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            events::changes(&self.compositor).map(|_| Message::Changed),
            events::config_changes().map(Message::Config),
            tray::items().map(Message::Tray),
            iced::time::every(Duration::from_secs(3)).map(|_| Message::Poll),
            // In case an event is ever missed.
            iced::time::every(Duration::from_secs(15)).map(|_| Message::Changed),
        ])
    }

    pub fn view(&self) -> Element<'_, Message> {
        let p = &self.palette;
        let glass = Glass::of(p);

        let workspaces = row(self.desk.workspaces.iter().enumerate().map(|(i, ws)| {
            let (bg, fg) = if ws.active {
                (color(p.accent), color(p.on_accent))
            } else {
                (glass.chip, color(p.muted))
            };
            let label = if ws.name.is_empty() || ws.name == ws.id.to_string() {
                row![text(ws.id.to_string()).font(MONO).size(12)]
            } else {
                row![
                    text(ws.id.to_string()).font(MONO).size(12),
                    text(ws.name.clone())
                        .font(if ws.active { INTER_SEMIBOLD } else { INTER })
                        .size(12)
                ]
                .spacing(5)
            };
            button(label.align_y(Alignment::Center))
                .padding([5, 10])
                .on_press(Message::Switch(i))
                .style(move |_, status| button::Style {
                    background: Some(Background::Color(match status {
                        button::Status::Hovered if !ws.active => glass.chip_hover,
                        _ => bg,
                    })),
                    text_color: fg,
                    border: Border {
                        radius: 8.0.into(),
                        ..Border::default()
                    },
                    ..button::Style::default()
                })
                .into()
        }))
        .spacing(4);

        let apps_button = button(
            row![
                icon(icons::GRID, 13),
                text("Apps").font(INTER_SEMIBOLD).size(12)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([5, 10])
        .on_press(Message::Launcher)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                glass.chip_hover
            } else {
                glass.chip
            })),
            text_color: color(p.text),
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        });

        let stage = row![
            icon(icons::WINDOW, 14).color(color(p.dim)),
            text(
                self.desk
                    .stage
                    .clone()
                    .unwrap_or_else(|| "Empty stage".into())
            )
            .font(INTER)
            .size(13)
            .color(color(p.muted)),
        ]
        .spacing(7)
        .align_y(Alignment::Center);

        let open = self.month.is_some();
        let clock = button(text(self.clock.clone()).font(INTER_SEMIBOLD).size(13))
            .padding([5, 10])
            .on_press(Message::Calendar)
            .style(move |_, status| button::Style {
                background: (open || status == button::Status::Hovered)
                    .then_some(Background::Color(glass.chip_hover)),
                text_color: color(p.text),
                border: Border {
                    radius: 8.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            });

        let layout_label = match (self.layout, &self.compositor) {
            (_, Compositor::Demo) => "preview · no compositor".to_owned(),
            (Layout::Auto, c) => format!("auto · {}", c.label()),
            (l, c) => format!("{} · {}", layout_name(l), c.label()),
        };
        let chip_icon = match self.compositor {
            Compositor::Demo => icons::MONITOR,
            _ => layout_icon(self.layout),
        };
        let chip = container(
            row![
                icon(chip_icon, 12).color(color(p.muted)),
                text(layout_label).font(MONO).size(11).color(color(p.muted)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([4, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(glass.chip)),
            border: Border {
                radius: 6.0.into(),
                width: 1.0,
                color: glass.chip_border,
            },
            ..container::Style::default()
        });

        let mut right = row![].spacing(12).align_y(Alignment::Center);
        if let Some(t) = self.recording {
            right = right.push(recording_chip(p, glass, t));
        }
        if !self.tray.is_empty() {
            right = right.push(tray_icons(p, glass, &self.tray, self.menu.as_ref()));
        }
        right = right.push(chip);
        if let Some(ind) = indicators(p, glass, self.status) {
            right = right.push(ind);
        }
        if let Some(b) = self.battery {
            right = right.push(battery(p, b));
        }

        let bar = container(
            row![
                container(
                    row![apps_button, workspaces, stage]
                        .spacing(12)
                        .align_y(Alignment::Center)
                )
                .width(Length::Fill),
                clock,
                container(right)
                    .width(Length::Fill)
                    .align_x(Horizontal::Right),
            ]
            .align_y(Alignment::Center),
        )
        .padding([0, 10])
        .width(Length::Fill)
        .height(BAR_HEIGHT as f32)
        .align_y(Alignment::Center)
        .style(move |_| container::Style {
            background: Some(Background::Color(glass.bar)),
            border: Border {
                radius: 12.0.into(),
                width: 1.0,
                color: glass.bar_border,
            },
            shadow: Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.18),
                offset: Vector::new(0.0, 6.0),
                blur_radius: 24.0,
            },
            ..container::Style::default()
        });

        let framed = container(bar)
            .padding([BAR_GAP as f32, 12.0])
            .width(Length::Fill);
        let framed: Element<'_, Message> = match self.month {
            Some(month) => column![
                framed,
                // A click beside the calendar closes it.
                mouse_area(
                    container(
                        mouse_area(calendar_panel(p, glass, month, today()))
                            .on_press(Message::Keep)
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Horizontal::Center)
                )
                .on_press(Message::Calendar),
            ]
            .into(),
            None => match &self.menu {
                Some(menu) => column![
                    framed,
                    mouse_area(
                        container(mouse_area(menu_panel(p, glass, menu)).on_press(Message::Keep))
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .padding([0.0, 12.0])
                            .align_x(Horizontal::Right)
                    )
                    .on_press(Message::Dismiss),
                ]
                .into(),
                None => match &self.panel {
                    Some(panel) => column![
                        framed,
                        mouse_area(
                            container(
                                mouse_area(control_panel(p, glass, panel)).on_press(Message::Keep)
                            )
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .padding([0.0, 12.0])
                            .align_x(Horizontal::Right)
                        )
                        .on_press(Message::Dismiss),
                    ]
                    .into(),
                    None => framed.into(),
                },
            },
        };
        if !self.preview {
            return framed;
        }
        // A stand-in for the blurred wallpaper so the glass reads in a plain window.
        let (a, b) = if p.dark {
            (
                Color::from_rgb8(0x2F, 0x4F, 0xC9),
                Color::from_rgb8(0x6D, 0x3F, 0xD9),
            )
        } else {
            (
                Color::from_rgb8(0x8F, 0xAE, 0xFF),
                Color::from_rgb8(0xC7, 0xB2, 0xFF),
            )
        };
        let ground = color(p.ground);
        container(framed)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Gradient(
                    Linear::new(Radians(1.2))
                        .add_stop(0.0, a)
                        .add_stop(0.55, ground)
                        .add_stop(1.0, b)
                        .into(),
                )),
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

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn calendar_panel<'a>(
    p: &Palette,
    glass: Glass,
    month: NaiveDate,
    today: NaiveDate,
) -> Element<'a, Message> {
    let (accent, on_accent) = (color(p.accent), color(p.on_accent));
    let (ink, dim) = (color(p.text), color(p.dim));
    let step = move |label: &'static str, message: Message| {
        button(text(label).font(INTER_SEMIBOLD).size(14))
            .padding([2, 10])
            .on_press(message)
            .style(move |_, status| button::Style {
                background: Some(Background::Color(if status == button::Status::Hovered {
                    glass.chip_hover
                } else {
                    glass.chip
                })),
                text_color: ink,
                border: Border {
                    radius: 7.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            })
    };
    let title = button(
        text(month.format("%B %Y").to_string())
            .font(INTER_SEMIBOLD)
            .size(14),
    )
    .padding(0)
    .on_press(Message::Today)
    .style(move |_, _| button::Style {
        text_color: ink,
        ..button::Style::default()
    });
    let cell = |content: Element<'a, Message>| {
        container(content)
            .width(Length::Fill)
            .height(30)
            .align_x(Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center)
    };
    let names = row(["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
        .map(|d| cell(text(d).font(INTER).size(11).color(dim).into()).into()));
    let mut grid = column![
        row![
            container(title).width(Length::Fill),
            step("‹", Message::Month(-1)),
            step("›", Message::Month(1)),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        names,
    ]
    .spacing(6);
    for week in calendar::weeks(month) {
        grid = grid.push(row(week.map(|day| {
            let is_today = day == today;
            let ink = if is_today {
                on_accent
            } else if day.month() == month.month() {
                ink
            } else {
                dim
            };
            let number = container(text(day.day().to_string()).font(INTER).size(12).color(ink))
                .width(28)
                .height(28)
                .align_x(Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center)
                .style(move |_| container::Style {
                    background: is_today.then_some(Background::Color(accent)),
                    border: Border {
                        radius: 14.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                });
            cell(number.into()).into()
        })));
    }
    let ground = color(p.ground);
    container(grid)
        .width(CALENDAR_WIDTH)
        .padding(14)
        .style(move |_| container::Style {
            // Darker than the bar: it sits over windows, not just wallpaper.
            background: Some(Background::Color(Color { a: 0.94, ..ground })),
            border: Border {
                radius: 14.0.into(),
                width: 1.0,
                color: glass.bar_border,
            },
            ..container::Style::default()
        })
        .into()
}

/// The tray's icons, each a little button: left click activates, middle
/// is the app's second action, right opens the menu, scroll scrolls.
fn tray_icons<'a>(
    p: &Palette,
    glass: Glass,
    items: &'a [tray::Item],
    open: Option<&Menu>,
) -> Element<'a, Message> {
    let ink = color(p.text);
    let alert = color(p.ansi[1]);
    row(items.iter().enumerate().map(|(i, item)| {
        let size = f32::from(tray::SIZE);
        let face: Element<'a, Message> = match &item.icon {
            tray::Icon::Image(h) => image(h.clone()).width(size).height(size).into(),
            tray::Icon::Svg(h, symbolic) => {
                let tint = *symbolic;
                svg(h.clone())
                    .width(size)
                    .height(size)
                    .style(move |_, _| svg::Style {
                        color: tint.then_some(ink),
                    })
                    .into()
            }
            tray::Icon::Letter(c) => text(c.to_string())
                .font(INTER_SEMIBOLD)
                .size(12)
                .color(ink)
                .into(),
        };
        let lit = open.is_some_and(|m| m.service == item.service);
        let attention = item.attention;
        mouse_area(
            container(face)
                .width(26)
                .height(26)
                .align_x(Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center)
                .style(move |_| container::Style {
                    background: lit.then_some(Background::Color(glass.chip_hover)),
                    border: Border {
                        radius: 7.0.into(),
                        width: if attention { 1.0 } else { 0.0 },
                        color: alert,
                    },
                    ..container::Style::default()
                }),
        )
        .on_press(Message::TrayClick(i))
        .on_middle_press(Message::TrayMiddle(i))
        .on_right_press(Message::TrayMenu(i))
        .on_scroll(move |d| {
            let y = match d {
                iced::mouse::ScrollDelta::Lines { y, .. } => y * 120.0,
                iced::mouse::ScrollDelta::Pixels { y, .. } => y,
            };
            Message::TrayScroll(i, -y.round() as i32)
        })
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
    }))
    .spacing(2)
    .align_y(Alignment::Center)
    .into()
}

/// Network, Bluetooth and volume, each a click from its Settings page.
/// Scrolling on the volume changes it. None when there's nothing to show.
fn indicators<'a>(p: &Palette, glass: Glass, s: system::Status) -> Option<Element<'a, Message>> {
    let (ink, dim, alert) = (color(p.text), color(p.dim), color(p.ansi[1]));
    let mut cells: Vec<Element<'a, Message>> = vec![];
    let cell = |glyph: char, tone: Color, message: Message| {
        mouse_area(
            container(icon(glyph, 15).color(tone))
                .width(24)
                .height(26)
                .align_x(Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center),
        )
        .on_press(message)
        .interaction(iced::mouse::Interaction::Pointer)
    };
    if let Some(link) = s.link {
        let (glyph, tone) = match link {
            system::Link::Wifi => (icons::WIFI, ink),
            // Lucide's ethernet icon isn't in the bundled set yet.
            system::Link::Wired | system::Link::Other => (icons::SWAP, ink),
            system::Link::Limited => (icons::WIFI, dim),
            system::Link::Offline => (icons::WIFI_OFF, alert),
        };
        cells.push(cell(glyph, tone, Message::Panel(controls::Kind::Network)).into());
    }
    if let Some(on) = s.bluetooth {
        let (glyph, tone) = if on {
            (icons::BLUETOOTH, ink)
        } else {
            (icons::BLUETOOTH_OFF, dim)
        };
        cells.push(cell(glyph, tone, Message::Panel(controls::Kind::Bluetooth)).into());
    }
    if let Some(v) = s.volume {
        let tone = if v.muted { dim } else { ink };
        cells.push(
            cell(
                icons::volume(v.level, v.muted),
                tone,
                Message::Panel(controls::Kind::Sound),
            )
            .on_scroll(|d| {
                let y = match d {
                    iced::mouse::ScrollDelta::Lines { y, .. }
                    | iced::mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                Message::Volume(if y > 0.0 { 1 } else { -1 })
            })
            .into(),
        );
    }
    if cells.is_empty() {
        return None;
    }
    Some(
        container(row(cells).spacing(2).align_y(Alignment::Center))
            .padding([0, 4])
            .style(move |_| container::Style {
                background: Some(Background::Color(glass.chip)),
                border: Border {
                    radius: 8.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into(),
    )
}

/// Height of a menu's panel: a title row, its rows, capped (it scrolls).
fn menu_height(m: &Menu) -> u32 {
    let rows: u32 = match m.levels.last() {
        Some(level) => {
            level
                .iter()
                .map(|e| if e.separator { 9 } else { MENU_ROW })
                .sum::<u32>()
                + if m.levels.len() > 1 { MENU_ROW } else { 0 }
        }
        None => MENU_ROW,
    };
    (rows + MENU_ROW + 20).min(MENU_MAX)
}

/// The network, Bluetooth or sound panel, on the same glass as menus.
fn control_panel<'a>(
    p: &Palette,
    glass: Glass,
    panel: &'a controls::Panel,
) -> Element<'a, Message> {
    let ground = color(p.ground);
    container(controls::view(p, panel).map(Message::Control))
        .width(controls::WIDTH)
        .height(panel.height() as f32)
        .padding(8)
        .style(move |_| container::Style {
            background: Some(Background::Color(Color { a: 0.94, ..ground })),
            border: Border {
                radius: 14.0.into(),
                width: 1.0,
                color: glass.bar_border,
            },
            ..container::Style::default()
        })
        .into()
}

fn menu_panel<'a>(p: &Palette, glass: Glass, m: &Menu) -> Element<'a, Message> {
    let (ink, dim, muted) = (color(p.text), color(p.dim), color(p.muted));
    let row_button = |label: Element<'a, Message>, message: Option<Message>| {
        button(label)
            .width(Length::Fill)
            .height(MENU_ROW as f32)
            .padding([6, 10])
            .on_press_maybe(message)
            .style(move |_, status| button::Style {
                background: (status == button::Status::Hovered)
                    .then_some(Background::Color(glass.chip_hover)),
                text_color: if status == button::Status::Disabled {
                    dim
                } else {
                    ink
                },
                border: Border {
                    radius: 7.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            })
    };
    let mut list = column![
        container(
            text(m.title.clone())
                .font(INTER_SEMIBOLD)
                .size(11)
                .color(muted)
        )
        .height(MENU_ROW as f32)
        .padding([8, 10])
    ];
    if m.levels.len() > 1 {
        list = list.push(row_button(
            text("‹  Back").font(INTER).size(13).into(),
            Some(Message::MenuBack),
        ));
    }
    match m.levels.last() {
        None => {
            list = list
                .push(container(text("Loading…").font(INTER).size(13).color(dim)).padding([6, 10]));
        }
        Some(level) => {
            for e in level {
                if e.separator {
                    list = list.push(
                        container(
                            container(Space::new().width(Length::Fill).height(1.0)).style(
                                move |_| container::Style {
                                    background: Some(Background::Color(glass.chip_border)),
                                    ..container::Style::default()
                                },
                            ),
                        )
                        .padding([4, 8]),
                    );
                    continue;
                }
                let mark = match e.toggle {
                    Some(true) => "✓",
                    _ => "",
                };
                let label = row![
                    text(mark).font(INTER).size(12).width(14),
                    text(e.label.clone())
                        .font(INTER)
                        .size(13)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::None),
                    text(if e.children.is_empty() { "" } else { "›" })
                        .font(INTER)
                        .size(13),
                ]
                .spacing(6)
                .align_y(Alignment::Center);
                list = list.push(row_button(
                    label.into(),
                    e.enabled.then(|| Message::MenuPick(e.clone())),
                ));
            }
        }
    }
    let ground = color(p.ground);
    container(scrollable(list.spacing(1)).height(Length::Fill))
        .width(MENU_WIDTH)
        .height(menu_height(m) as f32)
        .padding(8)
        .style(move |_| container::Style {
            background: Some(Background::Color(Color { a: 0.94, ..ground })),
            border: Border {
                radius: 14.0.into(),
                width: 1.0,
                color: glass.bar_border,
            },
            ..container::Style::default()
        })
        .into()
}

/// While `nordlys-record` runs: its pid file, and how long ago it started.
fn recording() -> Option<Duration> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    let file = std::path::Path::new(&dir).join("nordlys-record.pid");
    let pid = std::fs::read_to_string(&file).ok()?;
    let pid = pid.trim();
    if pid.is_empty() || !std::path::Path::new("/proc").join(pid).exists() {
        return None;
    }
    let started = std::fs::metadata(&file).ok()?.modified().ok()?;
    Some(started.elapsed().unwrap_or_default())
}

/// A red dot and the time; a click stops the recording.
fn recording_chip<'a>(p: &Palette, glass: Glass, t: Duration) -> Element<'a, Message> {
    let red = color(p.ansi[1]);
    let ink = color(p.text);
    let secs = t.as_secs();
    button(
        row![
            text("●").font(INTER).size(11).color(red),
            text(format!("{}:{:02}", secs / 60, secs % 60))
                .font(MONO)
                .size(11),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([4, 9])
    .on_press(Message::StopRecording)
    .style(move |_, status| button::Style {
        background: Some(Background::Color(if status == button::Status::Hovered {
            glass.chip_hover
        } else {
            glass.chip
        })),
        text_color: ink,
        border: Border {
            radius: 7.0.into(),
            width: 1.0,
            color: Color { a: 0.6, ..red },
        },
        ..button::Style::default()
    })
    .into()
}

/// Translucent fills from the canvas. The compositor supplies the blur.
#[derive(Clone, Copy)]
struct Glass {
    bar: Color,
    bar_border: Color,
    chip: Color,
    chip_hover: Color,
    chip_border: Color,
}

impl Glass {
    fn of(p: &Palette) -> Self {
        if p.dark {
            Glass {
                bar: Color::from_rgba8(18, 22, 30, 0.50),
                bar_border: Color::from_rgba8(255, 255, 255, 0.08),
                chip: Color::from_rgba8(255, 255, 255, 0.07),
                chip_hover: Color::from_rgba8(255, 255, 255, 0.13),
                chip_border: Color::from_rgba8(255, 255, 255, 0.10),
            }
        } else {
            Glass {
                bar: Color::from_rgba8(255, 255, 255, 0.50),
                bar_border: Color::from_rgba8(255, 255, 255, 0.75),
                chip: Color::from_rgba8(20, 24, 33, 0.06),
                chip_hover: Color::from_rgba8(20, 24, 33, 0.11),
                chip_border: Color::from_rgba8(20, 24, 33, 0.12),
            }
        }
    }
}

fn battery<'a>(p: &Palette, b: Battery) -> Element<'a, Message> {
    let tone = if b.charging {
        color(p.accent)
    } else if b.percent <= 15 {
        color(p.ansi[1])
    } else {
        color(p.text)
    };
    row![
        icon(icons::battery(b.percent, b.charging), 16).color(tone),
        text(format!("{}%", b.percent))
            .font(INTER)
            .size(12)
            .color(color(p.muted)),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

fn layout_icon(l: Layout) -> char {
    match l {
        Layout::Auto => icons::GRID,
        Layout::Ultrawide => icons::MONITOR,
        Layout::Rails => icons::TWO_UP,
        Layout::Peek => icons::WINDOW,
    }
}

fn layout_name(l: Layout) -> &'static str {
    match l {
        Layout::Auto => "auto",
        Layout::Ultrawide => "ultrawide",
        Layout::Rails => "rails",
        Layout::Peek => "peek",
    }
}

pub(crate) fn color(c: Rgb) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
}
