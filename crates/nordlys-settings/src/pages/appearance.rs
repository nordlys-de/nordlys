use super::{PageState, hypr_set, toml_set};
use crate::app::Ctx;
use crate::icons::{Icon, icon};
use crate::store::{self, V, WALLPAPER_MODES};
use crate::sys::{self, job};
use crate::theme::{ACCENTS, C, Kind, SEMIBOLD, alpha, color};
use crate::ui::{self, El};
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Length, Task};
use nordlys_config::{BarItem, Config, Mode, Rgb};
use std::path::PathBuf;

#[derive(Default)]
pub struct State {
    hex: String,
    blur: Option<f32>,
    /// Sliders show where they are while dragged and save on release.
    round: Option<f32>,
    bar_gap: Option<f32>,
    bar_side: Option<f32>,
    top_gap: Option<f32>,
    shelf_gap: Option<f32>,
    wallpaper: Option<PathBuf>,
    wall_mode: String,
    wall_input: String,
    theming: bool,
    /// GTK's text scale, when gsettings can say, and where its slider is
    /// while dragged.
    text: Option<f32>,
    text_drag: Option<f32>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Mode(Mode),
    Accent(Option<u32>),
    Hex(String),
    HexSubmit,
    Blur(f32),
    BlurDone,
    Round(f32),
    RoundDone,
    BarGap(f32),
    BarGapDone,
    BarSide(f32),
    BarSideDone,
    TopGap(f32),
    TopGapDone,
    ShelfGap(f32),
    ShelfGapDone,
    BarItem(BarItem, bool),
    Glass(bool),
    Solid(bool),
    Motion(bool),
    TextRead(Result<f32, String>),
    TextSize(f32),
    TextSizeDone,
    TextSaved(Result<(), String>),
    ThemeApps,
    Themed(Result<String, String>),
    WallInput(String),
    WallApply,
    WallBrowse,
    WallPicked(Result<Option<String>, String>),
    WallMode(&'static str),
    WallDone(Result<(), String>),
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.hex = ctx.toml.config.theme.accent.clone().unwrap_or_default();
        let (path, mode) = store::wallpaper();
        self.wall_input = path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.wallpaper = path;
        self.wall_mode = mode;
        job(text_scale, Msg::TextRead)
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Mode(m) => toml_set(
                ctx,
                "theme",
                "mode",
                match m {
                    Mode::Dark => "dark",
                    Mode::Light => "light",
                },
            ),
            Msg::Accent(None) => {
                if let Err(e) = ctx.toml.unset("theme", "accent") {
                    ctx.fail(e);
                }
                ctx.restyle();
                self.hex.clear();
                self.border(ctx);
            }
            Msg::Accent(Some(v)) => {
                let hex = Rgb::hex_u32(v).hex();
                toml_set(ctx, "theme", "accent", hex.as_str());
                self.hex = hex;
                self.border(ctx);
            }
            Msg::Hex(s) => self.hex = s,
            Msg::HexSubmit => match Rgb::parse(self.hex.trim()) {
                Some(rgb) => {
                    return self.update(
                        Msg::Accent(Some(u32::from_str_radix(&rgb.bare(), 16).unwrap_or(0))),
                        ctx,
                    );
                }
                None => ctx.fail("Write a colour as #RRGGBB, like #8AB4FF"),
            },
            Msg::Blur(v) => self.blur = Some(v),
            Msg::BlurDone => {
                if let Some(v) = self.blur.take() {
                    hypr_set(ctx, "decoration:blur:size", V::Int(v.round() as i64));
                    toml_set(ctx, "theme", "blur", (v * 2.4).round() as i64);
                }
            }
            Msg::Round(v) => self.round = Some(v),
            Msg::RoundDone => {
                if let Some(v) = self.round.take() {
                    // Windows (Hyprland) and the shell share one radius.
                    let v = v.round() as i64;
                    toml_set(ctx, "theme", "roundness", v);
                    hypr_set(ctx, "decoration:rounding", V::Int(v));
                }
            }
            Msg::BarGap(v) => self.bar_gap = Some(v),
            Msg::BarGapDone => {
                if let Some(v) = self.bar_gap.take() {
                    toml_set(ctx, "bar", "gap", v.round() as i64);
                }
            }
            Msg::BarSide(v) => self.bar_side = Some(v),
            Msg::BarSideDone => {
                if let Some(v) = self.bar_side.take() {
                    toml_set(ctx, "bar", "side", v.round() as i64);
                }
            }
            Msg::TopGap(v) => self.top_gap = Some(v),
            Msg::TopGapDone => {
                if let Some(v) = self.top_gap.take() {
                    toml_set(ctx, "gaps", "topbar", v.round() as i64);
                }
            }
            Msg::ShelfGap(v) => self.shelf_gap = Some(v),
            Msg::ShelfGapDone => {
                if let Some(v) = self.shelf_gap.take() {
                    toml_set(ctx, "gaps", "shelf", v.round() as i64);
                }
            }
            Msg::BarItem(item, shown) => {
                let mut hidden = ctx.toml.config.bar.hidden.clone();
                hidden.retain(|h| *h != item);
                if !shown {
                    hidden.push(item);
                }
                let names: toml_edit::Array = hidden.iter().map(|h| bar_item_key(*h)).collect();
                toml_set(ctx, "bar", "hidden", names);
            }
            Msg::Glass(on) => hypr_set(ctx, "decoration:blur:enabled", V::Bool(on)),
            Msg::Solid(on) => toml_set(ctx, "theme", "reduce_transparency", on),
            Msg::Motion(on) => {
                toml_set(ctx, "theme", "reduce_motion", on);
                hypr_set(ctx, "animations:enabled", V::Bool(!on));
            }
            // Without gsettings or its schema there's nothing to offer.
            Msg::TextRead(r) => self.text = r.ok(),
            Msg::TextSize(v) => self.text_drag = Some(v),
            Msg::TextSizeDone => {
                if let Some(v) = self.text_drag.take() {
                    self.text = Some(v);
                    return job(move || set_text_scale(v), Msg::TextSaved);
                }
            }
            Msg::TextSaved(r) => {
                ctx.report(&r, "Text size changed");
                return job(text_scale, Msg::TextRead);
            }
            Msg::ThemeApps => {
                self.theming = true;
                return job(theme_apps, Msg::Themed);
            }
            Msg::Themed(r) => {
                self.theming = false;
                match r {
                    Ok(s) => ctx.done(s),
                    Err(e) => ctx.fail(e),
                }
            }
            Msg::WallInput(s) => self.wall_input = s,
            Msg::WallApply => {
                if self.wall_input.trim().is_empty() {
                    ctx.fail("Give the image's path, or Browse for one");
                    return Task::none();
                }
                let path = PathBuf::from(expand(self.wall_input.trim()));
                let mode = self.wall_mode.clone();
                self.wallpaper = Some(path.clone());
                return job(
                    move || store::set_wallpaper(Some(&path), &mode),
                    Msg::WallDone,
                );
            }
            Msg::WallBrowse => return job(pick_image, Msg::WallPicked),
            Msg::WallPicked(Ok(Some(p))) => {
                self.wall_input = p;
                return self.update(Msg::WallApply, ctx);
            }
            Msg::WallPicked(Ok(None)) => {}
            Msg::WallPicked(Err(e)) => ctx.fail(e),
            Msg::WallMode(m) => {
                self.wall_mode = m.into();
                let mode = m.to_owned();
                return job(move || store::set_wallpaper(None, &mode), Msg::WallDone);
            }
            Msg::WallDone(r) => ctx.report(&r, "Wallpaper changed"),
        }
        Task::none()
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        let theme = &ctx.toml.config.theme;
        let accent_now = ctx.toml.config.palette().accent;
        let mut swatches = row![].spacing(10).align_y(Alignment::Center);
        let unset = theme.accent.is_none();
        swatches = swatches.push(swatch(
            c,
            None,
            "Default",
            unset,
            mode_default(&ctx.toml.config),
        ));
        for (name, v) in ACCENTS {
            let on = !unset && accent_now == Rgb::hex_u32(v);
            swatches = swatches.push(swatch(c, Some(v), name, on, color(Rgb::hex_u32(v))));
        }
        let blur_size = self
            .blur
            .unwrap_or(ctx.hypr.get("decoration:blur:size").int() as f32);
        let glass_on = ctx.hypr.get("decoration:blur:enabled").bool();
        let round_now = self.round.unwrap_or(theme.roundness as f32);
        let gap_now = self.bar_gap.unwrap_or(ctx.toml.config.bar.gap as f32);
        let side_now = self.bar_side.unwrap_or(ctx.toml.config.bar.side as f32);
        let top_gap_now = self.top_gap.unwrap_or(ctx.toml.config.gaps.topbar as f32);
        let shelf_gap_now = self.shelf_gap.unwrap_or(ctx.toml.config.gaps.shelf as f32);

