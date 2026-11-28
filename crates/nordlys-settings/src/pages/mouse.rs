use super::{PageState, hypr_set};
use crate::app::Ctx;
use crate::store::{V, nordlys::tidy};
use crate::sys::{self, job};
use crate::ui::{self, El};
use iced::Task;
use iced::widget::column;

#[derive(Default)]
pub struct State {
    drag: Option<(&'static str, f32)>,
    /// Asked once when the page opens; unknown until then.
    touchpad: Option<bool>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Drag(&'static str, f32),
    Release,
    Toggle(&'static str, bool),
    Accel(&'static str),
    Touchpad(bool),
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        // When Hyprland can't say, show the settings rather than hide them.
        job(
            || {
                Ok(sys::run("hyprctl", &["devices"])
                    .map(|s| s.to_lowercase().contains("touchpad"))
                    .unwrap_or(true))
            },
            |r| Msg::Touchpad(r.unwrap_or(true)),
        )
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Drag(k, v) => self.drag = Some((k, v)),
            Msg::Release => {
                if let Some((k, v)) = self.drag.take() {
                    let value = if k == "cursor:size" {
                        V::Int(v.round() as i64)
                    } else {
                        V::Float(tidy(v))
                    };
                    hypr_set(ctx, k, value);
                }
            }
            Msg::Toggle(k, on) => hypr_set(ctx, k, V::Bool(on)),
            Msg::Accel(p) => hypr_set(ctx, "input:accel_profile", V::Str(p.into())),
            Msg::Touchpad(found) => self.touchpad = Some(found),
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let h = &ctx.hypr;
        let val = |k: &'static str| match self.drag {
            Some((d, v)) if d == k => v,
            _ => h.get(k).float() as f32,
        };
        let toggle = |k: &'static str, title: &'static str, detail: &'static str| {
            ui::toggle_row(c, title, detail, h.get(k).bool(), move |on| {
                Msg::Toggle(k, on)
            })
        };
        let speed = val("input:sensitivity");
        let cursor_size = match self.drag {
            Some(("cursor:size", v)) => v,
            _ => h.get("cursor:size").int() as f32,
        };
        let scroll = val("input:scroll_factor");
        let tp_scroll = val("input:touchpad:scroll_factor");
        let accel = h.get("input:accel_profile").str();
        let accel = match accel.as_str() {
            "flat" => "flat",
            "adaptive" => "adaptive",
            _ => "",
        };
        let mut body = column![
            ui::section(
                c,
                "Pointer",
                vec![
                    ui::slider_row(
                        c,
                        "Speed",
                        "Slower to the left, faster to the right",
                        -1.0..=1.0,
                        0.05,
                        speed,
                        format!("{speed:+.2}"),
                        |v| Msg::Drag("input:sensitivity", v),
                        Some(Msg::Release),
                    ),
                    ui::slider_row(
                        c,
                        "Cursor size",
                        "Pixels; the pointer's picture is drawn at this size",
                        16.0..=64.0,
                        1.0,
                        cursor_size,
                        format!("{cursor_size:.0} px"),
                        |v| Msg::Drag("cursor:size", v),
                        Some(Msg::Release),
                    ),
                    ui::setting(
                        c,
                        "Acceleration",
                        "Flat moves the pointer exactly as far as the mouse, at any speed",
                        ui::segmented(
                            c,
                            &[("", "Default"), ("adaptive", "Adaptive"), ("flat", "Flat")],
                            accel,
                            Msg::Accel,
                        ),
                    ),
                    toggle(
                        "input:left_handed",
                        "Left-handed",
                        "Swap the left and right buttons"
                    ),
                ],
            ),
            ui::section(
                c,
                "Scrolling",
                vec![
                    toggle(
                        "input:natural_scroll",
                        "Natural scrolling (mouse)",
                        "Content follows the wheel, like a touchscreen"
                    ),
                    ui::slider_row(
                        c,
                        "Scroll speed (mouse)",
                        "How far each wheel notch scrolls",
                        0.2..=3.0,
                        0.1,
                        scroll,
                        format!("{scroll:.1}×"),
                        |v| Msg::Drag("input:scroll_factor", v),
                        Some(Msg::Release),
                    ),
                ],
            )
        ]
        .spacing(26);
        if self.touchpad == Some(true) {
            body = body.push(ui::section(
                c,
                "Touchpad",
                vec![
                    toggle(
                        "input:touchpad:tap-to-click",
                        "Tap to click",
                        "A light tap counts as a click",
                    ),
                    toggle(
                        "input:touchpad:tap-and-drag",
                        "Tap and drag",
                        "With tap to click on: tap, then touch again and move to drag",
                    ),
                    toggle(
                        "input:touchpad:natural_scroll",
                        "Natural scrolling",
                        "Content follows your fingers",
                    ),
                    toggle(
                        "input:touchpad:disable_while_typing",
                        "Ignore while typing",
                        "So a palm doesn't move the pointer",
                    ),
                    toggle(
                        "input:touchpad:clickfinger_behavior",
                        "Click with fingers",
                        "Two fingers right-click and three middle-click, anywhere on the pad",
                    ),
                    toggle(
                        "input:touchpad:middle_button_emulation",
                        "Middle click with both buttons",
                        "Pressing left and right together counts as a middle click",
                    ),
                    ui::slider_row(
                        c,
                        "Scroll speed",
                        "How far a two-finger swipe scrolls",
                        0.2..=3.0,
                        0.1,
                        tp_scroll,
                        format!("{tp_scroll:.1}×"),
                        |v| Msg::Drag("input:touchpad:scroll_factor", v),
                        Some(Msg::Release),
                    ),
                ],
            ));
        }
        body.into()
    }
}
