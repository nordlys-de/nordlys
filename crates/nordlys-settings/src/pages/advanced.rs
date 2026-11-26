use super::{PageState, hypr_set};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::store::{V, idle};
use crate::sys::{self, job};
use crate::theme::{BOLD, Kind, MONO};
use crate::ui::{self, El};
use iced::widget::{column, row, text};
use iced::{Alignment, Task};
use std::path::PathBuf;

#[derive(Default)]
pub struct State {
    confirm_reset: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Open(PathBuf),
    OpenFolder,
    Reload,
    RestartShell,
    Retheme,
    Bool(&'static str, bool),
    Int(&'static str, i64),
    ResetOne(String),
    AskReset,
    Reset,
    CancelReset,
    Done(Result<(), String>, String),
}

/// What a fresh `~/.config/nordlys/hyprland.lua` says.
const USER_LUA: &str = "-- Your own Hyprland settings. Nordlys runs this last, after the\n\
-- session's settings and the ones made in Settings, so anything here wins.\n\
--\n\
-- hl.config({ general = { gaps_out = 20 } })\n\
-- hl.bind(\"SUPER + T\", hl.dsp.exec_cmd(\"kitty\"))\n";

fn user_lua() -> PathBuf {
    sys::config_home().join("nordlys/hyprland.lua")
}

fn open(path: PathBuf) -> Result<(), String> {
    if !path.exists() {
        if path == user_lua() {
            sys::write_atomic(&path, USER_LUA)?;
        } else {
            return Err(format!(
                "{} doesn't exist yet; change a setting first",
                path.display()
            ));
        }
    }
    sys::spawn("xdg-open", &[&path.to_string_lossy()])
}

impl PageState for State {
    type Msg = Msg;

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Open(p) => return job(move || open(p), |r| Msg::Done(r, String::new())),
            Msg::OpenFolder => {
                let dir = sys::config_home().join("nordlys");
                return job(
                    move || {
                        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                        sys::spawn("xdg-open", &[&dir.to_string_lossy()])
                    },
                    |r| Msg::Done(r, String::new()),
                );
            }
            Msg::Reload => {
                return job(
                    || sys::run("hyprctl", &["reload"]).map(|_| ()),
                    |r| Msg::Done(r, "Hyprland read its settings again".into()),
                );
            }
            Msg::RestartShell => {
                return job(
                    || {
                        sys::run_for(
                            sys::SLOW,
                            "systemctl",
                            &["--user", "restart", "nordlys-shell@*.service"],
                        )
                        .map(|_| ())
                    },
                    |r| Msg::Done(r, "Bar, rails and close button restarted".into()),
                );
            }
            Msg::Retheme => {
                let theme = sys::sibling("nordlys-theme");
                return job(
                    move || sys::run_for(sys::SLOW, &theme, &[]).map(|_| ()),
                    |r| Msg::Done(r, "GTK, Qt and terminal colours rewritten".into()),
                );
            }
            Msg::Bool(k, on) => hypr_set(ctx, k, V::Bool(on)),
            Msg::Int(k, v) => hypr_set(ctx, k, V::Int(v)),
            Msg::ResetOne(k) => {
                let r = ctx.hypr.reset(&k);
                ctx.report(&r, &format!("{k} is back to its default"));
            }
            Msg::AskReset => self.confirm_reset = true,
            Msg::CancelReset => self.confirm_reset = false,
            Msg::Reset => {
                self.confirm_reset = false;
                let r = ctx.hypr.reset_all();
                ctx.report(&r, "Hyprland settings are back to the Nordlys defaults");
            }
            Msg::Done(r, ok) => ctx.report(&r, &ok),
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let mut body = column![].spacing(26);
        if let Some(p) = &ctx.toml.problem {
            body = body.push(ui::note(
                c,
                Icon::CircleAlert,
                c.bad,
                format!(
                    "nordlys.toml has a mistake, so the desktop is using its defaults and these pages can't save to it: {p}"
                ),
            ));
        }
        let file = |title: &str, detail: &str, path: PathBuf| {
            let shown = path
                .to_string_lossy()
                .replace(&*sys::home().to_string_lossy(), "~");
            ui::setting(
                c,
                title.to_owned(),
                format!("{detail}\n{shown}"),
                ui::icon_text_btn(
                    c,
                    Icon::ExternalLink,
                    "Open",
                    Kind::Secondary,
                    Some(Msg::Open(path)),
                ),
            )
        };
        body = body.push(ui::section(
            c,
            "Configuration files",
            vec![
                file(
                    "Desktop",
                    "Groups, panels, theme and layout",
                    ctx.toml.path.clone(),
                ),
                file(
                    "Your Hyprland additions",
                    "Binds, rules and options of your own; runs last",
                    user_lua(),
                ),
                file(
                    "Made by Settings",
                    "Written by these pages; don't edit by hand",
                    ctx.hypr.lua_path(),
                ),
                file(
                    "Idle",
                    "When the screen dims, locks and sleeps",
                    idle::user_path(),
                ),
                ui::setting(
                    c,
                    "All of them",
                    "",
                    ui::icon_text_btn(
                        c,
                        Icon::Folder,
                        "Show folder",
                        Kind::Ghost,
                        Some(Msg::OpenFolder),
                    ),
                ),
            ],
        ));
        body = body.push(ui::section(
            c,
            "Session",
            vec![
                ui::setting(
                    c,
                    "Reload Hyprland",
                    "Read hyprland.lua again after editing it",
                    ui::icon_text_btn(
                        c,
                        Icon::RefreshCw,
                        "Reload",
                        Kind::Secondary,
                        sys::on_hyprland().then_some(Msg::Reload),
                    ),
                ),
                ui::setting(
                    c,
                    "Restart the shell",
                    "Bar, rails and close button, if one is stuck",
                    ui::icon_text_btn(
                        c,
                        Icon::RotateCcw,
                        "Restart",
                        Kind::Secondary,
                        Some(Msg::RestartShell),
                    ),
                ),
                ui::setting(
                    c,
                    "Re-theme apps",
                    "Write GTK, Qt and terminal colours from the Nordlys palette",
                    ui::icon_text_btn(
                        c,
                        Icon::Palette,
                        "Apply",
                        Kind::Secondary,
                        Some(Msg::Retheme),
                    ),
                ),
            ],
        ));
        let h = &ctx.hypr;
        let b = |k: &'static str| h.get(k).bool();
        let i = |k: &'static str| h.get(k).int();
        body = body.push(ui::section(
            c,
            "Graphics",
            vec![
                ui::toggle_row(
                    c,
                    "Allow tearing",
                    "Lower input lag for games that ask for it, at the cost of a torn frame now and then",
                    b("general:allow_tearing"),
                    |on| Msg::Bool("general:allow_tearing", on),
                ),
                ui::setting(
                    c,
                    "Direct scanout",
                    "Hand a fullscreen app's frames straight to the screen; can flicker on some GPUs",
                    ui::segmented(c, &[(0i64, "Off"), (1, "On"), (2, "Games only")], i("render:direct_scanout"), |v| {
                        Msg::Int("render:direct_scanout", v)
                    }),
                ),
                ui::setting(
                    c,
                    "Hardware cursor",
                    "Turn off if the pointer flickers or is drawn in the wrong place",
                    ui::segmented(c, &[(0i64, "On"), (1, "Off"), (2, "Auto")], i("cursor:no_hardware_cursors"), |v| {
                        Msg::Int("cursor:no_hardware_cursors", v)
                    }),
                ),
            ],
        ));
        body = body.push(ui::section(
            c,
            "Older apps (XWayland)",
            vec![
                ui::toggle_row(
                    c,
                    "Run X11 apps",
                    "Off saves a little memory if every app you use is Wayland-native; applies after logging in again",
                    b("xwayland:enabled"),
                    |on| Msg::Bool("xwayland:enabled", on),
                ),
                ui::toggle_row(
                    c,
                    "Sharp X11 apps on scaled screens",
                    "Draw them at full resolution instead of stretched; they may look small",
                    b("xwayland:force_zero_scaling"),
                    |on| Msg::Bool("xwayland:force_zero_scaling", on),
                ),
            ],
        ));
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "not set".into());
        body = body.push(ui::section(
            c,
            "Troubleshooting",
            vec![
                ui::toggle_row(
                    c,
                    "Hyprland debug logs",
                    "Write detailed logs to $XDG_RUNTIME_DIR/hypr for bug reports",
                    !b("debug:disable_logs"),
                    |on| Msg::Bool("debug:disable_logs", !on),
                ),
                ui::mono_fact(c, "XDG_CURRENT_DESKTOP", env("XDG_CURRENT_DESKTOP")),
                ui::mono_fact(c, "XDG_SESSION_TYPE", env("XDG_SESSION_TYPE")),
                ui::mono_fact(c, "WAYLAND_DISPLAY", env("WAYLAND_DISPLAY")),
                ui::mono_fact(
                    c,
                    "Settings saved in",
                    ctx.hypr
                        .lua_path()
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            ],
        ));
        let changed: Vec<El<'a, Msg>> = ctx
            .hypr
            .changed()
            .into_iter()
            .map(|(k, v)| {
                let shown = match v {
                    V::Str(s) if s.is_empty() => "(empty)".to_owned(),
                    other => other.lua().trim_matches('"').to_owned(),
                };
                ui::setting(
                    c,
                    k.to_owned(),
                    shown,
                    ui::icon_btn(
                        c,
                        Icon::RotateCcw,
                        Kind::Ghost,
                        Some(Msg::ResetOne(k.to_owned())),
                    ),
                )
            })
            .collect();
        if !changed.is_empty() {
            body = body.push(ui::section(c, "Changed here", changed));
        }
        body.push(ui::section(
            c,
            "Start over",
            vec![ui::setting(
                c,
                "Reset Hyprland settings",
                "Forget every window, input and screen setting made here. nordlys.toml and your own hyprland.lua stay.",
                ui::btn(c, "Reset…", Kind::Danger, Some(Msg::AskReset)),
            )],
        ))
        .into()
    }

    fn dialog<'a>(&'a self, ctx: &'a Ctx) -> Option<El<'a, Msg>> {
        let c = ctx.c;
        if !self.confirm_reset {
            return None;
        }
        Some(
            column![
                text("Reset Hyprland settings?").size(18).font(BOLD).color(c.text),
                text("Gaps, borders, keyboard, mouse, touchpad and screen arrangements go back to how Nordlys ships. Hyprland reloads straight away.")
                    .size(13)
                    .color(c.muted),
                text(ctx.hypr.lua_path().display().to_string()).size(11).font(MONO).color(c.dim),
                row![
                    ui::fill(),
                    ui::btn(c, "Cancel", Kind::Secondary, Some(Msg::CancelReset)),
                    ui::btn(c, "Reset", Kind::Danger, Some(Msg::Reset)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(14)
            .into(),
        )
    }
}