        let mut bar_rows = vec![
            ui::slider_row(
                c,
                "Space below the top bar",
                "Between the top bar and the windows and shelf under it",
                0.0..=24.0,
                1.0,
                gap_now,
                format!("{gap_now:.0} px"),
                Msg::BarGap,
                Some(Msg::BarGapDone),
            ),
            ui::slider_row(
                c,
                "Space at the sides",
                "Margin between the top bar and the screen's left and right edges",
                0.0..=40.0,
                1.0,
                side_now,
                format!("{side_now:.0} px"),
                Msg::BarSide,
                Some(Msg::BarSideDone),
            ),
        ];
        for item in BarItem::ALL {
            let shown = !ctx.toml.config.bar.hidden.contains(&item);
            bar_rows.push(ui::toggle_row(
                c,
                bar_item_title(item),
                bar_item_detail(item),
                shown,
                move |on| Msg::BarItem(item, on),
            ));
        }

        let mut app_rows = vec![ui::setting(
            c,
            "Theme GTK and Qt apps",
            "Write the palette for GTK 3/4, libadwaita, qt6ct and foot. Restart apps to see it.",
            ui::btn(
                c,
                if self.theming {
                    "Theming…"
                } else {
                    "Apply to apps"
                },
                Kind::Primary,
                (!self.theming).then_some(Msg::ThemeApps),
            ),
        )];
        if let Some(set) = self.text {
            let size = self.text_drag.unwrap_or(set);
            app_rows.push(ui::slider_row(
                c,
                "Text size in apps",
                "GTK and libadwaita apps follow it. The top bar and panels don't",
                0.8..=1.6,
                0.05,
                size,
                format!("{:.0}%", size * 100.0),
                Msg::TextSize,
                Some(Msg::TextSizeDone),
            ));
        }

