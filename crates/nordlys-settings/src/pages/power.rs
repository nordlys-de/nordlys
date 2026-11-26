use super::{PageState, Reads};
use crate::app::{Ctx, Page};
use crate::icons::Icon;
use crate::sys::{job, power};
use crate::theme::{BOLD, Kind};
use crate::ui::{self, El};
use iced::widget::{column, row, text};
use iced::{Alignment, Task};

#[derive(Default)]
pub struct State {
    data: Option<power::Power>,
    reads: Reads,
    confirm: Option<power::Action>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<power::Power, String>),
    Profile(String),
    Act(power::Action),
    Confirm,
    Cancel,
    Done(Result<(), String>),
}

impl State {
    fn read(&mut self) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(power::read, Msg::Read)
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
            Msg::Profile(p) => {
                if let Some(d) = self.data.as_mut() {
                    d.profile = Some(p.clone());
                }
                job(move || power::set_profile(&p), Msg::Done)
            }
            Msg::Act(a) if a.needs_confirming() => {
                self.confirm = Some(a);
                Task::none()
            }
            Msg::Act(a) => job(move || power::act(a), Msg::Done),
            Msg::Confirm => match self.confirm.take() {
                Some(a) => job(move || power::act(a), Msg::Done),
                None => Task::none(),
            },
            Msg::Cancel => {
                self.confirm = None;
                Task::none()
            }
            Msg::Done(r) => {
                ctx.report(&r, "");
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
        for b in &d.batteries {
            let tint = match b.percent {
                0..=15 => c.bad,
                16..=30 => c.warn,
                _ => c.good,
            };
            let state = match (b.status.as_str(), b.hours_left) {
                ("Charging", Some(h)) => format!("Charging · full in {}", hours(h)),
                ("Charging", None) => "Charging".into(),
                ("Discharging", Some(h)) => format!("On battery · {} left", hours(h)),
                ("Full", _) => "Fully charged".into(),
                ("Not charging", _) => "Plugged in, not charging".into(),
                (s, _) => s.to_owned(),
            };
            let mut rows = vec![ui::icon_setting(
                c,
                if b.status == "Charging" {
                    Icon::BatteryCharging
                } else {
                    Icon::BatteryFull
                },
                tint,
                format!("{}%", b.percent),
                state,
                iced::widget::container(ui::meter(c, b.percent as f32 / 100.0, tint)).width(220),
            )];
            if let Some(w) = b.watts {
                rows.push(ui::fact(
                    c,
                    if b.status == "Charging" {
                        "Charging at"
                    } else {
                        "Using"
                    },
                    format!("{w:.1} W"),
                ));
            }
            if let Some(h) = b.health {
                rows.push(ui::fact(
                    c,
                    "Health",
                    format!("{h}% of its original capacity"),
                ));
            }
            if let Some(n) = b.cycles {
                rows.push(ui::fact(c, "Charge cycles", n.to_string()));
            }
            if !b.model.is_empty() {
                rows.push(ui::fact(c, "Battery", b.model.clone()));
            }
            body = body.push(ui::section(
                c,
                if d.batteries.len() > 1 {
                    &b.name
                } else {
                    "Battery"
                },
                rows,
            ));
        }
        if d.batteries.is_empty() {
            body = body.push(ui::section(
                c,
                "Battery",
                vec![ui::fact(
                    c,
                    "Power",
                    if d.on_ac == Some(false) {
                        "Unknown"
                    } else {
                        "Mains, no battery"
                    },
                )],
            ));
        }
        if !d.profiles.is_empty() {
            let current = d.profile.clone().unwrap_or_default();
            let opts: Vec<(&str, &str)> = d
                .profiles
                .iter()
                .map(|p| {
                    (
                        p.as_str(),
                        match p.as_str() {
                            "power-saver" => "Power saver",
                            "balanced" => "Balanced",
                            "performance" => "Performance",
                            other => other,
                        },
                    )
                })
                .collect();
            let selected = d
                .profiles
                .iter()
                .find(|p| **p == current)
                .map(String::as_str)
                .unwrap_or("");
            body = body.push(ui::section(
                c,
                "Power mode",
                vec![ui::icon_setting(
                    c,
                    match current.as_str() {
                        "power-saver" => Icon::Leaf,
                        "performance" => Icon::Gauge,
                        _ => Icon::Zap,
                    },
                    c.accent,
                    "Mode",
                    match current.as_str() {
                        "power-saver" => "Longer battery life, a bit slower",
                        "performance" => "Fastest, uses more power and runs warmer",
                        _ => "Speed when it's needed, saving power the rest of the time",
                    },
                    ui::segmented(c, &opts, selected, |p: &str| Msg::Profile(p.to_owned())),
                )],
            ));
        } else {
            body = body.push(ui::note(
                c,
                Icon::Info,
                c.accent,
                "Install power-profiles-daemon (or tuned-ppd) to switch between power saver, balanced and performance.",
            ));
        }
        let action = |a: power::Action, ic: Icon, kind: Kind| {
            ui::icon_text_btn(c, ic, a.label(), kind, Some(Msg::Act(a)))
        };
        let mut actions = row![
            action(power::Action::Lock, Icon::Lock, Kind::Secondary),
            action(power::Action::Suspend, Icon::Moon, Kind::Secondary),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if d.can_hibernate {
            actions = actions.push(action(
                power::Action::Hibernate,
                Icon::HardDrive,
                Kind::Secondary,
            ));
        }
        actions = actions
            .push(action(power::Action::LogOut, Icon::LogOut, Kind::Secondary))
            .push(action(
                power::Action::Restart,
                Icon::RotateCcw,
                Kind::Secondary,
            ))
            .push(action(power::Action::ShutDown, Icon::Power, Kind::Danger));
        body = body.push(ui::section(
            c,
            "Session",
            vec![
                iced::widget::container(actions.wrap().vertical_spacing(8))
                    .padding([16, 18])
                    .into(),
            ],
        ));
        body.push(
            text(format!(
                "When the machine sleeps on its own is under {}.",
                Page::LockIdle.title()
            ))
            .size(12)
            .color(c.dim),
        )
        .into()
    }

    fn dialog<'a>(&'a self, ctx: &'a Ctx) -> Option<El<'a, Msg>> {
        let c = ctx.c;
        let a = self.confirm?;
        let what = match a {
            power::Action::LogOut => "Every app closes and you go back to the login screen.",
            power::Action::Restart => "Every app closes and the computer starts again.",
            _ => "Every app closes and the computer turns off.",
        };
        Some(
            column![
                text(format!("{}?", a.label()))
                    .size(18)
                    .font(BOLD)
                    .color(c.text),
                text(format!("{what} Unsaved work is lost."))
                    .size(13)
                    .color(c.muted),
                row![
                    ui::fill(),
                    ui::btn(c, "Cancel", Kind::Secondary, Some(Msg::Cancel)),
                    ui::btn(c, a.label(), Kind::Danger, Some(Msg::Confirm)),
                ]
                .spacing(8)
            ]
            .spacing(14)
            .into(),
        )
    }
}

fn hours(h: f32) -> String {
    let m = (h * 60.0).round() as u32;
    if m < 60 {
        format!("{m} min")
    } else {
        format!("{} h {:02} min", m / 60, m % 60)
    }
}
