use super::{PageState, hypr_set};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::store::V;
use crate::theme::{Kind, MONO};
use crate::ui::{self, El};
use iced::widget::{column, container, row, text};
use iced::{Alignment, Length, Task};

#[derive(Default)]
pub struct State {
    layout: String,
    variant: String,
    options: String,
    drag: Option<(&'static str, f32)>,
    test: String,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Layout(String),
    Variant(String),
    Options(String),
    ApplyLayout,
    Preset(&'static str),
    Drag(&'static str, f32),
    Release,
    Numlock(bool),
    Test(String),
}

/// Common xkb options, as a quick way to add one.
const PRESETS: [(&str, &str); 5] = [
    ("caps:escape", "Caps Lock is Escape"),
    ("caps:ctrl_modifier", "Caps Lock is Ctrl"),
    ("grp:alt_shift_toggle", "Alt+Shift switches layout"),
    ("grp:win_space_toggle", "Super+Space switches layout"),
    ("compose:ralt", "Right Alt is Compose"),
];

/// The session's keys (session/hyprland.lua).
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Stage and shelf",
        &[
            ("Super K", "Previous window to the stage"),
            ("Super J", "Cycle through the shelf"),
            ("Super T", "Tiled view on or off (two-up)"),
            ("Super Tab", "Swap main and side"),
            ("Super S", "Send to the shelf"),
            ("Super G", "App groups"),
            ("Super Q", "Close the window"),
            ("Super F", "Float the window"),
        ],
    ),
    (
        "Workspaces",
        &[
            ("Super 1–9", "Go to a workspace"),
            ("Super Shift 1–9", "Send the window there"),
            ("Super [  ]", "Previous · next workspace"),
            ("Super scroll", "Previous · next workspace"),
            ("Super -", "Scratchpad"),
            ("Super Shift -", "Send to the scratchpad"),
        ],
    ),
    (
        "Apps and system",
        &[
            ("Super Space", "Launcher"),
            ("Super Shift Space", "Commands"),
            ("Super Enter", "Terminal"),
            ("Super E", "Editor"),
            ("Super ,", "Settings"),
            ("Super Shift S", "Screenshot a region to the clipboard"),
            ("Super Esc · Super L", "Lock"),
            ("Super Shift Esc, twice", "Log out"),
        ],
    ),
    (
        "Mouse",
        &[
            ("Super drag", "Move a window"),
            ("Super right-drag", "Resize a window"),
            ("Three-finger swipe", "Switch workspace"),
        ],
    ),
];

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.layout = ctx.hypr.get("input:kb_layout").str();
        self.variant = ctx.hypr.get("input:kb_variant").str();
        self.options = ctx.hypr.get("input:kb_options").str();
        Task::none()
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Layout(s) => self.layout = s,
            Msg::Variant(s) => self.variant = s,
            Msg::Options(s) => self.options = s,
            Msg::ApplyLayout => {
                let layout = tidy(&self.layout);
                if layout.is_empty() {
                    ctx.fail("Name at least one layout, like us or us,de");
                    return Task::none();
                }
                if let Some(bad) = layout.split(',').find(|l| !known_layout(l)) {
                    ctx.fail(format!("{bad:?} isn't a keyboard layout xkb knows"));
                    return Task::none();
                }
                // Order matters: Hyprland checks the layout and variant together.
                let r = ctx
                    .hypr
                    .set("input:kb_variant", V::Str(tidy(&self.variant)))
                    .and_then(|_| ctx.hypr.set("input:kb_layout", V::Str(layout.clone())))
                    .and_then(|_| {
                        ctx.hypr
                            .set("input:kb_options", V::Str(tidy(&self.options)))
                    });
                ctx.report(&r, &format!("Keyboard layout: {layout}"));
                self.layout = layout;
            }
            Msg::Preset(opt) => {
                let mut opts: Vec<String> = tidy(&self.options)
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                if let Some(i) = opts.iter().position(|o| o == opt) {
                    opts.remove(i);
                } else {
                    // One choice per group: caps:escape replaces caps:ctrl_modifier.
                    let group = opt.split(':').next().unwrap_or("");
                    opts.retain(|o| !o.starts_with(&format!("{group}:")));
                    opts.push(opt.to_owned());
                }
                self.options = opts.join(",");
                return self.update(Msg::ApplyLayout, ctx);
            }
            Msg::Drag(k, v) => self.drag = Some((k, v)),
            Msg::Release => {
                if let Some((k, v)) = self.drag.take() {
                    hypr_set(ctx, k, V::Int(v.round() as i64));
                }
            }
            Msg::Numlock(on) => hypr_set(ctx, "input:numlock_by_default", V::Bool(on)),
            Msg::Test(s) => self.test = s,
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let h = &ctx.hypr;
        let val = |k: &'static str| match self.drag {
            Some((d, v)) if d == k => v,
            _ => h.get(k).int() as f32,
        };
        let rate = val("input:repeat_rate");
        let delay = val("input:repeat_delay");
        let opts = tidy(&self.options);
        let mut presets = row![].spacing(8);
        for (opt, label) in PRESETS {
            let on = opts.split(',').any(|o| o == opt);
            presets = presets.push(ui::btn(
                c,
                label,
                if on { Kind::Primary } else { Kind::Secondary },
                Some(Msg::Preset(opt)),
            ));
        }
        let mut shortcut_sections = column![].spacing(26);
        for (title, keys) in SHORTCUTS {
            shortcut_sections = shortcut_sections.push(ui::section(
                c,
                title,
                keys.iter()
                    .map(|(k, what)| {
                        row![
                            text(*what).size(14).color(c.text).width(Length::Fill),
                            keycaps(c, k)
                        ]
                        .align_y(Alignment::Center)
                        .padding([10, 18])
                        .into()
                    })
                    .collect(),
            ));
        }
        column![
            ui::section(
                c,
                "Layout",
                vec![
                    ui::setting(
                        c,
                        "Layouts",
                        "xkb names, comma-separated: us, de, fr, us,ru…",
                        ui::input(c, "us", &self.layout)
                            .on_input(Msg::Layout)
                            .on_submit(Msg::ApplyLayout)
                            .font(MONO)
                            .width(200)
                    ),
                    ui::setting(
                        c,
                        "Variants",
                        "One per layout, or empty: dvorak, colemak, intl…",
                        ui::input(c, "", &self.variant)
                            .on_input(Msg::Variant)
                            .on_submit(Msg::ApplyLayout)
                            .font(MONO)
                            .width(200)
                    ),
                    ui::setting(
                        c,
                        "Options",
                        "xkb options, comma-separated",
                        ui::input(c, "caps:escape", &self.options)
                            .on_input(Msg::Options)
                            .on_submit(Msg::ApplyLayout)
                            .font(MONO)
                            .width(200)
                    ),
                    container(presets.wrap().vertical_spacing(8)).padding([4, 18]).into(),
                    container(
                        row![
                            ui::fill(),
                            ui::btn(c, "Apply layout", Kind::Primary, Some(Msg::ApplyLayout))
                        ]
                    )
                    .padding([12, 18])
                    .into(),
                ]
            ),
            ui::section(
                c,
                "Typing",
                vec![
                    ui::slider_row(
                        c,
                        "Repeat delay",
                        "How long a key is held before it repeats",
                        150.0..=1000.0,
                        25.0,
                        delay,
                        format!("{delay:.0} ms"),
                        |v| Msg::Drag("input:repeat_delay", v),
                        Some(Msg::Release)
                    ),
                    ui::slider_row(
                        c,
                        "Repeat rate",
                        "Characters a second while held",
                        5.0..=80.0,
                        1.0,
                        rate,
                        format!("{rate:.0}/s"),
                        |v| Msg::Drag("input:repeat_rate", v),
                        Some(Msg::Release)
                    ),
                    ui::setting(
                        c,
                        "Try it",
                        "",
                        ui::input(c, "Type here to test the layout and repeat", &self.test)
                            .on_input(Msg::Test)
                            .width(300)
                    ),
                    ui::toggle_row(
                        c,
                        "Num Lock on at login",
                        "",
                        h.get("input:numlock_by_default").bool(),
                        Msg::Numlock
                    ),
                ]
            ),
            ui::note(
                c,
                Icon::Keyboard,
                c.accent,
                "Super is the Nordlys key. Change or add keys in ~/.config/nordlys/hyprland.lua (Advanced → Your Hyprland Lua)."
            ),
            shortcut_sections,
        ]
        .spacing(26)
        .into()
    }
}