        column![
            preview(c),
            ui::section(
                c,
                "Style",
                vec![
                    ui::setting(
                        c,
                        "Mode",
                        "The shell, Settings and themed apps follow it",
                        ui::segmented(
                            c,
                            &[(Mode::Light, "Light"), (Mode::Dark, "Dark")],
                            theme.mode,
                            Msg::Mode
                        )
                    ),
                    ui::slider_row(
                        c,
                        "Roundness",
                        "Corners of windows, the top bar, panels and the shelf",
                        0.0..=24.0,
                        1.0,
                        round_now,
                        format!("{round_now:.0} px"),
                        Msg::Round,
                        Some(Msg::RoundDone)
                    ),
                    column![
                        ui::setting(
                            c,
                            "Accent colour",
                            "Highlights, toggles, the focused window's border",
                            row![
                                ui::input(c, "#RRGGBB", &self.hex)
                                    .on_input(Msg::Hex)
                                    .on_submit(Msg::HexSubmit)
                                    .width(110),
                                ui::btn(c, "Use", Kind::Secondary, Some(Msg::HexSubmit))
                            ]
                            .spacing(8)
                        ),
                        container(swatches).padding(iced::Padding::from([0, 18]).bottom(16)),
                    ]
                    .into(),
                ]
            ),
            ui::section(
                c,
                "Glass",
                vec![
                    ui::toggle_row(
                        c,
                        "Frosted glass",
                        "Blur what's behind the bar, rails, launcher and see-through windows",
                        glass_on,
                        Msg::Glass
                    ),
                    ui::slider_row(
                        c,
                        "Blur strength",
                        "How soft the glass is",
                        1.0..=20.0,
                        1.0,
                        blur_size,
                        format!("{blur_size:.0}"),
                        Msg::Blur,
                        Some(Msg::BlurDone)
                    ),
                    ui::toggle_row(
                        c,
                        "Solid surfaces",
                        "No see-through panels: easier to read, lighter on the GPU",
                        theme.reduce_transparency,
                        Msg::Solid
                    ),
                    ui::toggle_row(
                        c,
                        "Reduce motion",
                        "No window animations and no shake on a wrong password",
                        theme.reduce_motion,
                        Msg::Motion
                    ),
                ]
            ),
            ui::section(
                c,
                "Screen gaps",
                vec![
                    ui::slider_row(
                        c,
                        "Top bar from the top edge",
                        "Space above the top bar, between it and the screen's top edge",
                        0.0..=40.0,
                        1.0,
                        top_gap_now,
                        format!("{top_gap_now:.0} px"),
                        Msg::TopGap,
                        Some(Msg::TopGapDone),
                    ),
                    ui::slider_row(
                        c,
                        "Shelf from the left edge",
                        "Space between the shelf and the screen's left edge",
                        0.0..=40.0,
                        1.0,
                        shelf_gap_now,
                        format!("{shelf_gap_now:.0} px"),
                        Msg::ShelfGap,
                        Some(Msg::ShelfGapDone),
                    ),
                ]
            ),
            ui::section(c, "Top bar", bar_rows),
            ui::section(
                c,
                "Wallpaper",
                vec![
                    ui::setting(
                        c,
                        "Image",
                        self.wallpaper
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "The Nordlys aurora".into()),
                        row![
                            ui::input(c, "/path/to/image.jpg", &self.wall_input)
                                .on_input(Msg::WallInput)
                                .on_submit(Msg::WallApply)
                                .width(260),
                            ui::icon_text_btn(
                                c,
                                Icon::Folder,
                                "Browse",
                                Kind::Secondary,
                                Some(Msg::WallBrowse)
                            ),
                        ]
                        .spacing(8)
                    ),
                    ui::setting(
                        c,
                        "Fit",
                        "How the image covers the screen",
                        ui::segmented(
                            c,
                            &[
                                (WALLPAPER_MODES[0], "Fill"),
                                (WALLPAPER_MODES[1], "Fit"),
                                (WALLPAPER_MODES[2], "Centre"),
                                (WALLPAPER_MODES[3], "Tile")
                            ],
                            WALLPAPER_MODES
                                .into_iter()
                                .find(|m| *m == self.wall_mode)
                                .unwrap_or("fill"),
                            Msg::WallMode
                        )
                    ),
                ]
            ),
            ui::section(c, "Apps", app_rows),
        ]
        .spacing(26)
        .into()
    }
}

