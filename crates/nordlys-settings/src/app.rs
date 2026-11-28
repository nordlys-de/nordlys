//! The window: a sidebar of pages and the page you're on, with notes that
//! say what happened after each change.

use crate::icons::{Icon, icon};
use crate::pages::{self, PageState};
use crate::store::{Hypr, NordlysToml};
use crate::theme::{BOLD, C, INTER, Kind, SEMIBOLD, alpha};
use crate::ui::{self, El};
use iced::widget::{button, column, container, opaque, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, Padding, Subscription, Task};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Appearance,
    Windows,
    Groups,
    Notifications,
    LockIdle,
    Keyboard,
    Mouse,
    Network,
    Bluetooth,
    Sound,
    Displays,
    Power,
    DateTime,
    DefaultApps,
    Startup,
    Services,
    Advanced,
    About,
}

impl Page {
    pub const ALL: [Page; 18] = [
        Page::Appearance,
        Page::Windows,
        Page::Groups,
        Page::Notifications,
        Page::LockIdle,
        Page::Keyboard,
        Page::Mouse,
        Page::Network,
        Page::Bluetooth,
        Page::Sound,
        Page::Displays,
        Page::Power,
        Page::DateTime,
        Page::DefaultApps,
        Page::Startup,
        Page::Services,
        Page::Advanced,
        Page::About,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Windows => "Windows & stage",
            Page::Groups => "App groups",
            Page::Notifications => "Notifications",
            Page::LockIdle => "Lock & idle",
            Page::Keyboard => "Keyboard",
            Page::Mouse => "Mouse & touchpad",
            Page::Network => "Network",
            Page::Bluetooth => "Bluetooth",
            Page::Sound => "Sound",
            Page::Displays => "Displays",
            Page::Power => "Power",
            Page::DateTime => "Date & time",
            Page::DefaultApps => "Default apps",
            Page::Startup => "Startup apps",
            Page::Services => "Session services",
            Page::Advanced => "Advanced",
            Page::About => "About",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Page::Appearance => "Light or dark, the accent colour, glass and the wallpaper.",
            Page::Windows => "How the stage and the shelf behave, gaps, corners and motion.",
            Page::Groups => "Sets of apps that open together with Super G and a key.",
            Page::Notifications => "Do not disturb, what's waiting and what came in.",
            Page::LockIdle => "When the screen dims, locks, turns off and the machine sleeps.",
            Page::Keyboard => "Layouts, key repeat and every Nordlys shortcut.",
            Page::Mouse => "Pointer speed, scrolling and touchpad gestures.",
            Page::Network => "Wi-Fi, wired and saved connections, and this computer's name.",
            Page::Bluetooth => "Pair, connect and forget devices.",
            Page::Sound => "Outputs, inputs and the apps playing right now.",
            Page::Displays => "Resolution, refresh rate, scale, arrangement and brightness.",
            Page::Power => "Battery, power mode and turning off.",
            Page::DateTime => "Time zone, network time and language.",
            Page::DefaultApps => "Which app opens links, folders, pictures and more.",
            Page::Startup => "Apps that start when you log in.",
            Page::Services => "The pieces of the Nordlys session and anything failing.",
            Page::Advanced => "Config files, resets and what Nordlys is running on.",
            Page::About => "This computer and the system on it.",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Page::Appearance => Icon::Palette,
            Page::Windows => Icon::LayoutDashboard,
            Page::Groups => Icon::Layers,
            Page::Notifications => Icon::Bell,
            Page::LockIdle => Icon::Lock,
            Page::Keyboard => Icon::Keyboard,
            Page::Mouse => Icon::Mouse,
            Page::Network => Icon::Wifi,
            Page::Bluetooth => Icon::Bluetooth,
            Page::Sound => Icon::Volume2,
            Page::Displays => Icon::Monitor,
            Page::Power => Icon::Zap,
            Page::DateTime => Icon::Clock,
            Page::DefaultApps => Icon::Package,
            Page::Startup => Icon::Rocket,
            Page::Services => Icon::Server,
            Page::Advanced => Icon::Wrench,
            Page::About => Icon::Info,
        }
    }

    fn group(self) -> &'static str {
        match self {
            Page::Appearance
            | Page::Windows
            | Page::Groups
            | Page::Notifications
            | Page::LockIdle
            | Page::Keyboard
            | Page::Mouse => "Desktop",
            Page::Network
            | Page::Bluetooth
            | Page::Sound
            | Page::Displays
            | Page::Power
            | Page::DateTime
            | Page::DefaultApps
            | Page::Startup => "System",
            Page::Services | Page::Advanced | Page::About => "Nordlys",
        }
    }

    /// Words a search should find this page by.
    fn keywords(self) -> &'static str {
        match self {
            Page::Appearance => {
                "theme dark light mode accent colour color blur glass transparency motion wallpaper background gtk qt roundness corners top bar topbar gap gaps screen edge shelf sides hide widgets clock tray battery bluetooth network sound"
            }
            Page::Windows => {
                "stage shelf hide show layout two-up tiled monocle split gaps border rounding corners animations shadow dim opacity ultrawide rails peek"
            }
            Page::Groups => "groups apps workspace super g launch",
            Page::Notifications => "do not disturb dnd toasts history clear dunst",
            Page::LockIdle => {
                "lock screen idle dim timeout suspend sleep screen off hypridle password pam"
            }
            Page::Keyboard => {
                "layout keymap language repeat delay rate numlock shortcuts keys bindings caps"
            }
            Page::Mouse => {
                "pointer cursor speed sensitivity acceleration scroll natural touchpad tap click left handed focus"
            }
            Page::Network => {
                "wifi wi-fi wireless ethernet wired vpn airplane hostname ip dns connection"
            }
            Page::Bluetooth => "pair devices headphones speaker mouse keyboard",
            Page::Sound => "audio volume output input microphone speakers headphones mute pipewire",
            Page::Displays => {
                "monitor screen resolution refresh rate scale hidpi rotate arrangement brightness night light vrr"
            }
            Page::Power => {
                "battery power profile performance saver suspend hibernate restart shut down log out"
            }
            Page::DateTime => "time zone timezone clock ntp locale language region",
            Page::DefaultApps => "browser email files mime open with default",
            Page::Startup => "autostart login startup",
            Page::Services => "systemd units restart logs journal failed bar shelf",
            Page::Advanced => "config files reset reload hyprland lua toml edit",
            Page::About => "system info cpu memory gpu disk kernel version hostname",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Page::Appearance => "appearance",
            Page::Windows => "windows",
            Page::Groups => "groups",
            Page::Notifications => "notifications",
            Page::LockIdle => "lock",
            Page::Keyboard => "keyboard",
            Page::Mouse => "mouse",
            Page::Network => "network",
            Page::Bluetooth => "bluetooth",
            Page::Sound => "sound",
            Page::Displays => "displays",
            Page::Power => "power",
            Page::DateTime => "time",
            Page::DefaultApps => "default-apps",
            Page::Startup => "startup",
            Page::Services => "services",
            Page::Advanced => "advanced",
            Page::About => "about",
        }
    }

    pub fn from_slug(s: &str) -> Option<Page> {
        let s = s.to_lowercase();
        Page::ALL
            .into_iter()
            .find(|p| p.slug() == s || p.title().to_lowercase() == s)
    }

    fn matches(self, q: &str) -> bool {
        let q = q.trim().to_lowercase();
        q.is_empty()
            || q.split_whitespace()
                .all(|w| self.title().to_lowercase().contains(w) || self.keywords().contains(w))
    }
}

