use super::{PageState, Reads};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{self, job, network};
use crate::theme::{BOLD, Kind};
use crate::ui::{self, El};
use iced::widget::{column, row, text};
use iced::{Alignment, Length, Task};

#[derive(Default)]
pub struct State {
    data: Option<network::Network>,
    reads: Reads,
    scanning: bool,
    /// Asking for a password: SSID, security, what's typed, shown or hidden.
    ask: Option<(String, String, String, bool)>,
    connecting: Option<String>,
    forget: Option<(String, String)>,
    hostname: String,
    unavailable: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<network::Network, String>),
    Wifi(bool),
    Airplane(bool),
    Scan,
    Scanned(Result<(), String>),
    Join(String, String, bool),
    Password(String),
    ShowPassword,
    SubmitPassword,
    CancelPassword,
    Connected(Result<String, String>),
    Disconnect(String),
    Up(String),
    Down(String),
    AskForget(String, String),
    Forget,
    CancelForget,
    Autoconnect(String, bool),
    Hostname(String),
    SaveHostname,
    Done(Result<(), String>, &'static str),
}

impl State {
    fn read(&mut self) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(network::read, Msg::Read)
    }
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        self.read()
    }

    fn tick(&mut self, _: &mut Ctx) -> Task<Msg> {
        if self.connecting.is_some() || self.ask.is_some() {
            return Task::none();
        }
        self.read()
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Read(r) => {
                self.reads.done();
                match r {
                    Ok(d) => {
                        if self.hostname.is_empty()
                            || self
                                .data
                                .as_ref()
                                .is_some_and(|o| o.hostname == self.hostname)
                        {
                            self.hostname = d.hostname.clone();
                        }
                        self.unavailable = None;
                        self.data = Some(d);
                    }
                    Err(e) => self.unavailable = Some(e),
                }
            }
            Msg::Wifi(on) => {
                if let Some(d) = self.data.as_mut() {
                    d.wifi_enabled = on;
                }
                return job(move || network::set_wifi(on), |r| Msg::Done(r, ""));
            }
            Msg::Airplane(on) => {
                return job(
                    move || network::set_airplane(on),
                    move |r| {
                        Msg::Done(
                            r,
                            if on {
                                "Airplane mode on: every radio is off"
                            } else {
                                "Airplane mode off"
                            },
                        )
                    },
                );
            }
            Msg::Scan => {
                self.scanning = true;
                return job(network::rescan, Msg::Scanned);
            }
            Msg::Scanned(r) => {
                self.scanning = false;
                if let Err(e) = r {
                    ctx.fail(e);
                }
                return self.read();
            }
            Msg::Join(ssid, security, secured) => {
                let saved = self
                    .data
                    .as_ref()
                    .is_some_and(|d| d.saved.iter().any(|s| s.name == ssid));
                if secured && !saved {
                    self.ask = Some((ssid, security, String::new(), false));
                } else {
                    self.connecting = Some(ssid.clone());
                    return job(
                        move || network::connect(&ssid, &security, None),
                        Msg::Connected,
                    );
                }
            }
            Msg::Password(p) => {
                if let Some(a) = self.ask.as_mut() {
                    a.2 = p;
                }
            }
            Msg::ShowPassword => {
                if let Some(a) = self.ask.as_mut() {
                    a.3 = !a.3;
                }
            }
            Msg::CancelPassword => self.ask = None,
            Msg::SubmitPassword => {
                if let Some((ssid, security, pw, _)) = self.ask.take() {
                    if pw.len() < 8 && !security.contains("WEP") {
                        ctx.fail("Wi-Fi passwords are at least 8 characters");
                        self.ask = Some((ssid, security, pw, false));
                        return Task::none();
                    }
                    self.connecting = Some(ssid.clone());
                    return job(
                        move || network::connect(&ssid, &security, Some(&pw)),
                        Msg::Connected,
                    );
                }
            }
            Msg::Connected(r) => {
                let ssid = self.connecting.take();
                match r {
                    Ok(s) => ctx.done(s),
                    Err(e) if e.contains("needs a password") => {
                        if let Some(ssid) = ssid {
                            let sec = self
                                .data
                                .as_ref()
                                .and_then(|d| d.access_points.iter().find(|a| a.ssid == ssid))
                                .map(|a| a.security.clone())
                                .unwrap_or_else(|| "WPA2".into());
                            self.ask = Some((ssid, sec, String::new(), false));
                        }
                    }
                    Err(e) => ctx.fail(e),
                }
                return self.read();
            }
            Msg::Disconnect(dev) => {
                return job(
                    move || network::disconnect(&dev),
                    |r| Msg::Done(r, "Disconnected"),
                );
            }
            Msg::Up(uuid) => return job(move || network::up(&uuid), |r| Msg::Done(r, "Connected")),
            Msg::Down(uuid) => {
                return job(
                    move || network::down(&uuid),
                    |r| Msg::Done(r, "Disconnected"),
                );
            }
            Msg::AskForget(name, uuid) => self.forget = Some((name, uuid)),
            Msg::CancelForget => self.forget = None,
            Msg::Forget => {
                if let Some((_, uuid)) = self.forget.take() {
                    return job(
                        move || network::forget(&uuid),
                        |r| Msg::Done(r, "Forgotten"),
                    );
                }
            }
            Msg::Autoconnect(uuid, on) => {
                return job(
                    move || network::set_autoconnect(&uuid, on),
                    |r| Msg::Done(r, ""),
                );
            }
            Msg::Hostname(s) => self.hostname = s,
            Msg::SaveHostname => {
                let name = self.hostname.trim().to_owned();
                return job(
                    move || network::set_hostname(&name),
                    |r| Msg::Done(r, "Computer name changed"),
                );
            }
            Msg::Done(r, ok) => {
                ctx.report(&r, ok);
                return self.read();
            }
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        if let Some(e) = &self.unavailable {
            return ui::note(
                c,
                Icon::CircleAlert,
                c.warn,
                format!(
                    "NetworkManager isn't answering ({e}). Networking needs NetworkManager running: systemctl enable --now NetworkManager"
                ),
            );
        }
        let Some(d) = &self.data else {
            return ui::loading(c);
        };
        let mut body = column![].spacing(26);

        // Connections that are up, with their addresses.
        let mut up: Vec<El<'a, Msg>> = vec![];
        for dev in d
            .devices
            .iter()
            .filter(|d| d.state.starts_with("connected"))
        {
            let (ic, kind) = match dev.kind.as_str() {
                "wifi" => (Icon::Wifi, "Wi-Fi"),
                "ethernet" => (Icon::Globe, "Wired"),
                "wireguard" | "vpn" | "tun" => (Icon::Shield, "VPN"),
                _ => (Icon::Globe, "Network"),
            };
            up.push(ui::icon_setting(
                c,
                ic,
                c.good,
                dev.connection.clone(),
                format!(
                    "{kind} · {} · {}",
                    dev.name,
                    if dev.address.is_empty() {
                        "no address yet"
                    } else {
                        &dev.address
                    }
                ),
                ui::btn(
                    c,
                    "Disconnect",
                    Kind::Secondary,
                    Some(Msg::Disconnect(dev.name.clone())),
                ),
            ));
            if !dev.gateway.is_empty() {
                up.push(ui::mono_fact(c, "Gateway", dev.gateway.clone()));
            }
            if !dev.dns.is_empty() {
                up.push(ui::mono_fact(c, "DNS", dev.dns.join(", ")));
            }
            if !dev.hw.is_empty() {
                up.push(ui::mono_fact(c, "Hardware address", dev.hw.clone()));
            }
        }
        if up.is_empty() {
            up.push(ui::empty(c, "Not connected."));
        }
        body = body.push(ui::section(c, "Connected", up));

        let has_wifi = d.devices.iter().any(|x| x.kind == "wifi");
        let mut wifi = vec![
            ui::icon_setting(
                c,
                if d.wifi_enabled {
                    Icon::Wifi
                } else {
                    Icon::WifiOff
                },
                c.accent,
                "Wi-Fi",
                if has_wifi {
                    ""
                } else {
                    "No Wi-Fi adapter found"
                },
                ui::toggle(d.wifi_enabled, Msg::Wifi),
            ),
            ui::icon_setting(
                c,
                Icon::Zap,
                c.warn,
                "Airplane mode",
                "Turn off Wi-Fi, mobile and Bluetooth",
                ui::toggle(d.airplane, Msg::Airplane),
            ),
        ];
        if d.wifi_enabled {
            wifi.push(ui::setting(
                c,
                format!("{} networks nearby", d.access_points.len()),
                "",
                ui::icon_text_btn(
                    c,
                    Icon::RefreshCw,
                    if self.scanning {
                        "Looking…"
                    } else {
                        "Look again"
                    },
                    Kind::Ghost,
                    (!self.scanning).then_some(Msg::Scan),
                ),
            ));
            for ap in d.access_points.iter().take(30) {
                let strength = match ap.signal {
                    0..=25 => Icon::WifiZero,
                    26..=50 => Icon::WifiLow,
                    51..=75 => Icon::WifiHigh,
                    _ => Icon::Wifi,
                };
                let connecting = self.connecting.as_deref() == Some(ap.ssid.as_str());
                let control: El<'a, Msg> = if ap.in_use {
                    ui::chip(c, "Connected", c.good)
                } else if connecting {
                    ui::chip(c, "Connecting…", c.accent)
                } else {
                    ui::btn(
                        c,
                        "Connect",
                        Kind::Secondary,
                        self.connecting
                            .is_none()
                            .then(|| Msg::Join(ap.ssid.clone(), ap.security.clone(), ap.secured())),
                    )
                };
                let mut name = row![text(ap.ssid.clone()).size(14).color(c.text)]
                    .spacing(8)
                    .align_y(Alignment::Center);
                if ap.secured() {
                    name = name.push(crate::icons::icon(Icon::Lock, 12.0, c.dim));
                }
                wifi.push(
                    row![
                        crate::icons::icon(
                            strength,
                            18.0,
                            if ap.in_use { c.accent } else { c.muted }
                        ),
                        column![
                            name,
                            text(format!(
                                "{}% · {}",
                                ap.signal,
                                if ap.secured() {
                                    ap.security.as_str()
                                } else {
                                    "Open"
                                }
                            ))
                            .size(12)
                            .color(c.muted)
                        ]
                        .spacing(2)
                        .width(Length::Fill),
                        control
                    ]
                    .spacing(14)
                    .align_y(Alignment::Center)
                    .padding([10, 18])
                    .into(),
                );
            }
        }
        body = body.push(ui::section(c, "Wi-Fi", wifi));

        let saved: Vec<El<'a, Msg>> = if d.saved.is_empty() {
            vec![ui::empty(c, "No saved connections.")]
        } else {
            d.saved
                .iter()
                .map(|s| {
                    ui::setting(
                        c,
                        s.name.clone(),
                        format!("{}{}", s.kind, if s.active { " · in use" } else { "" }),
                        row![
                            iced::widget::tooltip(
                                ui::toggle(s.autoconnect, {
                                    let u = s.uuid.clone();
                                    move |on| Msg::Autoconnect(u.clone(), on)
                                }),
                                iced::widget::container(text("Connect by itself").size(12))
                                    .padding([4, 8])
                                    .style(c.toast(c.border)),
                                iced::widget::tooltip::Position::Top,
                            ),
                            if s.active {
                                ui::btn(
                                    c,
                                    "Disconnect",
                                    Kind::Ghost,
                                    Some(Msg::Down(s.uuid.clone())),
                                )
                            } else {
                                ui::btn(c, "Connect", Kind::Ghost, Some(Msg::Up(s.uuid.clone())))
                            },
                            ui::icon_btn(
                                c,
                                Icon::Trash2,
                                Kind::Ghost,
                                Some(Msg::AskForget(s.name.clone(), s.uuid.clone()))
                            ),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                })
                .collect()
        };
        body = body.push(ui::section(c, "Saved connections", saved));

        let host_ok = network::valid_hostname(self.hostname.trim());
        body = body.push(ui::section(
            c,
            "This computer",
            vec![ui::setting(
                c,
                "Name on the network",
                "How other devices see this computer",
                row![
                    ui::input(c, "hostname", &self.hostname)
                        .on_input(Msg::Hostname)
                        .on_submit(Msg::SaveHostname)
                        .width(200),
                    ui::btn(
                        c,
                        "Rename",
                        Kind::Secondary,
                        (host_ok && self.hostname.trim() != d.hostname)
                            .then_some(Msg::SaveHostname)
                    ),
                ]
                .spacing(8),
            )],
        ));
        if !sys::has("nm-connection-editor") {
            body = body.push(
                text("VPNs and enterprise Wi-Fi: set them up with nmcli or nm-connection-editor; they show up here once saved.")
                    .size(12)
                    .color(c.dim),
            );
        }
        body.into()
    }

    fn dialog<'a>(&'a self, ctx: &'a Ctx) -> Option<El<'a, Msg>> {
        let c = ctx.c;
        if let Some((name, _)) = &self.forget {
            return Some(
                column![
                    text(format!("Forget {name}?"))
                        .size(18)
                        .font(BOLD)
                        .color(c.text),
                    text("Its password and settings are removed. You can join it again later.")
                        .size(13)
                        .color(c.muted),
                    row![
                        ui::fill(),
                        ui::btn(c, "Cancel", Kind::Secondary, Some(Msg::CancelForget)),
                        ui::btn(c, "Forget", Kind::Danger, Some(Msg::Forget)),
                    ]
                    .spacing(8)
                ]
                .spacing(14)
                .into(),
            );
        }
        let (ssid, _, pw, shown) = self.ask.as_ref()?;
        Some(
            column![
                row![
                    crate::icons::icon(Icon::Lock, 20.0, c.accent),
                    text(format!("Join {ssid}"))
                        .size(18)
                        .font(BOLD)
                        .color(c.text)
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                text("This network needs a password.")
                    .size(13)
                    .color(c.muted),
                row![
                    ui::input(c, "Password", pw)
                        .secure(!shown)
                        .on_input(Msg::Password)
                        .on_submit(Msg::SubmitPassword)
                        .width(Length::Fill),
                    ui::icon_btn(
                        c,
                        if *shown { Icon::EyeOff } else { Icon::Eye },
                        Kind::Secondary,
                        Some(Msg::ShowPassword)
                    ),
                ]
                .spacing(8),
                row![
                    ui::fill(),
                    ui::btn(c, "Cancel", Kind::Secondary, Some(Msg::CancelPassword)),
                    ui::btn(
                        c,
                        "Join",
                        Kind::Primary,
                        (!pw.is_empty()).then_some(Msg::SubmitPassword)
                    ),
                ]
                .spacing(8)
            ]
            .spacing(14)
            .into(),
        )
    }
}