const TEXT_SCHEMA: &str = "org.gnome.desktop.interface";
const TEXT_KEY: &str = "text-scaling-factor";

fn text_scale() -> Result<f32, String> {
    sys::run("gsettings", &["get", TEXT_SCHEMA, TEXT_KEY])?
        .trim()
        .parse()
        .map_err(|_| "gsettings didn't give a text size".to_owned())
}

fn set_text_scale(v: f32) -> Result<(), String> {
    let v = format!("{v:.2}");
    sys::run("gsettings", &["set", TEXT_SCHEMA, TEXT_KEY, &v]).map(drop)
}

impl State {
    /// The focused window's border follows the accent.
    fn border(&self, ctx: &mut Ctx) {
        let a = ctx.toml.config.palette().accent;
        hypr_set(
            ctx,
            "general:col.active_border",
            V::Str(format!("rgba({}cc)", a.bare().to_lowercase())),
        );
    }
}

/// The key a part of the bar is written under in `[bar] hidden`.
fn bar_item_key(item: BarItem) -> &'static str {
    match item {
        BarItem::Apps => "apps",
        BarItem::Workspaces => "workspaces",
        BarItem::Stage => "stage",
        BarItem::Clock => "clock",
        BarItem::Recording => "recording",
        BarItem::Media => "media",
        BarItem::Notifications => "notifications",
        BarItem::Tray => "tray",
        BarItem::Layout => "layout",
        BarItem::Indicators => "indicators",
        BarItem::Battery => "battery",
    }
}