/// `Super Shift S` as key caps.
fn keycaps<'a>(c: crate::theme::C, keys: &str) -> El<'a, Msg> {
    let mut r = row![].spacing(4).align_y(Alignment::Center);
    for (i, part) in keys.split(" · ").enumerate() {
        if i > 0 {
            r = r.push(text("or").size(11).color(c.dim));
        }
        let (keys, tail) = match part.split_once(", ") {
            Some((k, t)) => (k, Some(t)),
            None => (part, None),
        };
        for k in keys.split(' ') {
            r = r.push(
                container(text(k.to_owned()).size(11).font(MONO).color(c.muted))
                    .padding([3, 7])
                    .style(c.segments()),
            );
        }
        if let Some(t) = tail {
            r = r.push(text(t.to_owned()).size(11).color(c.dim));
        }
    }
    r.into()
}

fn tidy(s: &str) -> String {
    s.split(',')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(",")
        .trim_matches(',')
        .to_owned()
}

/// Is `l` one of xkb's layouts? Unknown when xkb's list isn't installed.
fn known_layout(l: &str) -> bool {
    let list = std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.lst");
    match list {
        Err(_) => true,
        Ok(s) => s
            .split("! layout")
            .nth(1)
            .and_then(|rest| rest.split("! ").next())
            .map(|section| {
                section
                    .lines()
                    .any(|line| line.split_whitespace().next() == Some(l))
            })
            .unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn layouts_are_tidied() {
        assert_eq!(super::tidy(" us , de "), "us,de");
        assert_eq!(super::tidy(""), "");
    }
}