/// What every page can reach: colours, the config files and the note
/// at the bottom of the window.
pub struct Ctx {
    pub c: C,
    pub toml: NordlysToml,
    pub hypr: Hypr,
    toast: Option<Toast>,
}

struct Toast {
    text: String,
    error: bool,
    at: Instant,
}

impl Ctx {
    pub fn done(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    pub fn fail(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    /// Say how a change went: `ok` if it worked, the reason if not.
    pub fn report<T>(&mut self, r: &Result<T, String>, ok: &str) {
        match r {
            Ok(_) if !ok.is_empty() => self.done(ok),
            Ok(_) => {}
            Err(e) => self.fail(e.clone()),
        }
    }

    /// nordlys.toml changed: recolour from it.
    pub fn restyle(&mut self) {
        self.c = C::of(&self.toml.config);
    }
}

macro_rules! pages {
    ($($variant:ident => $module:ident),* $(,)?) => {
        struct Pages { $($module: pages::$module::State,)* }

        // Messages are short-lived; boxing the big ones isn't worth it.
        #[allow(clippy::large_enum_variant)]
        #[derive(Debug, Clone)]
        pub enum PageMsg { $($variant(<pages::$module::State as PageState>::Msg),)* }

        impl Pages {
            fn new() -> Self {
                Self { $($module: Default::default(),)* }
            }
            fn open(&mut self, p: Page, ctx: &mut Ctx) -> Task<PageMsg> {
                match p { $(Page::$variant => self.$module.open(ctx).map(PageMsg::$variant),)* }
            }
            fn tick(&mut self, p: Page, ctx: &mut Ctx) -> Task<PageMsg> {
                match p { $(Page::$variant => self.$module.tick(ctx).map(PageMsg::$variant),)* }
            }
            fn update(&mut self, m: PageMsg, ctx: &mut Ctx) -> Task<PageMsg> {
                match m { $(PageMsg::$variant(m) => self.$module.update(m, ctx).map(PageMsg::$variant),)* }
            }
            fn view<'a>(&'a self, p: Page, ctx: &'a Ctx) -> El<'a, PageMsg> {
                match p { $(Page::$variant => self.$module.view(ctx).map(PageMsg::$variant),)* }
            }
            fn dialog<'a>(&'a self, p: Page, ctx: &'a Ctx) -> Option<El<'a, PageMsg>> {
                match p { $(Page::$variant => self.$module.dialog(ctx).map(|e| e.map(PageMsg::$variant)),)* }
            }
        }
    };
}

pages! {
    Appearance => appearance,
    Windows => windows,
    Groups => groups,
    Notifications => notifications,
    LockIdle => lock,
    Keyboard => keyboard,
    Mouse => mouse,
    Network => network,
    Bluetooth => bluetooth,
    Sound => sound,
    Displays => displays,
    Power => power,
    DateTime => datetime,
    DefaultApps => default_apps,
    Startup => startup,
    Services => services,
    Advanced => advanced,
    About => about,
}

struct App {
    page: Page,
    search: String,
    ctx: Ctx,
    pages: Pages,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum Message {
    Go(Page),
    Search(String),
    SearchSubmit,
    Tick,
    Clock,
    LiveHypr(std::collections::HashMap<String, crate::store::V>),
    DismissToast,
    Page(PageMsg),
}

pub fn run(start: Page) -> Result<(), String> {
    iced::application(move || App::new(start), App::update, App::view)
        .title(|a: &App| format!("{} · Settings", a.page.title()))
        .theme(|a: &App| a.ctx.c.theme())
        .style(|a: &App, _| iced::theme::Style {
            background_color: a.ctx.c.window(),
            text_color: a.ctx.c.text,
        })
        .subscription(App::subscription)
        .default_font(INTER)
        .window(iced::window::Settings {
            size: iced::Size::new(1120.0, 780.0),
            min_size: Some(iced::Size::new(780.0, 520.0)),
            transparent: true,
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: "nordlys-settings".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
        .map_err(|e| e.to_string())
}

impl App {
    fn new(start: Page) -> (Self, Task<Message>) {
        let toml = NordlysToml::load();
        let mut ctx = Ctx {
            c: C::of(&toml.config),
            toml,
            hypr: Hypr::load(),
            toast: None,
        };
        if let Some(p) = ctx.toml.problem.clone() {
            ctx.fail(p);
        }
        let mut app = App {
            page: start,
            search: String::new(),
            pages: Pages::new(),
            ctx,
        };
        let live = crate::sys::job(
            || Ok(crate::store::hypr::read_live()),
            |r| Message::LiveHypr(r.unwrap_or_default()),
        );
        let open = app.pages.open(start, &mut app.ctx).map(Message::Page);
        (app, Task::batch([live, open]))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Go(p) => {
                if p == self.page {
                    return Task::none();
                }
                self.page = p;
                self.pages.open(p, &mut self.ctx).map(Message::Page)
            }
            Message::Search(s) => {
                self.search = s;
                Task::none()
            }
            Message::SearchSubmit => {
                match Page::ALL.into_iter().find(|p| p.matches(&self.search)) {
                    Some(p) => self.update(Message::Go(p)),
                    None => Task::none(),
                }
            }
            Message::LiveHypr(live) => {
                self.ctx.hypr.take_live(live);
                Task::none()
            }
            Message::Tick => {
                if self.ctx.toast.as_ref().is_some_and(|t| {
                    t.at.elapsed() > Duration::from_secs(if t.error { 9 } else { 4 })
                }) {
                    self.ctx.toast = None;
                }
                if self.ctx.toml.changed_on_disk() {
                    self.ctx.toml = NordlysToml::load();
                    self.ctx.restyle();
                    if let Some(p) = self.ctx.toml.problem.clone() {
                        self.ctx.fail(p);
                    }
                }
                self.pages.tick(self.page, &mut self.ctx).map(Message::Page)
            }
            Message::Clock => Task::none(),
            Message::DismissToast => {
                self.ctx.toast = None;
                Task::none()
            }
            Message::Page(PageMsg::About(pages::about::Msg::Rename)) => {
                self.update(Message::Go(pages::about::RENAME_ON))
            }
            Message::Page(m) => self.pages.update(m, &mut self.ctx).map(Message::Page),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![iced::time::every(Duration::from_secs(3)).map(|_| Message::Tick)];
        if self.page == Page::DateTime {
            subs.push(iced::time::every(Duration::from_secs(1)).map(|_| Message::Clock));
        }
        Subscription::batch(subs)
    }

    fn view(&self) -> Element<'_, Message> {
        let c = self.ctx.c;
        let body = column![
            ui::header(c, self.page.title(), self.page.detail()),
            self.pages.view(self.page, &self.ctx).map(Message::Page),
            ui::gap(40.0),
        ]
        .spacing(26)
        .max_width(820);
        let content = scrollable(
            container(body)
                .padding(Padding::from([36, 40]))
                .center_x(Length::Fill),
        )
        .height(Length::Fill)
        .width(Length::Fill);

        let main = row![self.sidebar(), content];
        let mut layers = stack![main].width(Length::Fill).height(Length::Fill);
        if let Some(t) = &self.ctx.toast {
            let tint = if t.error { c.bad } else { c.good };
            let note = container(
                row![
                    icon(
                        if t.error {
                            Icon::CircleAlert
                        } else {
                            Icon::CircleCheck
                        },
                        18.0,
                        tint
                    ),
                    text(t.text.clone()).size(13).color(c.text),
                    button(icon(Icon::X, 14.0, c.dim))
                        .padding(4)
                        .style(c.button(Kind::Ghost))
                        .on_press(Message::DismissToast),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([10, 16]).right(8))
            .max_width(620)
            .style(c.toast(tint));
            layers = layers.push(
                container(note)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::End)
                    .padding(Padding::ZERO.bottom(22).left(240)),
            );
        }
        if let Some(d) = self.pages.dialog(self.page, &self.ctx) {
            layers = layers.push(opaque(
                container(
                    container(d.map(Message::Page))
                        .padding(26)
                        .max_width(480)
                        .style(c.dialog()),
                )
                .center(Length::Fill)
                .style(c.scrim()),
            ));
        }
        layers.into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let c = self.ctx.c;
        let mut list = column![].spacing(2);
        let mut last_group = "";
        let mut any = false;
        for p in Page::ALL.into_iter().filter(|p| p.matches(&self.search)) {
            any = true;
            if p.group() != last_group {
                last_group = p.group();
                list = list.push(
                    container(
                        text(last_group.to_uppercase())
                            .font(SEMIBOLD)
                            .size(10)
                            .color(c.dim),
                    )
                    .padding(Padding::from([0, 12]).top(14).bottom(6)),
                );
            }
            let on = p == self.page;
            list = list.push(
                button(
                    row![
                        icon(p.icon(), 17.0, if on { c.accent } else { c.muted }),
                        text(p.title())
                            .size(14)
                            .font(if on { SEMIBOLD } else { INTER })
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([8, 12])
                .style(c.button(Kind::Nav(on)))
                .on_press(Message::Go(p)),
            );
        }
        if !any {
            list = list
                .push(container(text("Nothing matches").size(13).color(c.dim)).padding([14, 12]));
        }
        let search = ui::input(c, "Search settings", &self.search)
            .on_input(Message::Search)
            .on_submit(Message::SearchSubmit);
        let head = row![
            container(icon(Icon::SlidersHorizontal, 18.0, c.on_accent))
                .center(32)
                .style(move |_| container::Style {
                    background: Some(iced::Background::Color(c.accent)),
                    border: iced::Border {
                        radius: 10.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
            column![
                text("Settings").font(BOLD).size(16).color(c.text),
                text("Nordlys").size(11).color(c.dim)
            ]
            .spacing(1)
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        container(
            column![
                head,
                search,
                scrollable(list.padding(Padding::ZERO.right(10))).height(Length::Fill)
            ]
            .spacing(18),
        )
        .padding(Padding::from([24, 16]).right(6))
        .width(250)
        .height(Length::Fill)
        .style(move |t| {
            let mut s = c.sidebar()(t);
            s.border = iced::Border {
                width: 0.0,
                ..s.border
            };
            s.shadow = iced::Shadow {
                color: alpha(iced::Color::BLACK, 0.0),
                ..Default::default()
            };
            s
        })
        .into()
    }
}
