use super::{PageState, Reads};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{apps, job};
use crate::theme::{BOLD, Kind, MONO};
use crate::ui::{self, El};
use iced::widget::{column, row, text};
use iced::{Alignment, Task};

#[derive(Default)]
pub struct State {
    list: Option<Vec<apps::Startup>>,
    reads: Reads,
    adding: bool,
    name: String,
    command: String,
    remove: Option<apps::Startup>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Vec<apps::Startup>),
    Toggle(usize, bool),
    Add,
    Name(String),
    Command(String),
    Save,
    CancelAdd,
    AskRemove(usize),
    Remove,
    CancelRemove,
    Done(Result<(), String>, String),
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(
            || Ok(apps::autostart()),
            |r: Result<_, String>| Msg::Read(r.unwrap_or_default()),
        )
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Read(v) => {
                self.reads.done();
                self.list = Some(v);
            }
            Msg::Toggle(i, on) => {
                let Some(s) = self.list.as_mut().and_then(|l| l.get_mut(i)) else {
                    return Task::none();
                };
                s.enabled = on;
                let s = s.clone();
                return job(
                    move || apps::set_autostart(&s, on),
                    |r| Msg::Done(r, String::new()),
                );
            }
            Msg::Add => self.adding = true,
            Msg::Name(s) => self.name = s,
            Msg::Command(s) => self.command = s,
            Msg::CancelAdd => {
                self.adding = false;
                self.name.clear();
                self.command.clear();
            }
            Msg::Save => {
                let (name, command) = (self.name.clone(), self.command.clone());
                if name.trim().is_empty() || command.trim().is_empty() {
                    ctx.fail("Give it a name and a command");
                    return Task::none();
                }
                self.adding = false;
                self.name.clear();
                self.command.clear();
                let ok = format!("{} starts when you log in", name.trim());
                return job(
                    move || apps::add_autostart(&name, &command),
                    move |r| Msg::Done(r, ok.clone()),
                );
            }
            Msg::AskRemove(i) => self.remove = self.list.as_ref().and_then(|l| l.get(i)).cloned(),
            Msg::CancelRemove => self.remove = None,
            Msg::Remove => {
                if let Some(s) = self.remove.take() {
                    let ok = format!("Removed {}", s.entry.name);
                    return job(
                        move || apps::remove_autostart(&s),
                        move |r| Msg::Done(r, ok.clone()),
                    );
                }
            }
            Msg::Done(r, ok) => {
                ctx.report(&r, &ok);
                return self.open(ctx);
            }
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let Some(list) = &self.list else {
            return ui::loading(c);
        };
        let mut rows: Vec<El<'a, Msg>> = list
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let detail = if s.entry.comment.is_empty() {
                    s.entry.exec.clone()
                } else {
                    s.entry.comment.clone()
                };
                ui::icon_setting(
                    c,
                    Icon::Rocket,
                    if s.enabled { c.accent } else { c.dim },
                    s.entry.name.clone(),
                    detail,
                    row![
                        if s.yours {
                            ui::icon_btn(c, Icon::Trash2, Kind::Ghost, Some(Msg::AskRemove(i)))
                        } else {
                            ui::gap(0.0)
                        },
                        ui::toggle(s.enabled, move |on| Msg::Toggle(i, on)),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
            })
            .collect();
        if rows.is_empty() {
            rows.push(ui::empty(c, "Nothing starts on its own when you log in."));
        }
        if self.adding {
            rows.push(
                column![
                    text("New startup command")
                        .size(14)
                        .font(BOLD)
                        .color(c.text),
                    ui::input(c, "Name, like Syncthing", &self.name).on_input(Msg::Name),
                    ui::input(
                        c,
                        "Command, like syncthing serve --no-browser",
                        &self.command
                    )
                    .on_input(Msg::Command)
                    .on_submit(Msg::Save)
                    .font(MONO),
                    row![
                        ui::fill(),
                        ui::btn(c, "Cancel", Kind::Secondary, Some(Msg::CancelAdd)),
                        ui::btn(
                            c,
                            "Add",
                            Kind::Primary,
                            (!self.name.trim().is_empty() && !self.command.trim().is_empty())
                                .then_some(Msg::Save)
                        ),
                    ]
                    .spacing(8),
                ]
                .spacing(10)
                .padding([16, 18])
                .into(),
            );
        } else {
            rows.push(ui::setting(
                c,
                "Add a command",
                "Run something of your own each time you log in",
                ui::icon_text_btn(c, Icon::Plus, "Add", Kind::Secondary, Some(Msg::Add)),
            ));
        }
        column![
            ui::section(c, "When you log in", rows),
            text(
                "Switching off an app that came with the system writes a small override in ~/.config/autostart, \
                 so its own file stays as it was."
            )
            .size(12)
            .color(c.dim),
        ]
        .spacing(26)
        .into()
    }

    fn dialog<'a>(&'a self, ctx: &'a Ctx) -> Option<El<'a, Msg>> {
        let c = ctx.c;
        let s = self.remove.as_ref()?;
        Some(
            column![
                text(format!("Remove {}?", s.entry.name))
                    .size(18)
                    .font(BOLD)
                    .color(c.text),
                text("It won't start when you log in any more. You can add it again later.")
                    .size(13)
                    .color(c.muted),
                row![
                    ui::fill(),
                    ui::btn(c, "Cancel", Kind::Secondary, Some(Msg::CancelRemove)),
                    ui::btn(c, "Remove", Kind::Danger, Some(Msg::Remove)),
                ]
                .spacing(8),
            ]
            .spacing(14)
            .into(),
        )
    }
}
