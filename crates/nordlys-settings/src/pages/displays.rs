use super::{PageState, Reads};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::store::hypr::MonitorRule;
use crate::sys::{self, display, job};
use crate::theme::{Kind, MONO, SEMIBOLD};
use crate::ui::{self, El};
use iced::widget::{column, container, pick_list, row, text};
use iced::{Alignment, Length, Task};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a changed screen waits for "Keep" before going back.
const KEEP_WITHIN: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct State {
    data: Option<display::Displays>,
    reads: Reads,
    drafts: HashMap<String, Draft>,
    pending: Option<Pending>,
    brightness: Option<(String, f32)>,
    night_k: Option<f32>,
    failed: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Draft {
    mode: String,
    scale: String,
    transform: u8,
    vrr: bool,
    enabled: bool,
    x: String,
    y: String,
}

struct Pending {
    name: String,
    before: MonitorRule,
    after: MonitorRule,
    since: Instant,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<display::Displays, String>),
    Mode(String, String),
    Scale(String, String),
    Transform(String, u8),
    Vrr(String, bool),
    Enabled(String, bool),
    X(String, String),
    Y(String, String),
    Apply(String),
    Keep,
    Revert,
    Brightness(String, f32),
    BrightnessDone,
    Night(bool),
    NightK(f32),
    NightKDone,
    Done(Result<(), String>),
}

const SCALES: [&str; 8] = ["1", "1.25", "1.333333", "1.5", "1.6", "1.75", "2", "3"];

fn rule_of(m: &display::Monitor) -> MonitorRule {
    MonitorRule {
        mode: m.mode(),
        position: format!("{}x{}", m.x, m.y),
        scale: m.scale as f64,
        transform: m.transform % 4,
        vrr: m.vrr,
        disabled: m.disabled,
    }
}

