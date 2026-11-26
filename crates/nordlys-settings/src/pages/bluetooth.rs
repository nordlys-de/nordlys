use super::{PageState, Reads};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{bluetooth, job};
use crate::theme::{BOLD, Kind};
use crate::ui::{self, El};
use iced::widget::{column, row, text};
use iced::{Alignment, Task};

#[derive(Default)]
pub struct State {
    data: Option<bluetooth::Bluetooth>,
    reads: Reads,
    scanning: bool,
    /// The device something is being done to.
    busy: Option<String>,
    forget: Option<(String, String)>,
    failed: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<bluetooth::Bluetooth, String>),
    Power(bool),
    Visible(bool),
    Scan,
    Scanned(Result<(), String>),
    Connect(String),
    Disconnect(String),
    Pair(String),
    Trust(String, bool),
    AskForget(String, String),
    Forget,
    CancelForget,
    Done(Result<(), String>, String),
}

impl State {
    fn read(&mut self) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(bluetooth::read, Msg::Read)
    }

    fn act(
        &mut self,
        address: String,
        ok: String,
        work: impl FnOnce(&str) -> Result<(), String> + Send + 'static,
    ) -> Task<Msg> {
        self.busy = Some(address.clone());
        job(move || work(&address), move |r| Msg::Done(r, ok))
    }
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        self.read()
    }

    fn tick(&mut self, _: &mut Ctx) -> Task<Msg> {
        self.read()
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Read(r) => {
                self.reads.done();
                match r {
                    Ok(d) => {
                        self.data = Some(d);
                        self.failed = None;
                    }
                    Err(e) => self.failed = Some(e),
                }
                Task::none()
            }
            Msg::Power(on) => {
                if let Some(d) = self.data.as_mut() {
                    d.powered = on;
                }
                job(
                    move || bluetooth::set_power(on),
                    |r| Msg::Done(r, String::new()),
                )
            }
            Msg::Visible(on) => job(
                move || bluetooth::set_discoverable(on),
                move |r| {
                    Msg::Done(
                        r,
                        if on {
                            "Visible to nearby devices for 3 minutes".into()
                        } else {
                            String::new()
                        },
                    )
                },
            ),
            Msg::Scan => {
                self.scanning = true;
                job(|| bluetooth::scan(8), Msg::Scanned)
            }
            Msg::Scanned(r) => {
                self.scanning = false;
                if let Err(e) = r {
                    ctx.fail(e);
                }
                self.read()
            }
            Msg::Connect(a) => self.act(a, "Connected".into(), bluetooth::connect),
            Msg::Disconnect(a) => self.act(a, "Disconnected".into(), bluetooth::disconnect),
            Msg::Pair(a) => self.act(a, "Paired and connected".into(), bluetooth::pair),
            Msg::Trust(a, on) => self.act(a, String::new(), move |a| bluetooth::set_trusted(a, on)),
            Msg::AskForget(name, a) => {
                self.forget = Some((name, a));
                Task::none()
            }
            Msg::CancelForget => {
                self.forget = None;
                Task::none()
            }
            Msg::Forget => match self.forget.take() {
                Some((name, a)) => self.act(a, format!("Forgot {name}"), bluetooth::remove),
                None => Task::none(),
            },
            Msg::Done(r, ok) => {
                self.busy = None;
                ctx.report(&r, &ok);
                self.read()
            }
        }
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        if let Some(e) = &self.failed {
            return ui::note(
                c,
                Icon::CircleAlert,
                c.warn,
                format!("Bluetooth isn't answering: {e}"),
            );
        }
        let Some(d) = &self.data else {
            return ui::loading(c);
        };
        if !d.available {
            return ui::note(
                c,
                Icon::BluetoothOff,
                c.muted,
                "No Bluetooth adapter here, or bluetoothd isn't running (systemctl enable --now bluetooth).",
            );
        }
        let mut body = column![ui::section(
            c,
            "",
            vec![
                ui::icon_setting(
                    c,
                    if d.powered {
                        Icon::Bluetooth
                    } else {
                        Icon::BluetoothOff
                    },
                    c.accent,
                    "Bluetooth",
                    format!(
                        "This computer is {}",
                        if d.name.is_empty() {
                            "unnamed"
                        } else {
                            &d.name
                        }
                    ),
                    ui::toggle(d.powered, Msg::Power),
                ),
                ui::toggle_row(
                    c,
                    "Visible to others",
                    "Let nearby phones and computers find this one",
                    d.discoverable,
                    Msg::Visible,
                ),
            ],
        )]
        .spacing(26);
        if !d.powered {
            return body.into();
        }
        let row_for = |dev: &'a bluetooth::Device| -> El<'a, Msg> {
            let busy = self.busy.as_deref() == Some(dev.address.as_str());
            let ic = match dev.kind() {
                "Headphones" | "Speaker" => Icon::Volume2,
                "Keyboard" => Icon::Keyboard,
                "Mouse" => Icon::Mouse,
                "Computer" => Icon::Monitor,
                _ => Icon::Bluetooth,
            };
            let mut detail = dev.kind().to_owned();
            if dev.connected {
                detail.push_str(" · Connected");
            }
            if let Some(b) = dev.battery {
                detail.push_str(&format!(" · Battery {b}%"));
            }
            let a = dev.address.clone();
            let actions: El<'a, Msg> = if busy {
                ui::chip(c, "Working…", c.accent)
            } else if !dev.paired {
                ui::btn(c, "Pair", Kind::Primary, Some(Msg::Pair(a)))
            } else {
                row![
                    if dev.connected {
                        ui::btn(
                            c,
                            "Disconnect",
                            Kind::Secondary,
                            Some(Msg::Disconnect(a.clone())),
                        )
                    } else {
                        ui::btn(c, "Connect", Kind::Secondary, Some(Msg::Connect(a.clone())))
                    },
                    ui::btn(
                        c,
                        if dev.trusted { "Trusted" } else { "Trust" },
                        Kind::Ghost,
                        Some(Msg::Trust(a.clone(), !dev.trusted))
                    ),
                    ui::icon_btn(
                        c,
                        Icon::Trash2,
                        Kind::Ghost,
                        Some(Msg::AskForget(dev.name.clone(), a))
                    ),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into()
            };
            ui::icon_setting(
                c,
                ic,
                if dev.connected { c.good } else { c.muted },
                dev.name.clone(),
                detail,
                actions,
            )
        };
        let paired: Vec<El<'a, Msg>> = d.devices.iter().filter(|x| x.paired).map(row_for).collect();
        let nearby: Vec<El<'a, Msg>> = d
            .devices
            .iter()
            .filter(|x| !x.paired)
            .map(row_for)
            .collect();
        body = body.push(ui::section(
            c,
            "My devices",
            if paired.is_empty() {
                vec![ui::empty(c, "Nothing paired yet.")]
            } else {
                paired
            },
        ));
        let mut near = vec![ui::setting(
            c,
            "Find devices",
            "Put the device in pairing mode, then look",
            ui::icon_text_btn(
                c,
                Icon::ScanSearch,
                if self.scanning {
                    "Looking…"
                } else {
                    "Look for devices"
                },
                Kind::Secondary,
                (!self.scanning).then_some(Msg::Scan),
            ),
        )];
        near.extend(nearby);
        body.push(ui::section(c, "Nearby", near)).into()
    }

    fn dialog<'a>(&'a self, ctx: &'a Ctx) -> Option<El<'a, Msg>> {
        let c = ctx.c;
        let (name, _) = self.forget.as_ref()?;
        Some(
            column![
                text(format!("Forget {name}?"))
                    .size(18)
                    .font(BOLD)
                    .color(c.text),
                text("You'll need to pair it again to use it.")
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
        )
    }
}
