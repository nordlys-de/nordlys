use super::{PageState, Reads};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{job, notify};
use crate::theme::{Kind, SEMIBOLD};
use crate::ui::{self, El};
use iced::Task;
use iced::widget::{column, row, text};

#[derive(Default)]
pub struct State {
    data: Option<notify::Notices>,
    reads: Reads,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<notify::Notices, String>),
    Dnd(bool),
    ShowLast,
    Clear,
    Test,
    Done(Result<(), String>, &'static str),
}

impl State {
    fn read(&mut self) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(notify::read, Msg::Read)
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
            Msg::Dnd(on) => {
                if let Some(d) = self.data.as_mut() {
                    d.dnd = on;
                }
                job(
                    move || notify::set_dnd(on),
                    move |r| {
                        Msg::Done(
                            r,
                            if on {
                                "Do not disturb is on"
                            } else {
                                "Do not disturb is off"
                            },
                        )
                    },
                )
            }
            Msg::ShowLast => job(notify::show_last, |r| Msg::Done(r, "")),
            Msg::Clear => job(notify::clear, |r| Msg::Done(r, "Notifications cleared")),
            Msg::Test => job(notify::send_test, |r| Msg::Done(r, "")),
            Msg::Done(r, ok) => {
                ctx.report(&r, ok);
                self.read()
            }
        }
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let Some(d) = &self.data else {
            return ui::loading(c);
        };
        let mut body = column![].spacing(26);
        let daemon = match d.daemon.as_deref() {
            None => {
                body = body.push(ui::note(
                    c,
                    Icon::CircleAlert,
                    c.warn,
                    "No notification daemon is running. The Nordlys session starts one (nordlys-shell notify).",
                ));
                "None".to_owned()
            }
            Some("Nordlys") => "Nordlys".to_owned(),
            Some(other) => {
                body = body.push(ui::note(
                    c,
                    Icon::Info,
                    c.accent,
                    format!("{other} is showing notifications, not Nordlys. Stop it to use Nordlys toasts and history."),
                ));
                other.to_owned()
            }
        };
        let can = d.controllable;
        body = body.push(ui::section(
            c,
            "",
            vec![
                ui::icon_setting(
                    c,
                    if d.dnd { Icon::BellOff } else { Icon::Bell },
                    if d.dnd { c.warn } else { c.accent },
                    "Do not disturb",
                    "Toasts wait quietly in history; critical ones still show",
                    ui::toggle_maybe(d.dnd, can.then_some(Msg::Dnd)),
                ),
                ui::fact(c, "Shown by", daemon),
                ui::fact(c, "Waiting", d.waiting.to_string()),
                ui::fact(c, "In history", d.history.to_string()),
                ui::setting(
                    c,
                    "History",
                    "Show the last one again, or clear them all",
                    row![
                        ui::btn(
                            c,
                            "Show last",
                            Kind::Secondary,
                            (can && d.history > 0).then_some(Msg::ShowLast)
                        ),
                        ui::btn(
                            c,
                            "Clear",
                            Kind::Secondary,
                            (can && d.history > 0).then_some(Msg::Clear)
                        ),
                    ]
                    .spacing(8),
                ),
                ui::setting(
                    c,
                    "Try it",
                    "Send a notification to see how they look",
                    ui::btn(c, "Send a test", Kind::Secondary, Some(Msg::Test)),
                ),
            ],
        ));
        let recent: Vec<El<'a, Msg>> = if d.recent.is_empty() {
            vec![ui::empty(c, "Nothing yet.")]
        } else {
            d.recent
                .iter()
                .take(12)
                .map(|n| {
                    column![
                        row![text(n.app.clone()).size(11).font(SEMIBOLD).color(c.dim),],
                        text(n.summary.clone()).size(14).color(c.text),
                        text(n.body.lines().next().unwrap_or("").to_owned())
                            .size(12)
                            .color(c.muted),
                    ]
                    .spacing(2)
                    .padding([12, 18])
                    .into()
                })
                .collect()
        };
        body.push(ui::section(c, "Recent", recent)).into()
    }
}