fn bar_item_title(item: BarItem) -> &'static str {
    match item {
        BarItem::Apps => "Apps button",
        BarItem::Workspaces => "Workspaces",
        BarItem::Stage => "Stage name",
        BarItem::Clock => "Clock",
        BarItem::Recording => "Recording timer",
        BarItem::Media => "Now playing",
        BarItem::Notifications => "Notifications bell",
        BarItem::Tray => "Tray icons",
        BarItem::Layout => "Layout chip",
        BarItem::Indicators => "Network, Bluetooth and sound",
        BarItem::Battery => "Battery",
    }
}

/// What a part of the bar shows, for its row in the list.
fn bar_item_detail(item: BarItem) -> &'static str {
    match item {
        BarItem::Apps => "The launcher button at the left end",
        BarItem::Workspaces => "The numbered workspaces",
        BarItem::Stage => "The name of the stage you're on",
        BarItem::Clock => "The time and date",
        BarItem::Recording => "How long the screen has been recording",
        BarItem::Media => "The song playing, with its controls",
        BarItem::Notifications => "The bell, with what's waiting",
        BarItem::Tray => "Icons of apps running in the background",
        BarItem::Layout => "The keyboard layout, when there's more than one",
        BarItem::Indicators => "Wi-Fi, Bluetooth and volume at a glance",
        BarItem::Battery => "Charge and time left",
    }
}

fn mode_default(cfg: &Config) -> iced::Color {
    let p = match cfg.theme.mode {
        Mode::Dark => nordlys_config::Palette::dark(),
        Mode::Light => nordlys_config::Palette::light(),
    };
    color(p.accent)
}

fn swatch<'a>(c: C, value: Option<u32>, name: &'a str, on: bool, fill: iced::Color) -> El<'a, Msg> {
    let dot = container(if on {
        El::from(icon(
            Icon::Check,
            14.0,
            if fill.r * 0.299 + fill.g * 0.587 + fill.b * 0.114 > 0.6 {
                iced::Color::from_rgb8(10, 13, 19)
            } else {
                iced::Color::WHITE
            },
        ))
    } else {
        El::from(iced::widget::Space::new())
    })
    .center(30)
    .style(c.swatch(fill, on));
    iced::widget::tooltip(
        button(dot)
            .padding(2)
            .style(c.button(Kind::Ghost))
            .on_press(Msg::Accent(value)),
        container(text(name).size(12))
            .padding([4, 8])
            .style(c.toast(c.border)),
        iced::widget::tooltip::Position::Bottom,
    )
    .into()
}

