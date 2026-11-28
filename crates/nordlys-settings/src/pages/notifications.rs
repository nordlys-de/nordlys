use super::{PageState, Reads, toml_set};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{job, notify};
use crate::theme::{Kind, SEMIBOLD};
use crate::ui::{self, El};
use iced::Task;
use iced::widget::{column, row, text};
use nordlys_config::{Notifications, ToastPosition};

#[derive(Default)]
pub struct State {
    data: Option<notify::Notices>,
    reads: Reads,
    /// The duration while its slider is being dragged.
    seconds: Option<f32>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<notify::Notices, String>),
    Dnd(bool),
    ShowLast,
    Clear,
    Test,
    Position(ToastPosition),
    Seconds(f32),
    SecondsDone,
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    Left,
    Centre,
    Right,
}

fn side(p: ToastPosition) -> Side {
    match p {
        ToastPosition::TopLeft | ToastPosition::BottomLeft => Side::Left,
        ToastPosition::TopCenter | ToastPosition::BottomCenter => Side::Centre,
        ToastPosition::TopRight | ToastPosition::BottomRight => Side::Right,
    }
}

fn position(bottom: bool, side: Side) -> ToastPosition {
    ToastPosition::ALL
        .into_iter()
        .find(|p| p.bottom() == bottom && self::side(*p) == side)
        .unwrap_or_default()
}

impl State {
    /// Where toasts show and for how long. These come from nordlys.toml, so
    /// they work whether or not the daemon answers.
    fn toasts<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let cfg: &Notifications = &ctx.toml.config.notifications;
        let at = cfg.position;
        let seconds = self.seconds.unwrap_or(f32::from(cfg.seconds));
        ui::section(
            c,
            "Toasts",
            vec![
                ui::setting(
                    c,
                    "Edge",
                    "The edge of the screen toasts appear at",
                    ui::segmented(
                        c,
                        &[(false, "Top"), (true, "Bottom")],
                        at.bottom(),
                        move |b| Msg::Position(position(b, side(at))),
                    ),
                ),
                ui::setting(
                    c,
                    "Side",
                    "Where along that edge",
                    ui::segmented(
                        c,
                        &[
                            (Side::Left, "Left"),
                            (Side::Centre, "Centre"),
                            (Side::Right, "Right"),
                        ],
                        side(at),
                        move |s| Msg::Position(position(at.bottom(), s)),
                    ),
                ),
                ui::slider_row(
                    c,
                    "Stay on screen",
                    "When the app doesn't say how long. Critical ones stay until dismissed",
                    1.0..=60.0,
                    1.0,
                    seconds,
                    format!("{seconds:.0} s"),
                    Msg::Seconds,
                    Some(Msg::SecondsDone),
                ),
            ],
        )
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
            Msg::ShowLast => job(notify::show_last, |r| {
                Msg::Done(r, "Showing the last notification")
            }),
            Msg::Clear => job(notify::clear, |r| Msg::Done(r, "Notifications cleared")),
            Msg::Test => job(notify::send_test, |r| {
                Msg::Done(r, "Test notification sent")
            }),
            Msg::Position(p) => {
                toml_set(ctx, "notifications", "position", p.key());
                Task::none()
            }
            Msg::Seconds(v) => {
                self.seconds = Some(v);
                Task::none()
            }
            Msg::SecondsDone => {
                if let Some(v) = self.seconds.take() {
                    toml_set(ctx, "notifications", "seconds", v.round() as i64);
                }
                Task::none()
            }
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
        body.push(self.toasts(ctx))
            .push(ui::section(c, "Recent", recent))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_and_side_name_every_position() {
        for p in ToastPosition::ALL {
            assert_eq!(position(p.bottom(), side(p)), p);
        }
    }
}
