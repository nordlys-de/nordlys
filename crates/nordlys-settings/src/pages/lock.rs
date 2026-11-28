use super::PageState;
use crate::app::Ctx;
use crate::icons::Icon;
use crate::store::idle::{self, Idle};
use crate::sys::{self, job, power};
use crate::theme::Kind;
use crate::ui::{self, El};
use iced::Task;
use iced::widget::{column, pick_list, row};

#[derive(Default)]
pub struct State {
    idle: Option<Idle>,
    dim_level: Option<f32>,
    pam: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Dim,
    Lock,
    ScreenOff,
    Suspend,
}

#[derive(Debug, Clone)]
pub enum Msg {
    After(Step, Choice),
    DimLevel(f32),
    DimLevelDone,
    LockBeforeSleep(bool),
    Saved(Result<(), String>),
    LockNow,
    Locked(Result<(), String>),
}

/// A time to pick from the list, or never.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice(pub Option<u32>);

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&ui::minutes(self.0))
    }
}

const CHOICES: [Option<u32>; 12] = [
    Some(60),
    Some(120),
    Some(180),
    Some(240),
    Some(300),
    Some(360),
    Some(600),
    Some(900),
    Some(1800),
    Some(3600),
    Some(7200),
    None,
];

fn choices(current: Option<u32>) -> Vec<Choice> {
    let mut v: Vec<Choice> = CHOICES.iter().map(|c| Choice(*c)).collect();
    if !v.contains(&Choice(current)) {
        v.insert(0, Choice(current));
        v.sort_by_key(|c| c.0.unwrap_or(u32::MAX));
    }
    v
}

impl State {
    fn save(&mut self, ctx: &mut Ctx, change: impl FnOnce(&mut Idle)) -> Task<Msg> {
        let Some(i) = self.idle.as_mut() else {
            return Task::none();
        };
        change(i);
        // Keep the steps in order: dim, lock, screen off, suspend.
        if let (Some(l), Some(o)) = (i.lock, i.screen_off)
            && o < l
        {
            ctx.done("The screen will turn off before it locks");
        }
        let i = i.clone();
        job(move || idle::save(&i), Msg::Saved)
    }
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        self.idle = Some(idle::load());
        self.pam = Some(std::path::Path::new("/etc/pam.d/nordlys").exists());
        Task::none()
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::After(step, Choice(t)) => self.save(ctx, |i| match step {
                Step::Dim => i.dim = t,
                Step::Lock => i.lock = t,
                Step::ScreenOff => i.screen_off = t,
                Step::Suspend => i.suspend = t,
            }),
            Msg::DimLevel(v) => {
                self.dim_level = Some(v);
                Task::none()
            }
            Msg::DimLevelDone => match self.dim_level.take() {
                Some(v) => self.save(ctx, |i| i.dim_level = v.round() as u8),
                None => Task::none(),
            },
            Msg::LockBeforeSleep(on) => self.save(ctx, |i| i.lock_before_sleep = on),
            Msg::Saved(r) => {
                ctx.report(&r, "Idle settings saved");
                Task::none()
            }
            Msg::LockNow => job(|| power::act(power::Action::Lock), Msg::Locked),
            Msg::Locked(r) => {
                ctx.report(&r, "");
                Task::none()
            }
        }
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let Some(i) = &self.idle else {
            return ui::loading(c);
        };
        let pick = |step: Step, value: Option<u32>| {
            pick_list(choices(value), Some(Choice(value)), move |ch| {
                Msg::After(step, ch)
            })
            .padding([6, 12])
            .text_size(13)
        };
        let level = self.dim_level.unwrap_or(i.dim_level as f32);
        let mut steps = vec![ui::icon_setting(
            c,
            Icon::Sun,
            c.warn,
            "Dim the screen",
            "Turn the backlight down first",
            pick(Step::Dim, i.dim),
        )];
        if i.dim.is_some() {
            steps.push(ui::slider_row(
                c,
                "Dimmed brightness",
                "How far down it goes",
                5.0..=90.0,
                5.0,
                level,
                format!("{level:.0}%"),
                Msg::DimLevel,
                Some(Msg::DimLevelDone),
            ));
        }
        steps.extend([
            ui::icon_setting(
                c,
                Icon::Lock,
                c.accent,
                "Lock",
                "Ask for your password to come back",
                pick(Step::Lock, i.lock),
            ),
            ui::icon_setting(
                c,
                Icon::Monitor,
                c.muted,
                "Turn the screen off",
                "The monitors go dark; the computer keeps running",
                pick(Step::ScreenOff, i.screen_off),
            ),
            ui::icon_setting(
                c,
                Icon::Moon,
                c.accent,
                "Suspend",
                "Sleep to save power",
                pick(Step::Suspend, i.suspend),
            ),
        ]);

        let mut body = column![
            ui::section(c, "When you're away", steps),
            ui::section(
                c,
                "Lock screen",
                vec![
                    ui::toggle_row(
                        c,
                        "Lock before sleeping",
                        "So the machine wakes up locked",
                        i.lock_before_sleep,
                        Msg::LockBeforeSleep
                    ),
                    ui::setting(
                        c,
                        "Lock now",
                        "Super Esc or Super L does the same",
                        ui::icon_text_btn(
                            c,
                            Icon::Lock,
                            "Lock",
                            Kind::Secondary,
                            Some(Msg::LockNow)
                        )
                    ),
                    ui::fact(
                        c,
                        "Password check",
                        if self.pam == Some(true) {
                            "PAM service nordlys"
                        } else {
                            "PAM service login"
                        }
                    ),
                ]
            ),
        ]
        .spacing(26);
        if self.pam == Some(false) {
            body = body.push(ui::note(
                c,
                Icon::Shield,
                c.accent,
                "The lock screen checks your password with PAM's login service. Install config/pam.d/nordlys to give it its own, and run `nordlys-shell lock --check` once to be sure it takes your password.",
            ));
        }
        if !sys::has("hypridle") {
            body = body.push(ui::note(
                c,
                Icon::CircleAlert,
                c.warn,
                "hypridle isn't installed, so none of this happens on its own. Install it to dim, lock and sleep when you're away.",
            ));
        }
        body = body.push(row![
            ui::fill(),
            iced::widget::text(format!("Kept in {}", idle::user_path().display()))
                .size(12)
                .color(c.dim)
        ]);
        body.into()
    }
}
