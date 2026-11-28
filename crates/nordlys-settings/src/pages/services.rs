use super::{PageState, Reads};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{job, services};
use crate::theme::{BOLD, Kind, MONO};
use crate::ui::{self, El};
use iced::widget::{column, container, row, scrollable, text};
use iced::{Alignment, Length, Task};

#[derive(Default)]
pub struct State {
    data: Option<services::Services>,
    reads: Reads,
    busy: Option<String>,
    /// Unit, user or system, and its last lines.
    log: Option<(String, bool, Result<String, String>)>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<services::Services, String>),
    Control(bool, &'static str, String),
    AtLogin(String, bool),
    Log(bool, String),
    Logged(String, bool, Result<String, String>),
    CloseLog,
    Done(Result<(), String>, String),
}

impl State {
    fn read(&mut self) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(services::read, Msg::Read)
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
                    Ok(d) => self.data = Some(d),
                    Err(e) => ctx.fail(e),
                }
                Task::none()
            }
            Msg::Control(user, verb, unit) => {
                self.busy = Some(unit.clone());
                let ok = match verb {
                    "restart" => format!("Restarted {unit}"),
                    "start" => format!("Started {unit}"),
                    "stop" => format!("Stopped {unit}"),
                    "reset-failed" => format!("Cleared {unit}"),
                    _ => String::new(),
                };
                job(
                    move || services::control(user, verb, &unit),
                    move |r| Msg::Done(r, ok.clone()),
                )
            }
            Msg::AtLogin(unit, on) => {
                self.busy = Some(unit.clone());
                let (verb, ok) = if on {
                    ("unmask", format!("{unit} starts at login again"))
                } else {
                    ("mask", format!("{unit} stays off from your next login"))
                };
                job(
                    move || services::control(true, verb, &unit),
                    move |r| Msg::Done(r, ok.clone()),
                )
            }
            Msg::Log(user, unit) => {
                let u = unit.clone();
                job(
                    move || services::log(user, &u),
                    move |r| Msg::Logged(unit.clone(), user, r),
                )
            }
            Msg::Logged(unit, user, r) => {
                self.log = Some((unit, user, r));
                Task::none()
            }
            Msg::CloseLog => {
                self.log = None;
                Task::none()
            }
            Msg::Done(r, ok) => {
                self.busy = None;
                ctx.report(&r, &ok);
                self.read()
            }
        }
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let Some(d) = &self.data else {
            return ui::loading(c);
        };
        if !d.systemd {
            return ui::note(
                c,
                Icon::Info,
                c.accent,
                "This system doesn't run systemd, so the session's pieces can't be managed from here.",
            );
        }
        let unit_row =
            |u: &'a services::Unit, user: bool, title: String, detail: String| -> El<'a, Msg> {
                let name = u.name.clone();
                let busy = self.busy.as_deref() == Some(u.name.as_str());
                let tint = if u.failed() {
                    c.bad
                } else if u.running() {
                    c.good
                } else {
                    c.dim
                };
                let state = if u.failed() {
                    "Failed".to_owned()
                } else if u.running() {
                    "Running".to_owned()
                } else {
                    format!("{} ({})", u.active, u.sub)
                };
                // Targets and scopes (apps already running) aren't started by hand.
                let target = u.name.ends_with(".target") || u.name.ends_with(".scope");
                let actions: El<'a, Msg> = if busy {
                    ui::chip(c, "Working…", c.accent)
                } else {
                    let mut r = row![ui::chip(c, state, tint)]
                        .spacing(6)
                        .align_y(Alignment::Center);
                    r = r.push(ui::icon_btn(
                        c,
                        Icon::FileText,
                        Kind::Ghost,
                        Some(Msg::Log(user, name.clone())),
                    ));
                    if u.failed() {
                        r = r.push(ui::btn(
                            c,
                            "Clear",
                            Kind::Ghost,
                            Some(Msg::Control(user, "reset-failed", name.clone())),
                        ));
                    }
                    if user && u.optional() {
                        let unit = name.clone();
                        r = r
                            .push(text("At login").size(12).color(c.muted))
                            .push(ui::toggle(!u.masked, move |on| {
                                Msg::AtLogin(unit.clone(), on)
                            }));
                    }
                    if !target {
                        if u.running() {
                            r = r
                                .push(ui::icon_btn(
                                    c,
                                    Icon::RotateCcw,
                                    Kind::Ghost,
                                    Some(Msg::Control(user, "restart", name.clone())),
                                ))
                                .push(ui::icon_btn(
                                    c,
                                    Icon::Square,
                                    Kind::Ghost,
                                    Some(Msg::Control(user, "stop", name.clone())),
                                ));
                        } else if !u.masked {
                            r = r.push(ui::icon_btn(
                                c,
                                Icon::Play,
                                Kind::Ghost,
                                Some(Msg::Control(user, "start", name.clone())),
                            ));
                        }
                    }
                    r.into()
                };
                ui::setting(c, title, detail, actions)
            };
        let session: Vec<El<'a, Msg>> = if d.session.is_empty() {
            vec![ui::empty(
                c,
                "No Nordlys services are installed for this user.",
            )]
        } else {
            d.session
                .iter()
                .map(|u| {
                    let p = services::purpose(&u.name);
                    unit_row(
                        u,
                        true,
                        u.short(),
                        if p.is_empty() {
                            u.description.clone()
                        } else {
                            p.to_owned()
                        },
                    )
                })
                .collect()
        };
        let failed: Vec<El<'a, Msg>> = d
            .failed_user
            .iter()
            .map(|u| {
                unit_row(
                    u,
                    true,
                    unescape(&u.name),
                    format!("Yours · {}", unescape(&u.description)),
                )
            })
            .chain(d.failed_system.iter().map(|u| {
                unit_row(
                    u,
                    false,
                    unescape(&u.name),
                    format!("System · {}", unescape(&u.description)),
                )
            }))
            .collect();
        let mut body = column![
            ui::section(c, "Desktop session", session),
            ui::note(
                c,
                Icon::Info,
                c.accent,
                "At login off keeps a service from starting next time; what's running now keeps running. \
                 To start it by hand, turn At login back on first.",
            ),
        ]
        .spacing(26);
        body = body.push(ui::section(
            c,
            "Failed",
            if failed.is_empty() {
                vec![ui::icon_setting(
                    c,
                    Icon::CircleCheck,
                    c.good,
                    "Nothing has failed",
                    "Every service on this computer started fine",
                    ui::gap(0.0),
                )]
            } else {
                failed
            },
        ));
        body.push(
            text("Starting or stopping system services asks for your password.")
                .size(12)
                .color(c.dim),
        )
        .into()
    }

    fn dialog<'a>(&'a self, ctx: &'a Ctx) -> Option<El<'a, Msg>> {
        let c = ctx.c;
        let (unit, _, log) = self.log.as_ref()?;
        let body: El<'a, Msg> = match log {
            Ok(s) if s.trim().is_empty() || s.contains("-- No entries --") => {
                text("Nothing logged yet.").size(13).color(c.muted).into()
            }
            Ok(s) => scrollable(
                container(text(s.clone()).size(11).font(MONO).color(c.text))
                    .padding(12)
                    .width(Length::Fill),
            )
            .anchor_bottom()
            .height(360)
            .into(),
            Err(e) => text(e.clone()).size(13).color(c.bad).into(),
        };
        Some(
            column![
                row![
                    text(unit.clone()).size(16).font(BOLD).color(c.text),
                    ui::fill(),
                    ui::icon_btn(c, Icon::X, Kind::Ghost, Some(Msg::CloseLog)),
                ]
                .align_y(Alignment::Center),
                container(body).style(c.note(c.border)).width(Length::Fill),
            ]
            .spacing(12)
            .width(640)
            .into(),
        )
    }
}

/// systemd's `\x2d` escapes back to the characters they stand for.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\x") {
        out.push_str(&rest[..i]);
        let hex = rest
            .get(i + 2..i + 4)
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(b) if b.is_ascii() => {
                out.push(b as char);
                rest = &rest[i + 4..];
            }
            _ => {
                out.push_str("\\x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn unescapes_unit_names() {
        assert_eq!(
            super::unescape(r"app-arch\x2dupdate\x2dtray@autostart.service"),
            "app-arch-update-tray@autostart.service"
        );
        assert_eq!(super::unescape(r"odd\xZZ"), r"odd\xZZ");
    }
}
