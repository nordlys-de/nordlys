use super::{PageState, Reads, toml_set};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{datetime, job};
use crate::theme::{BOLD, Kind, MONO};
use crate::ui::{self, El};
use iced::widget::{column, combo_box, row, text};
use iced::{Alignment, Task};

#[derive(Default)]
pub struct State {
    data: Option<datetime::DateTime>,
    reads: Reads,
    zones: combo_box::State<String>,
    locales: combo_box::State<String>,
    manual: String,
    failed: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<datetime::DateTime, String>),
    Zone(String),
    Ntp(bool),
    LocalRtc(bool),
    Manual(String),
    SetTime,
    Locale(String),
    Done(Result<(), String>, String),
    /// A `[clock]` key in nordlys.toml.
    Clock(&'static str, bool),
}

impl State {
    fn read(&mut self) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(datetime::read, Msg::Read)
    }
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        self.read()
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Read(r) => {
                self.reads.done();
                match r {
                    Ok(d) => {
                        let first = self.data.is_none();
                        if first || self.data.as_ref().is_some_and(|o| o.zones != d.zones) {
                            self.zones = combo_box::State::new(d.zones.clone());
                        }
                        if first || self.data.as_ref().is_some_and(|o| o.locales != d.locales) {
                            self.locales = combo_box::State::new(d.locales.clone());
                        }
                        if self.manual.is_empty() {
                            self.manual = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
                        }
                        self.data = Some(d);
                        self.failed = None;
                    }
                    Err(e) => self.failed = Some(e),
                }
                Task::none()
            }
            Msg::Zone(z) => {
                let ok = format!("Time zone is now {z}");
                job(
                    move || datetime::set_timezone(&z),
                    move |r| Msg::Done(r, ok.clone()),
                )
            }
            Msg::Ntp(on) => job(
                move || datetime::set_ntp(on),
                |r| Msg::Done(r, String::new()),
            ),
            Msg::LocalRtc(on) => job(
                move || datetime::set_local_rtc(on),
                |r| Msg::Done(r, String::new()),
            ),
            Msg::Manual(s) => {
                self.manual = s;
                Task::none()
            }
            Msg::SetTime => {
                let s = self.manual.trim().to_owned();
                if !datetime::valid_time(&s) {
                    ctx.fail("Write the time as 2026-11-25 14:30");
                    return Task::none();
                }
                job(
                    move || datetime::set_time(&s),
                    |r| Msg::Done(r, "Clock set".into()),
                )
            }
            Msg::Locale(l) => {
                let ok = format!("Language set to {l}. Apps pick it up the next time you log in.");
                job(
                    move || datetime::set_locale(&l),
                    move |r| Msg::Done(r, ok.clone()),
                )
            }
            Msg::Clock(key, on) => {
                toml_set(ctx, "clock", key, on);
                Task::none()
            }
            Msg::Done(r, ok) => {
                ctx.report(&r, &ok);
                self.read()
            }
        }
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        // Nordlys's own clock needs no timedatectl, so it's always here.
        let shown = &ctx.toml.config.clock;
        let bar = ui::section(
            c,
            "Top bar and lock screen",
            vec![
                ui::toggle_row(
                    c,
                    "12-hour time",
                    format!(
                        "The top bar reads {}",
                        chrono::Local::now().format(&shown.bar_format())
                    ),
                    shown.twelve_hour,
                    |on| Msg::Clock("twelve_hour", on),
                ),
                ui::toggle_row(
                    c,
                    "Date on the top bar",
                    "The day and date before the time",
                    shown.date,
                    |on| Msg::Clock("date", on),
                ),
                ui::toggle_row(
                    c,
                    "Seconds on the top bar",
                    "Count the seconds beside the time",
                    shown.seconds,
                    |on| Msg::Clock("seconds", on),
                ),
            ],
        );
        if let Some(e) = &self.failed {
            return column![
                bar,
                ui::note(
                    c,
                    Icon::CircleAlert,
                    c.warn,
                    format!(
                        "Couldn't ask systemd about the clock ({e}). The rest of this page needs timedatectl."
                    ),
                ),
            ]
            .spacing(26)
            .into();
        }
        let Some(d) = &self.data else {
            return column![bar, ui::loading(c)].spacing(26).into();
        };
        let now = chrono::Local::now();
        let clock = column![
            text(now.format("%H:%M:%S").to_string())
                .size(44)
                .font(BOLD)
                .color(c.text),
            text(now.format("%A %-d %B %Y").to_string())
                .size(14)
                .color(c.muted),
            text(format!("{} · UTC{}", d.timezone, now.format("%:z")))
                .size(12)
                .font(MONO)
                .color(c.dim),
        ]
        .spacing(4);

        let mut clock_rows = vec![
            iced::widget::container(clock).padding([18, 18]).into(),
            ui::setting(
                c,
                "Time zone",
                "Type a city to find it",
                combo_box(&self.zones, "Europe/Oslo", Some(&d.timezone), Msg::Zone)
                    .width(260)
                    .padding([6, 12])
                    .size(13.0),
            ),
            ui::toggle_row(
                c,
                "Set the time automatically",
                if !d.can_ntp {
                    "No network time service is installed (systemd-timesyncd)"
                } else if d.ntp && d.synced {
                    "In step with network time"
                } else if d.ntp {
                    "Waiting to reach a time server"
                } else {
                    "Off: the clock keeps whatever time you set"
                },
                d.ntp,
                Msg::Ntp,
            ),
        ];
        if !d.ntp {
            clock_rows.push(ui::setting(
                c,
                "Date and time",
                "Year-month-day hours:minutes",
                row![
                    ui::input(c, "2026-11-25 14:30", &self.manual)
                        .on_input(Msg::Manual)
                        .on_submit(Msg::SetTime)
                        .font(MONO)
                        .width(170),
                    ui::btn(
                        c,
                        "Set",
                        Kind::Primary,
                        datetime::valid_time(self.manual.trim()).then_some(Msg::SetTime)
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ));
        }
        clock_rows.push(ui::toggle_row(
            c,
            "Hardware clock on local time",
            "Only for sharing the computer with Windows; otherwise leave off",
            d.local_rtc,
            Msg::LocalRtc,
        ));

        let lang = if d.locales.is_empty() {
            vec![ui::fact(
                c,
                "Language and formats",
                if d.locale.is_empty() {
                    "Not set"
                } else {
                    &d.locale
                },
            )]
        } else {
            vec![ui::setting(
                c,
                "Language and formats",
                "Dates, numbers and app language for the whole system",
                combo_box(
                    &self.locales,
                    "en_GB.UTF-8",
                    (!d.locale.is_empty()).then_some(&d.locale),
                    Msg::Locale,
                )
                .width(260)
                .padding([6, 12])
                .size(13.0),
            )]
        };
        column![
            ui::section(c, "Clock", clock_rows),
            bar,
            ui::section(c, "Region", lang),
            text("Changing the time zone, clock sync or language asks for your password, since they apply to everyone using this computer.")
                .size(12)
                .color(c.dim),
        ]
        .spacing(26)
        .into()
    }
}
