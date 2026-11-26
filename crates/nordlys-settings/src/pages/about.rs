use super::{PageState, Reads};
use crate::app::{Ctx, Page};
use crate::icons::Icon;
use crate::sys::{about, job};
use crate::theme::{BOLD, Kind};
use crate::ui::{self, El};
use iced::widget::{column, container, row, text};
use iced::{Alignment, Length, Task};

#[derive(Default)]
pub struct State {
    data: Option<Box<about::About>>,
    reads: Reads,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<Box<about::About>, String>),
    Copy,
    Rename,
}

/// What `Rename` asks the app to do: go to the page with the name on it.
pub const RENAME_ON: Page = Page::Network;

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(|| about::read().map(Box::new), Msg::Read)
    }

    fn tick(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.open(ctx)
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
            Msg::Copy => {
                let Some(d) = &self.data else {
                    return Task::none();
                };
                ctx.done("Copied the system details");
                iced::clipboard::write(summary(d))
            }
            // Handled by the app, which switches pages.
            Msg::Rename => Task::none(),
        }
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let Some(d) = &self.data else {
            return ui::loading(c);
        };
        let hero = container(
            row![
                ui::badge_icon(c, Icon::Cpu, c.accent),
                column![
                    text(if d.hostname.is_empty() {
                        "This computer".to_owned()
                    } else {
                        d.hostname.clone()
                    })
                    .size(22)
                    .font(BOLD)
                    .color(c.text),
                    text(if d.model.is_empty() {
                        d.os.clone()
                    } else {
                        format!("{} · {}", d.model, d.os)
                    })
                    .size(13)
                    .color(c.muted),
                ]
                .spacing(2),
                ui::fill(),
                ui::icon_text_btn(
                    c,
                    Icon::Pencil,
                    "Rename",
                    Kind::Secondary,
                    Some(Msg::Rename)
                ),
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        )
        .padding([18, 18]);

        let mem = if d.memory_total > 0 {
            d.memory_used as f32 / d.memory_total as f32
        } else {
            0.0
        };
        let disk = if d.disk_total > 0 {
            d.disk_used as f32 / d.disk_total as f32
        } else {
            0.0
        };
        let tint = |f: f32| {
            if f > 0.9 {
                c.bad
            } else if f > 0.75 {
                c.warn
            } else {
                c.accent
            }
        };
        let meter_row = |ic: Icon, title: &str, f: f32, detail: String| {
            ui::icon_setting(
                c,
                ic,
                tint(f),
                title.to_owned(),
                detail,
                container(ui::meter(c, f, tint(f))).width(220),
            )
        };
        let mut hardware = vec![
            hero.into(),
            ui::fact(
                c,
                "Processor",
                if d.cores > 0 {
                    format!("{} · {} threads", d.cpu, d.cores)
                } else {
                    d.cpu.clone()
                },
            ),
        ];
        for (i, g) in d.gpus.iter().enumerate() {
            hardware.push(ui::fact(c, if i == 0 { "Graphics" } else { "" }, g.clone()));
        }
        hardware.push(meter_row(
            Icon::MemoryStick,
            "Memory",
            mem,
            format!(
                "{} of {} in use{}",
                about::bytes(d.memory_used),
                about::bytes(d.memory_total),
                if d.swap_total > 0 {
                    format!(" · {} swap", about::bytes(d.swap_total))
                } else {
                    String::new()
                }
            ),
        ));
        hardware.push(meter_row(
            Icon::HardDrive,
            "Disk",
            disk,
            format!(
                "{} free of {}",
                about::bytes(d.disk_total.saturating_sub(d.disk_used)),
                about::bytes(d.disk_total)
            ),
        ));

        let software = vec![
            ui::fact(c, "System", d.os.clone()),
            ui::mono_fact(c, "Kernel", d.kernel.clone()),
            ui::fact(
                c,
                "Desktop",
                format!("Nordlys {}", env!("CARGO_PKG_VERSION")),
            ),
            ui::fact(c, "Compositor", d.compositor.clone()),
            ui::fact(c, "Session", d.session.clone()),
            ui::fact(c, "Signed in as", format!("{} · {}", d.user, d.shell)),
            ui::fact(c, "Up for", about::duration(d.uptime_secs)),
        ];
        column![
            ui::section(c, "Hardware", hardware),
            ui::section(c, "Software", software),
            row![
                ui::fill(),
                ui::icon_text_btn(
                    c,
                    Icon::FileText,
                    "Copy details",
                    Kind::Secondary,
                    Some(Msg::Copy)
                ),
            ]
            .width(Length::Fill),
        ]
        .spacing(26)
        .into()
    }
}

/// Plain text for a bug report.
fn summary(d: &about::About) -> String {
    let mut s = format!(
        "Nordlys {}\nSystem: {}\nKernel: {}\nCompositor: {}\nSession: {}\nModel: {}\nCPU: {} ({} threads)\n",
        env!("CARGO_PKG_VERSION"),
        d.os,
        d.kernel,
        d.compositor,
        d.session,
        d.model,
        d.cpu,
        d.cores
    );
    for g in &d.gpus {
        s.push_str(&format!("GPU: {g}\n"));
    }
    s.push_str(&format!("Memory: {}\n", about::bytes(d.memory_total)));
    s
}
