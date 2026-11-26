use super::PageState;
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{self, job};
use crate::theme::{Kind, MONO};
use crate::ui::{self, El};
use iced::widget::{column, container, row, text};
use iced::{Alignment, Length, Task};
use nordlys_config::{Group, OpenIn};

#[derive(Default)]
pub struct State {
    /// The group being edited: its index, or None for a new one.
    editing: Option<(Option<usize>, Draft)>,
    confirm_delete: Option<usize>,
}

#[derive(Debug, Clone, Default)]
struct Draft {
    name: String,
    key: String,
    apps: String,
    open_in: OpenInChoice,
    dir: String,
    reuse_running: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OpenInChoice {
    #[default]
    New,
    Current,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Add,
    Edit(usize),
    Name(String),
    Key(String),
    Apps(String),
    OpenIn(OpenInChoice),
    Dir(String),
    Reuse(bool),
    Save,
    Cancel,
    AskDelete(usize),
    Delete,
    CancelDelete,
    Launch(usize),
    Launched(Result<String, String>),
}

impl Draft {
    fn of(g: &Group) -> Self {
        Self {
            name: g.name.clone(),
            key: g.key.map(String::from).unwrap_or_default(),
            apps: g.apps.join(" "),
            open_in: match g.open_in {
                OpenIn::NewWorkspace => OpenInChoice::New,
                OpenIn::Current => OpenInChoice::Current,
            },
            dir: g.dir.clone().unwrap_or_default(),
            reuse_running: g.reuse_running,
        }
    }

    fn group(&self, others: &[Group]) -> Result<Group, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Give the group a name".into());
        }
        let apps: Vec<String> = self
            .apps
            .split([' ', ',', '\n'])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if apps.is_empty() {
            return Err("List at least one app, like: firefox foot".into());
        }
        let key = match self.key.trim() {
            "" => None,
            k if k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => {
                k.chars().next().map(|c| c.to_ascii_lowercase())
            }
            _ => return Err("The key is one letter or digit".into()),
        };
        if let Some(k) = key
            && let Some(g) = others
                .iter()
                .find(|g| g.key.map(|x| x.to_ascii_lowercase()) == Some(k))
        {
            return Err(format!("Super G then {k} already opens {}", g.name));
        }
        if others.iter().any(|g| g.name.eq_ignore_ascii_case(name)) {
            return Err(format!("There's already a group called {name}"));
        }
        Ok(Group {
            name: name.to_owned(),
            key,
            apps,
            open_in: match self.open_in {
                OpenInChoice::New => OpenIn::NewWorkspace,
                OpenInChoice::Current => OpenIn::Current,
            },
            dir: Some(self.dir.trim().to_owned()).filter(|d| !d.is_empty()),
            reuse_running: self.reuse_running,
        })
    }
}

