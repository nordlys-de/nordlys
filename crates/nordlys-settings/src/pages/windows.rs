use super::{PageState, hypr_set, toml_set};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::store::{V, nordlys::tidy};
use crate::ui::{self, El};
use iced::Task;
use iced::widget::column;
use nordlys_config::Layout;

/// Sliders show where they are while dragged and apply on release.
#[derive(Default)]
pub struct State {
    drag: Option<(&'static str, f32)>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Layout(Layout),
    View(&'static str),
    Drag(&'static str, f32),
    Release,
    Toggle(&'static str, bool),
    FollowFocus(bool),
}

/// Hyprland options shown as whole-number sliders: key, title, detail, range.
const SIZES: [(&str, &str, &str, f32, f32); 4] = [
    (
        "general:gaps_in",
        "Gap between windows",
        "Space between windows side by side",
        0.0,
        30.0,
    ),
    (
        "general:gaps_out",
        "Gap at the edges",
        "Space between windows and the screen's edge",
        0.0,
        60.0,
    ),
    (
        "general:border_size",
        "Border",
        "Width of the line around each window",
        0.0,
        6.0,
    ),
    (
        "decoration:rounding",
        "Corner rounding",
        "How round window corners are",
        0.0,
        28.0,
    ),
];

impl PageState for State {
    type Msg = Msg;

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Layout(l) => toml_set(
                ctx,
                "stage",
                "layout",
                match l {
                    Layout::Auto => "auto",
                    Layout::Ultrawide => "ultrawide",
                    Layout::Rails => "rails",
                    Layout::Peek => "peek",
                },
            ),
            Msg::View(v) => hypr_set(ctx, "general:layout", V::Str(v.into())),
            Msg::Drag(k, v) => self.drag = Some((k, v)),
            Msg::Release => match self.drag.take() {
                Some(("split", v)) => {
                    toml_set(ctx, "stage", "split", tidy(v));
                    hypr_set(ctx, "master:mfact", V::Float(tidy(v)));
                }
                Some(("shelf_fps", v)) => toml_set(ctx, "stage", "shelf_fps", v.round() as i64),
                Some(("decoration:inactive_opacity", v)) => {
                    hypr_set(ctx, "decoration:inactive_opacity", V::Float(tidy(v)))
                }
                Some(("cursor:inactive_timeout", v)) => {
                    hypr_set(ctx, "cursor:inactive_timeout", V::Float(v.round() as f64))
                }
                Some((k, v)) => hypr_set(ctx, k, V::Int(v.round() as i64)),
                None => {}
            },
            Msg::Toggle(k, on) => hypr_set(ctx, k, V::Bool(on)),
            Msg::FollowFocus(on) => {
                hypr_set(ctx, "input:follow_mouse", V::Int(if on { 1 } else { 2 }))
            }
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let h = &ctx.hypr;
        let stage = &ctx.toml.config.stage;
        let val = |k: &'static str, saved: f32| match self.drag {
            Some((d, v)) if d == k => v,
            _ => saved,
        };
        let view = h.get("general:layout").str();
        let view = if view == "monocle" {
            "monocle"
        } else if view == "master" {
            "master"
        } else {
            "other"
        };

        let split = val("split", stage.split);
        let fps = val("shelf_fps", stage.shelf_fps as f32);
        let mut sizes = vec![];
        for (k, title, detail, lo, hi) in SIZES {
            let v = val(k, h.get(k).int() as f32);
            sizes.push(ui::slider_row(
                c,
                title,
                detail,
                lo..=hi,
                1.0,
                v,
                format!("{v:.0} px"),
                move |x| Msg::Drag(k, x),
                Some(Msg::Release),
            ));
        }
        let opacity = val(
            "decoration:inactive_opacity",
            h.get("decoration:inactive_opacity").float() as f32,
        );
        let cursor_hide = val(
            "cursor:inactive_timeout",
            h.get("cursor:inactive_timeout").float() as f32,
        );
        let toggle = |k: &'static str, title: &'static str, detail: &'static str| {
            ui::toggle_row(c, title, detail, h.get(k).bool(), move |on| {
                Msg::Toggle(k, on)
            })
        };

        let mut body = column![].spacing(26);
        if !crate::sys::on_hyprland() {
            body = body.push(ui::note(
                c,
                Icon::Info,
                c.accent,
                "Not on Hyprland: window settings are saved and take effect at the next Nordlys login.",
            ));
        }
        body.push(ui::section(
            c,
            "Stage",
            vec![
                ui::setting(
                    c,
                    "Starting view",
                    "Stage: the focused window fills the stage, others wait on the shelf. Tiled: side by side. Super T switches.",
                    ui::segmented(
                        c,
                        &[("monocle", "Stage"), ("master", "Tiled")],
                        view,
                        Msg::View,
                    ),
                ),
                ui::slider_row(
                    c,
                    "Main window's share",
                    "In the tiled view and two-up",
                    0.3..=0.8,
                    0.01,
                    split,
                    format!("{:.0}%", split * 100.0),
                    |x| Msg::Drag("split", x),
                    Some(Msg::Release),
                ),
                ui::setting(
                    c,
                    "Shelf layout",
                    "Auto picks by screen width: ultrawide over 2400 px, rails from 1600 px, peek below",
                    ui::segmented(
                        c,
                        &[
                            (Layout::Auto, "Auto"),
                            (Layout::Ultrawide, "Ultrawide"),
                            (Layout::Rails, "Rails"),
                            (Layout::Peek, "Peek"),
                        ],
                        stage.layout,
                        Msg::Layout,
                    ),
                ),
                ui::slider_row(
                    c,
                    "Shelf thumbnail rate",
                    "How often waiting windows redraw on the shelf (nordlys-comp)",
                    1.0..=30.0,
                    1.0,
                    fps,
                    format!("{fps:.0} fps"),
                    |x| Msg::Drag("shelf_fps", x),
                    Some(Msg::Release),
                ),
            ],
        ))
        .push(ui::section(c, "Spacing and corners", sizes))
        .push(ui::section(
            c,
            "Windows",
            vec![
                toggle("decoration:shadow:enabled", "Shadows", "A soft shadow under each window"),
                toggle(
                    "decoration:dim_inactive",
                    "Dim windows in the background",
                    "Darken every window but the focused one",
                ),
                ui::slider_row(
                    c,
                    "Background window opacity",
                    "See through windows you're not using",
                    0.5..=1.0,
                    0.01,
                    opacity,
                    format!("{:.0}%", opacity * 100.0),
                    |x| Msg::Drag("decoration:inactive_opacity", x),
                    Some(Msg::Release),
                ),
                toggle(
                    "general:resize_on_border",
                    "Resize from the edges",
                    "Drag a window's border to resize it, as well as Super + right-drag",
                ),
                toggle(
                    "misc:focus_on_activate",
                    "Let apps take focus",
                    "An app asking for attention comes to the stage",
                ),
            ],
        ))
        .push(ui::section(
            c,
            "Focus and pointer",
            vec![
                ui::toggle_row(
                    c,
                    "Focus follows the pointer",
                    "Off: focus changes only when you click",
                    h.get("input:follow_mouse").int() == 1,
                    Msg::FollowFocus,
                ),
                toggle(
                    "cursor:hide_on_key_press",
                    "Hide the pointer while typing",
                    "It comes back when the mouse moves",
                ),
                ui::slider_row(
                    c,
                    "Hide an idle pointer",
                    "After this many seconds without moving; 0 never hides it",
                    0.0..=30.0,
                    1.0,
                    cursor_hide,
                    if cursor_hide < 0.5 { "Never".into() } else { format!("{cursor_hide:.0} s") },
                    |x| Msg::Drag("cursor:inactive_timeout", x),
                    Some(Msg::Release),
                ),
            ],
        ))
        .into()
    }
}
