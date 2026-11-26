use super::{PageState, Reads};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{apps, job};
use crate::ui::{self, El};
use iced::Task;
use iced::widget::{column, pick_list, text};

#[derive(Default)]
pub struct State {
    data: Option<Data>,
    reads: Reads,
}

#[derive(Debug, Clone)]
pub struct Data {
    /// For each of `apps::KINDS`: the apps that can, and the one that does.
    kinds: Vec<(Vec<App>, Option<App>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    id: String,
    name: String,
}

impl std::fmt::Display for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<Data, String>),
    Set(usize, App),
    Done(Result<(), String>, String),
}

fn read() -> Result<Data, String> {
    let installed = apps::installed();
    let app = |e: &apps::Entry| App {
        id: e.id.clone(),
        name: e.name.clone(),
    };
    let kinds = apps::KINDS
        .iter()
        .map(|k| {
            let mut can: Vec<App> = vec![];
            for m in k.mimes {
                for e in apps::candidates(&installed, m) {
                    if !can.iter().any(|a| a.id == e.id) {
                        can.push(app(e));
                    }
                }
            }
            can.sort_by_key(|a| a.name.to_lowercase());
            let now = apps::default_for(k.mimes[0]).map(|id| {
                can.iter()
                    .find(|a| a.id == id)
                    .cloned()
                    .unwrap_or_else(|| App {
                        name: installed
                            .iter()
                            .find(|e| e.id == id)
                            .map(|e| e.name.clone())
                            .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned()),
                        id,
                    })
            });
            (can, now)
        })
        .collect();
    Ok(Data { kinds })
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(read, Msg::Read)
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
            Msg::Set(i, app) => {
                let Some(kind) = apps::KINDS.get(i) else {
                    return Task::none();
                };
                if let Some(d) = self.data.as_mut() {
                    d.kinds[i].1 = Some(app.clone());
                }
                let ok = format!("{} now opens {}", app.name, kind.label.to_lowercase());
                job(
                    move || apps::set_default(&app.id, kind.mimes),
                    move |r| Msg::Done(r, ok.clone()),
                )
            }
            Msg::Done(r, ok) => {
                ctx.report(&r, &ok);
                self.open(ctx)
            }
        }
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let Some(d) = &self.data else {
            return ui::loading(c);
        };
        let rows = apps::KINDS
            .iter()
            .zip(&d.kinds)
            .enumerate()
            .map(|(i, (k, (can, now)))| {
                if can.is_empty() {
                    ui::setting(
                        c,
                        k.label,
                        "",
                        text("No app installed for this").size(13).color(c.dim),
                    )
                } else {
                    ui::setting(
                        c,
                        k.label,
                        "",
                        pick_list(can.clone(), now.clone(), move |a| Msg::Set(i, a))
                            .placeholder("Choose an app")
                            .padding([6, 12])
                            .text_size(13)
                            .width(260),
                    )
                }
            })
            .collect();
        column![
            ui::section(c, "Open with", rows),
            ui::note(
                c,
                Icon::Info,
                c.accent,
                "Each choice covers the usual types for it (a browser gets web pages and http/https links, say). \
                 Apps can still be picked one file at a time from their own Open menus.",
            ),
        ]
        .spacing(26)
        .into()
    }
}