impl PageState for State {
    type Msg = Msg;

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        let groups = ctx.toml.config.groups.clone();
        match msg {
            Msg::Add => {
                self.editing = Some((
                    None,
                    Draft {
                        reuse_running: true,
                        ..Draft::default()
                    },
                ))
            }
            Msg::Edit(i) => {
                if let Some(g) = groups.get(i) {
                    self.editing = Some((Some(i), Draft::of(g)));
                }
            }
            Msg::Name(s) => self.with(|d| d.name = s),
            Msg::Key(s) => {
                self.with(|d| d.key = s.chars().last().map(String::from).unwrap_or_default())
            }
            Msg::Apps(s) => self.with(|d| d.apps = s),
            Msg::OpenIn(o) => self.with(|d| d.open_in = o),
            Msg::Dir(s) => self.with(|d| d.dir = s),
            Msg::Reuse(on) => self.with(|d| d.reuse_running = on),
            Msg::Cancel => self.editing = None,
            Msg::Save => {
                let Some((index, draft)) = &self.editing else {
                    return Task::none();
                };
                let others: Vec<Group> = groups
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| Some(*i) != *index)
                    .map(|(_, g)| g.clone())
                    .collect();
                match draft.group(&others) {
                    Err(e) => ctx.fail(e),
                    Ok(g) => {
                        let mut all = groups.clone();
                        match index {
                            Some(i) => all[*i] = g.clone(),
                            None => all.push(g.clone()),
                        }
                        match ctx.toml.set_groups(&all) {
                            Ok(()) => {
                                ctx.done(format!("Saved {}", g.name));
                                self.editing = None;
                            }
                            Err(e) => ctx.fail(e),
                        }
                    }
                }
            }
            Msg::AskDelete(i) => self.confirm_delete = Some(i),
            Msg::CancelDelete => self.confirm_delete = None,
            Msg::Delete => {
                if let Some(i) = self.confirm_delete.take()
                    && i < groups.len()
                {
                    let mut all = groups.clone();
                    let gone = all.remove(i);
                    let r = ctx.toml.set_groups(&all);
                    ctx.report(&r, &format!("Removed {}", gone.name));
                }
            }
            Msg::Launch(i) => {
                if let Some(g) = groups.get(i).cloned() {
                    return job(move || launch(&g), Msg::Launched);
                }
            }
            Msg::Launched(r) => ctx.report(&r, ""),
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let groups = &ctx.toml.config.groups;
        let mut rows: Vec<El<'a, Msg>> = vec![];
        for (i, g) in groups.iter().enumerate() {
            let key = g
                .key
                .map(|k| format!("Super G · {}", k.to_ascii_uppercase()))
                .unwrap_or_else(|| "No key".into());
            let detail = format!(
                "{} · {}{}",
                g.apps.join(", "),
                match g.open_in {
                    OpenIn::NewWorkspace => "new workspace",
                    OpenIn::Current => "this workspace",
                },
                g.dir
                    .as_deref()
                    .map(|d| format!(" · in {d}"))
                    .unwrap_or_default()
            );
            rows.push(ui::icon_setting(
                c,
                Icon::Layers,
                crate::theme::color(nordlys_config::Rgb::hex_u32(
                    [0xC9A7FF, 0xA8D8A0, 0xF4B183, 0x8AB4FF, 0xF2A7C3, 0x8EE0D4][i % 6],
                )),
                g.name.clone(),
                detail,
                row![
                    ui::chip(c, key, c.accent),
                    ui::icon_btn(c, Icon::Play, Kind::Ghost, Some(Msg::Launch(i))),
                    ui::icon_btn(c, Icon::Pencil, Kind::Ghost, Some(Msg::Edit(i))),
                    ui::icon_btn(c, Icon::Trash2, Kind::Ghost, Some(Msg::AskDelete(i))),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            ));
        }
        if rows.is_empty() {
            rows.push(ui::empty(c, "No groups yet. A group opens a set of apps at once: an editor, a terminal and a browser for one project, say."));
        }
        let mut body = column![
            ui::note(
                c,
                Icon::Info,
                c.accent,
                "Press Super G, then the group's key. The first app takes the stage; the rest wait on the shelf in order."
            ),
            ui::section(c, "Groups", rows),
        ]
        .spacing(26);
        match &self.editing {
            None => {
                body = body.push(row![
                    ui::fill(),
                    ui::icon_text_btn(c, Icon::Plus, "New group", Kind::Primary, Some(Msg::Add))
                ]);
            }
            Some((index, d)) => {
                body = body.push(ui::section(
                    c,
                    if index.is_some() { "Edit group" } else { "New group" },
                    vec![
                        ui::setting(
                            c,
                            "Name",
                            "",
                            ui::input(c, "Research", &d.name).on_input(Msg::Name).width(280),
                        ),
                        ui::setting(
                            c,
                            "Key",
                            "Super G, then this letter",
                            ui::input(c, "r", &d.key).on_input(Msg::Key).width(60),
                        ),
                        ui::setting(
                            c,
                            "Apps",
                            "App ids or commands in order, separated by spaces",
                            ui::input(c, "firefox foot", &d.apps)
                                .on_input(Msg::Apps)
                                .font(MONO)
                                .width(280),
                        ),
                        ui::setting(
                            c,
                            "Open in",
                            "",
                            ui::segmented(
                                c,
                                &[(OpenInChoice::New, "New workspace"), (OpenInChoice::Current, "This workspace")],
                                d.open_in,
                                Msg::OpenIn,
                            ),
                        ),
                        ui::setting(
                            c,
                            "Folder",
                            "Where the apps start; ~ is your home",
                            ui::input(c, "~/code/project", &d.dir).on_input(Msg::Dir).width(280),
                        ),
                        ui::toggle_row(
                            c,
                            "Reuse open apps",
                            "Bring an app that's already open instead of a second copy (nordlys-comp)",
                            d.reuse_running,
                            Msg::Reuse,
                        ),
                        container(
                            row![
                                ui::fill(),
                                ui::btn(c, "Cancel", Kind::Secondary, Some(Msg::Cancel)),
                                ui::btn(c, "Save group", Kind::Primary, Some(Msg::Save)),
                            ]
                            .spacing(8),
                        )
                        .padding([12, 18])
                        .into(),
                    ],
                ));
            }
        }
        body.into()
    }

    fn dialog<'a>(&'a self, ctx: &'a Ctx) -> Option<El<'a, Msg>> {
        let c = ctx.c;
        let i = self.confirm_delete?;
        let g = ctx.toml.config.groups.get(i)?;
        Some(
            column![
                text(format!("Remove {}?", g.name))
                    .size(18)
                    .font(crate::theme::BOLD)
                    .color(c.text),
                text("The apps stay installed; only the group goes.")
                    .size(13)
                    .color(c.muted),
                row![
                    ui::fill(),
                    ui::btn(c, "Cancel", Kind::Secondary, Some(Msg::CancelDelete)),
                    ui::btn(c, "Remove", Kind::Danger, Some(Msg::Delete)),
                ]
                .spacing(8)
            ]
            .spacing(14)
            .width(Length::Fill)
            .into(),
        )
    }
}

impl State {
    fn with(&mut self, f: impl FnOnce(&mut Draft)) {
        if let Some((_, d)) = self.editing.as_mut() {
            f(d);
        }
    }
}

/// Open a group now: through nordlys-comp when it runs, else each app in
/// order, as the launcher does.
fn launch(g: &Group) -> Result<String, String> {
    if sys::run(&sys::sibling("nordlysctl"), &["group", &g.name]).is_ok() {
        return Ok(format!("Opened {}", g.name));
    }
    let dir = g.dir_path().unwrap_or_else(sys::home);
    for app in &g.apps {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exec {app}"))
            .current_dir(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("{app}: {e}"))?;
    }
    Ok(format!("Opened {}", g.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_are_checked() {
        let other = Group {
            name: "Dev".into(),
            key: Some('d'),
            apps: vec!["foot".into()],
            open_in: OpenIn::Current,
            dir: None,
            reuse_running: true,
        };
        let mut d = Draft {
            name: "Research".into(),
            key: "R".into(),
            apps: "firefox, helix".into(),
            reuse_running: true,
            ..Draft::default()
        };
        let g = d.group(std::slice::from_ref(&other)).unwrap();
        assert_eq!(g.key, Some('r'));
        assert_eq!(g.apps, ["firefox", "helix"]);
        d.key = "d".into();
        assert!(d.group(std::slice::from_ref(&other)).is_err());
        d.key.clear();
        d.apps.clear();
        assert!(d.group(&[]).is_err());
    }
}