/// A small picture of the desktop in the current colours.
fn preview<'a>(c: C) -> El<'a, Msg> {
    let glass = move |w: f32, h: f32| {
        container(iced::widget::Space::new().width(w).height(h)).style(move |_| container::Style {
            background: Some(iced::Background::Color(alpha(
                c.surface,
                if c.glass { 0.6 } else { 1.0 },
            ))),
            border: iced::Border {
                radius: 8.0.into(),
                width: 1.0,
                color: alpha(c.text, 0.12),
            },
            ..container::Style::default()
        })
    };
    let window = container(
        column![
            row![
                container(iced::widget::Space::new().width(40).height(6)).style(move |_| {
                    container::Style {
                        background: Some(iced::Background::Color(c.accent)),
                        border: iced::Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                }),
                ui::fill(),
            ],
            container(iced::widget::Space::new().width(Length::Fill).height(6)).style(move |_| {
                container::Style {
                    background: Some(iced::Background::Color(alpha(c.text, 0.15))),
                    border: iced::Border {
                        radius: 3.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
            container(
                iced::widget::Space::new()
                    .width(Length::FillPortion(2))
                    .height(6)
            )
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(alpha(c.text, 0.10))),
                border: iced::Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        ]
        .spacing(8)
        .padding(14),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_| container::Style {
        background: Some(iced::Background::Color(c.surface)),
        border: iced::Border {
            radius: 10.0.into(),
            width: 1.5,
            color: c.accent,
        },
        ..Default::default()
    });
    let desk = column![
        glass(0.0, 14.0).width(Length::Fill),
        row![
            glass(22.0, 0.0).height(Length::Fill),
            window,
            glass(22.0, 0.0).height(Length::Fill)
        ]
        .spacing(8)
        .height(Length::Fill),
    ]
    .spacing(8)
    .padding(12);
    container(
        row![
            container(desk)
                .width(300)
                .height(170)
                .style(move |_| container::Style {
                    background: Some(c.wallpaper()),
                    border: iced::Border {
                        radius: 14.0.into(),
                        width: 1.0,
                        color: c.border,
                    },
                    ..Default::default()
                }),
            column![
                text("Nordlys").font(SEMIBOLD).size(18).color(c.text),
                text(if c.dark {
                    "Dark, with frosted glass"
                } else {
                    "Light, with frosted glass"
                })
                .size(13)
                .color(c.muted),
                ui::gap(6.0),
                row![
                    ui::chip(c, "Accent", c.accent),
                    ui::chip(c, if c.glass { "Glass" } else { "Solid" }, c.muted)
                ]
                .spacing(8),
            ]
            .spacing(4)
        ]
        .spacing(24)
        .align_y(Alignment::Center),
    )
    .padding(18)
    .width(Length::Fill)
    .style(c.card())
    .into()
}

fn expand(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => sys::home().join(rest).to_string_lossy().into_owned(),
        None => p.to_owned(),
    }
}

/// Ask for an image with whichever picker is installed.
fn pick_image() -> Result<Option<String>, String> {
    let pictures = sys::home().join("Pictures");
    let start = format!("{}/", pictures.to_string_lossy());
    let tries: [(&str, Vec<&str>); 2] = [
        (
            "zenity",
            vec![
                "--file-selection",
                "--title=Choose a wallpaper",
                "--file-filter=Images | *.jpg *.jpeg *.png *.webp *.bmp",
                "--filename",
                &start,
            ],
        ),
        (
            "kdialog",
            vec![
                "--getopenfilename",
                &start,
                "image/jpeg image/png image/webp",
            ],
        ),
    ];
    for (tool, args) in tries {
        if sys::has(tool) {
            return match sys::run_for(std::time::Duration::from_secs(600), tool, &args) {
                Ok(p) if !p.trim().is_empty() => Ok(Some(p.trim().to_owned())),
                _ => Ok(None),
            };
        }
    }
    Err("No file picker found (install zenity); type the image's path instead".into())
}

fn theme_apps() -> Result<String, String> {
    sys::run_for(
        std::time::Duration::from_secs(20),
        &sys::sibling("nordlys-theme"),
        &[],
    )
    .map(|_| "GTK, Qt and foot now use the Nordlys palette".to_owned())
}