fn draft_of(m: &display::Monitor) -> Draft {
    Draft {
        mode: m.mode(),
        scale: trim(m.scale as f64),
        transform: m.transform % 4,
        vrr: m.vrr,
        enabled: !m.disabled,
        x: m.x.to_string(),
        y: m.y.to_string(),
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

impl Draft {
    fn rule(&self) -> Result<MonitorRule, String> {
        let scale: f64 = self
            .scale
            .parse()
            .map_err(|_| "Scale is a number like 1.5".to_owned())?;
        if !(0.25..=4.0).contains(&scale) {
            return Err("Scale goes from 0.25 to 4".into());
        }
        let x: i32 = self
            .x
            .trim()
            .parse()
            .map_err(|_| "Position is in whole pixels".to_owned())?;
        let y: i32 = self
            .y
            .trim()
            .parse()
            .map_err(|_| "Position is in whole pixels".to_owned())?;
        Ok(MonitorRule {
            mode: self.mode.clone(),
            position: format!("{x}x{y}"),
            scale,
            transform: self.transform,
            vrr: self.vrr,
            disabled: !self.enabled,
        })
    }
}

impl State {
    fn read(&mut self) -> Task<Msg> {
        if !self.reads.start() {
            return Task::none();
        }
        job(display::read, Msg::Read)
    }

    fn draft(&mut self, name: &str, f: impl FnOnce(&mut Draft)) {
        if let Some(d) = self.drafts.get_mut(name) {
            f(d);
        }
    }

    fn revert(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let Some(p) = self.pending.take() else {
            return Task::none();
        };
        let r = ctx.hypr.apply_monitor(&p.name, &p.before);
        ctx.report(&r, "Went back to the screen's old settings");
        self.drafts.remove(&p.name);
        self.read()
    }
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        self.read()
    }

    fn tick(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.since.elapsed() >= KEEP_WITHIN)
        {
            return self.revert(ctx);
        }
        if self.brightness.is_some() || self.pending.is_some() {
            return Task::none();
        }
        self.read()
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Read(r) => {
                self.reads.done();
                match r {
                    Ok(d) => {
                        for m in &d.monitors {
                            // Keep what's being edited; refresh the rest.
                            let fresh = draft_of(m);
                            let old = self
                                .data
                                .as_ref()
                                .and_then(|o| o.monitors.iter().find(|x| x.name == m.name));
                            let untouched = match (old, self.drafts.get(&m.name)) {
                                (Some(o), Some(dr)) => *dr == draft_of(o),
                                _ => true,
                            };
                            if untouched {
                                self.drafts.insert(m.name.clone(), fresh);
                            }
                        }
                        self.data = Some(d);
                        self.failed = None;
                    }
                    Err(e) => self.failed = Some(e),
                }
            }
            Msg::Mode(n, v) => self.draft(&n, |d| d.mode = v),
            Msg::Scale(n, v) => self.draft(&n, |d| d.scale = v),
            Msg::Transform(n, v) => self.draft(&n, |d| d.transform = v),
            Msg::Vrr(n, v) => self.draft(&n, |d| d.vrr = v),
            Msg::Enabled(n, on) => {
                let others_on = self
                    .data
                    .as_ref()
                    .map(|d| {
                        d.monitors
                            .iter()
                            .filter(|m| m.name != n && !m.disabled)
                            .count()
                    })
                    .unwrap_or(0);
                if !on && others_on == 0 {
                    ctx.fail(
                        "That's the only screen on; turning it off would leave you nothing to see",
                    );
                } else {
                    self.draft(&n, |d| d.enabled = on);
                }
            }
            Msg::X(n, v) => self.draft(&n, |d| d.x = v),
            Msg::Y(n, v) => self.draft(&n, |d| d.y = v),
            Msg::Apply(n) => {
                let (Some(d), Some(m)) = (
                    self.drafts.get(&n),
                    self.data
                        .as_ref()
                        .and_then(|x| x.monitors.iter().find(|m| m.name == n)),
                ) else {
                    return Task::none();
                };
                let before = ctx.hypr.monitor(&n).cloned().unwrap_or_else(|| rule_of(m));
                match d.rule() {
                    Err(e) => ctx.fail(e),
                    Ok(after) => match ctx.hypr.apply_monitor(&n, &after) {
                        Err(e) => ctx.fail(e),
                        Ok(()) => {
                            self.pending = Some(Pending {
                                name: n,
                                before,
                                after,
                                since: Instant::now(),
                            });
                            return self.read();
                        }
                    },
                }
            }
            Msg::Keep => {
                if let Some(p) = self.pending.take() {
                    let r = ctx.hypr.keep_monitor(&p.name, p.after);
                    ctx.report(&r, &format!("{} saved", p.name));
                }
            }
            Msg::Revert => return self.revert(ctx),
            Msg::Brightness(dev, v) => {
                self.brightness = Some((dev.clone(), v));
                let p = v.round() as u8;
                // A failed brightness change is reported, not dropped.
                return job(move || display::set_brightness(&dev, p), Msg::Done);
            }
            Msg::BrightnessDone => {
                self.brightness = None;
                return self.read();
            }
            Msg::Night(on) => {
                let k = on.then(|| self.night_k.unwrap_or(4500.0) as u32);
                return job(move || display::set_night_light(k), Msg::Done);
            }
            Msg::NightK(v) => self.night_k = Some(v),
            Msg::NightKDone => {
                if self.data.as_ref().is_some_and(|d| d.night_light.is_some()) {
                    let k = self.night_k.unwrap_or(4500.0) as u32;
                    return job(move || display::set_night_light(Some(k)), Msg::Done);
                }
            }
            Msg::Done(r) => {
                if let Err(e) = r {
                    ctx.fail(e);
                }
                return self.read();
            }
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        if let Some(e) = &self.failed {
            return ui::note(c, Icon::CircleAlert, c.warn, e.clone());
        }
        let Some(d) = &self.data else {
            return ui::loading(c);
        };
        let mut body = column![].spacing(26);
        if let Some(p) = &self.pending {
            let left = KEEP_WITHIN.saturating_sub(p.since.elapsed()).as_secs();
            body = body.push(ui::note_with(
                c,
                Icon::Monitor,
                c.accent,
                format!(
                    "Keep these settings for {}? Going back in {left} s.",
                    p.name
                ),
                row![
                    ui::btn(c, "Go back", Kind::Secondary, Some(Msg::Revert)),
                    ui::btn(c, "Keep", Kind::Primary, Some(Msg::Keep)),
                ]
                .spacing(8)
                .into(),
            ));
        }
        if d.monitors.is_empty() {
            body = body.push(ui::note(
                c,
                Icon::Info,
                c.accent,
                if sys::on_hyprland() {
                    "Hyprland reports no screens."
                } else {
                    "Screens can be arranged on Hyprland; this session isn't one."
                },
            ));
        }
        if d.monitors.len() > 1 {
            body = body.push(arrangement(c, &d.monitors));
        }
        for m in &d.monitors {
            let Some(dr) = self.drafts.get(&m.name) else {
                continue;
            };
            let n = m.name.clone();
            let changed = *dr != draft_of(m);
            let mut modes = m.modes();
            if !modes.contains(&dr.mode) {
                modes.insert(0, dr.mode.clone());
            }
            let mut scales: Vec<String> = SCALES.iter().map(|s| (*s).to_owned()).collect();
            if !scales.contains(&dr.scale) {
                scales.insert(0, dr.scale.clone());
            }
            let title = if m.description.is_empty() {
                m.name.clone()
            } else {
                format!("{} · {}", m.name, m.description)
            };
            let inches = m
                .diagonal()
                .map(|i| format!(" · {i:.0}″"))
                .unwrap_or_default();
            let logical = if m.disabled || m.width == 0 {
                "Off".to_owned()
            } else {
                format!(
                    "{}×{} looks like {:.0}×{:.0}{inches}",
                    m.width,
                    m.height,
                    m.width as f32 / m.scale.max(0.1),
                    m.height as f32 / m.scale.max(0.1)
                )
            };
            let mut rows = vec![ui::icon_setting(
                c,
                Icon::Monitor,
                if m.focused { c.accent } else { c.muted },
                title,
                logical,
                ui::toggle(dr.enabled, {
                    let n = n.clone();
                    move |on| Msg::Enabled(n.clone(), on)
                }),
            )];
            if dr.enabled {
                rows.extend([
                    ui::setting(
                        c,
                        "Resolution and refresh rate",
                        "",
                        pick_list(modes, Some(dr.mode.clone()), {
                            let n = n.clone();
                            move |v| Msg::Mode(n.clone(), v)
                        })
                        .padding([6, 12])
                        .text_size(13)
                        .font(MONO)
                        .width(200),
                    ),
                    ui::setting(
                        c,
                        "Scale",
                        "Bigger text and controls on dense screens",
                        pick_list(scales, Some(dr.scale.clone()), {
                            let n = n.clone();
                            move |v| Msg::Scale(n.clone(), v)
                        })
                        .padding([6, 12])
                        .text_size(13)
                        .width(120),
                    ),
                    ui::setting(
                        c,
                        "Rotation",
                        "",
                        ui::segmented(
                            c,
                            &[
                                (0u8, display::TRANSFORMS[0]),
                                (1, display::TRANSFORMS[1]),
                                (2, display::TRANSFORMS[2]),
                                (3, display::TRANSFORMS[3]),
                            ],
                            dr.transform,
                            {
                                let n = n.clone();
                                move |v| Msg::Transform(n.clone(), v)
                            },
                        ),
                    ),
                    ui::setting(
                        c,
                        "Position",
                        "Top-left corner, in pixels, among your screens",
                        row![
                            ui::input(c, "x", &dr.x)
                                .on_input({
                                    let n = n.clone();
                                    move |v| Msg::X(n.clone(), v)
                                })
                                .font(MONO)
                                .width(80),
                            text("×").color(c.dim),
                            ui::input(c, "y", &dr.y)
                                .on_input({
                                    let n = n.clone();
                                    move |v| Msg::Y(n.clone(), v)
                                })
                                .font(MONO)
                                .width(80),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    ),
                    ui::toggle_row(
                        c,
                        "Variable refresh rate",
                        "Smoother games and video on screens with FreeSync or G-Sync",
                        dr.vrr,
                        {
                            let n = n.clone();
                            move |on| Msg::Vrr(n.clone(), on)
                        },
                    ),
                ]);
            }
            rows.push(
                container(
                    row![
                        text(if ctx.hypr.monitor(&m.name).is_some() {
                            "Saved for every login"
                        } else {
                            "Using the screen's preferred settings"
                        })
                        .size(12)
                        .color(c.dim),
                        ui::fill(),
                        ui::btn(
                            c,
                            "Apply",
                            Kind::Primary,
                            (changed && self.pending.is_none()).then(|| Msg::Apply(n.clone()))
                        ),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding([12, 18])
                .into(),
            );
            body = body.push(ui::section(c, "", rows));
        }

        let mut light = vec![];
        for b in &d.backlights {
            let v = match &self.brightness {
                Some((dev, v)) if *dev == b.device => *v,
                _ => b.percent as f32,
            };
            let dev = b.device.clone();
            light.push(ui::slider_row(
                c,
                "Brightness",
                b.device.clone(),
                1.0..=100.0,
                1.0,
                v,
                format!("{v:.0}%"),
                move |x| Msg::Brightness(dev.clone(), x),
                Some(Msg::BrightnessDone),
            ));
        }
        if d.can_night_light {
            let k = self.night_k.unwrap_or(d.night_light.unwrap_or(4500) as f32);
            light.push(ui::icon_setting(
                c,
                Icon::MoonStar,
                c.warn,
                "Night light",
                "Warmer colours, easier on the eyes in the evening",
                ui::toggle(d.night_light.is_some(), Msg::Night),
            ));
            light.push(ui::slider_row(
                c,
                "Warmth",
                "Lower is warmer",
                2500.0..=6500.0,
                100.0,
                k,
                format!("{k:.0} K"),
                Msg::NightK,
                Some(Msg::NightKDone),
            ));
        } else {
            light.push(ui::setting(
                c,
                "Night light",
                "Install hyprsunset to warm the screen's colours in the evening",
                ui::chip(c, "Not installed", c.dim),
            ));
        }
        if d.backlights.is_empty() {
            light.insert(0, ui::empty(c, "No built-in backlight. External screens set brightness with their own buttons."));
        }
        body.push(ui::section(c, "Brightness and colour", light))
            .into()
    }
}

/// The screens drawn to scale where they sit.
fn arrangement<'a>(c: crate::theme::C, monitors: &'a [display::Monitor]) -> El<'a, Msg> {
    let on: Vec<&display::Monitor> = monitors.iter().filter(|m| !m.disabled).collect();
    let size = |m: &display::Monitor| {
        let (w, h) = (
            m.width as f32 / m.scale.max(0.1),
            m.height as f32 / m.scale.max(0.1),
        );
        if m.transform % 2 == 1 { (h, w) } else { (w, h) }
    };
    let min_x = on.iter().map(|m| m.x).min().unwrap_or(0) as f32;
    let min_y = on.iter().map(|m| m.y).min().unwrap_or(0) as f32;
    let max_x = on
        .iter()
        .map(|m| m.x as f32 + size(m).0)
        .fold(1.0, f32::max);
    let max_y = on
        .iter()
        .map(|m| m.y as f32 + size(m).1)
        .fold(1.0, f32::max);
    let (span_w, span_h) = (max_x - min_x, max_y - min_y);
    let k = (640.0 / span_w).min(180.0 / span_h);
    let mut layers = iced::widget::Stack::new()
        .width(span_w * k)
        .height(span_h * k);
    for m in on {
        let (w, h) = size(m);
        let tile = container(
            column![
                text(m.name.clone())
                    .size(12)
                    .font(SEMIBOLD)
                    .color(if m.focused { c.on_accent } else { c.text }),
                text(format!("{}×{}", m.width, m.height))
                    .size(10)
                    .color(if m.focused { c.on_accent } else { c.muted }),
            ]
            .align_x(Alignment::Center),
        )
        .center_x(w * k - 4.0)
        .center_y(h * k - 4.0)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(if m.focused {
                c.accent
            } else {
                c.raised
            })),
            border: iced::Border {
                radius: 6.0.into(),
                width: 1.0,
                color: c.border,
            },
            ..Default::default()
        });
        layers = layers.push(
            container(tile).padding(
                iced::Padding::ZERO
                    .left((m.x as f32 - min_x) * k + 2.0)
                    .top((m.y as f32 - min_y) * k + 2.0),
            ),
        );
    }
    container(layers)
        .center_x(Length::Fill)
        .padding(20)
        .style(c.card())
        .into()
}
