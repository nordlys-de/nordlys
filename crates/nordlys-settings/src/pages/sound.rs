use super::{PageState, Reads};
use crate::app::Ctx;
use crate::icons::Icon;
use crate::sys::{audio, job};
use crate::theme::{Kind, MONO};
use crate::ui::{self, El};
use iced::widget::{column, container, pick_list, row, slider, text};
use iced::{Alignment, Length, Task};

#[derive(Default)]
pub struct State {
    data: Option<audio::Audio>,
    reads: Reads,
    /// A slider being dragged: node and level. Reads don't overwrite it.
    drag: Option<(u32, f32)>,
    sent: Option<std::time::Instant>,
    failed: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<audio::Audio, String>),
    Default(Device),
    Volume(u32, f32),
    Release,
    Mute(u32, bool),
    Test,
    /// Input or not, the device, and its socket.
    Port(bool, String, audio::Choice),
    Profile(String, audio::Choice),
    Done(Result<(), String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    id: u32,
    name: String,
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

impl std::fmt::Display for audio::Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// A picker over `choices` with the one whose key is `active` chosen.
fn picker<'a>(
    choices: &[audio::Choice],
    active: &str,
    on_pick: impl Fn(audio::Choice) -> Msg + 'a,
) -> El<'a, Msg> {
    let chosen = choices.iter().find(|c| c.key == active).cloned();
    pick_list(choices.to_vec(), chosen, on_pick)
        .padding([6, 12])
        .text_size(13)
        .width(320)
        .into()
}

fn devices(nodes: &[audio::Node]) -> Vec<Device> {
    nodes
        .iter()
        .map(|n| Device {
            id: n.id,
            name: n.name.clone(),
        })
        .collect()
}

impl State {
    fn read(&mut self) -> Task<Msg> {
        if self.drag.is_some() || !self.reads.start() {
            return Task::none();
        }
        job(audio::read, Msg::Read)
    }

    fn node_mut(&mut self, id: u32) -> Option<&mut audio::Node> {
        let d = self.data.as_mut()?;
        d.sinks
            .iter_mut()
            .chain(d.sources.iter_mut())
            .chain(d.streams.iter_mut())
            .find(|n| n.id == id)
    }
}

impl PageState for State {
    type Msg = Msg;

    fn open(&mut self, _: &mut Ctx) -> Task<Msg> {
        self.read()
    }

    fn tick(&mut self, _: &mut Ctx) -> Task<Msg> {
        self.read()
    }

    fn update(&mut self, msg: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match msg {
            Msg::Read(r) => {
                self.reads.done();
                match r {
                    Ok(d) => {
                        self.data = Some(d);
                        self.failed = None;
                    }
                    Err(e) => self.failed = Some(e),
                }
                Task::none()
            }
            Msg::Default(d) => {
                let id = d.id;
                job(move || audio::set_default(id), Msg::Done)
            }
            Msg::Volume(id, v) => {
                self.drag = Some((id, v));
                if let Some(n) = self.node_mut(id) {
                    n.volume = v;
                }
                // Change it as the slider moves, so you hear it, but no
                // more than twenty times a second.
                if self
                    .sent
                    .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(50))
                {
                    return Task::none();
                }
                self.sent = Some(std::time::Instant::now());
                job(move || audio::set_volume(id, v), |_| Msg::Done(Ok(())))
            }
            Msg::Release => match self.drag.take() {
                Some((id, v)) => job(move || audio::set_volume(id, v), Msg::Done),
                None => self.read(),
            },
            Msg::Mute(id, muted) => {
                if let Some(n) = self.node_mut(id) {
                    n.muted = muted;
                }
                job(move || audio::set_mute(id, muted), Msg::Done)
            }
            Msg::Test => job(audio::test_sound, Msg::Done),
            Msg::Port(input, device, port) => job(
                move || audio::set_port(input, &device, &port.key),
                Msg::Done,
            ),
            Msg::Profile(card, profile) => {
                job(move || audio::set_profile(&card, &profile.key), Msg::Done)
            }
            Msg::Done(r) => {
                if let Err(e) = r {
                    ctx.fail(e);
                }
                self.read()
            }
        }
    }

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let c = ctx.c;
        if let Some(e) = &self.failed {
            return ui::note(
                c,
                Icon::CircleAlert,
                c.warn,
                format!(
                    "PipeWire isn't answering ({e}). Sound needs pipewire and wireplumber running."
                ),
            );
        }
        let Some(d) = &self.data else {
            return ui::loading(c);
        };
        let level = |n: &audio::Node| -> f32 {
            match self.drag {
                Some((id, v)) if id == n.id => v,
                _ => n.volume,
            }
        };
        let volume_row = |n: &'a audio::Node,
                          title: String,
                          detail: String,
                          ic_on: Icon,
                          ic_off: Icon|
         -> El<'a, Msg> {
            let v = level(n);
            let id = n.id;
            row![
                ui::icon_btn(
                    c,
                    if n.muted || v <= 0.0 { ic_off } else { ic_on },
                    if n.muted { Kind::Danger } else { Kind::Ghost },
                    Some(Msg::Mute(id, !n.muted))
                ),
                column![
                    text(title).size(14).color(c.text),
                    text(detail).size(12).color(c.muted),
                ]
                .spacing(2)
                .width(Length::FillPortion(2)),
                // Up to 150%: a quiet source can be boosted, not only lowered.
                slider(0.0..=1.5, v, move |x| Msg::Volume(id, x))
                    .step(0.01_f32)
                    .on_release(Msg::Release)
                    .width(Length::FillPortion(3)),
                container(
                    text(format!("{:.0}%", v * 100.0))
                        .size(13)
                        .font(MONO)
                        .color(if n.muted { c.dim } else { c.muted })
                )
                .width(52)
                .align_x(Alignment::End),
            ]
            .spacing(14)
            .align_y(Alignment::Center)
            .padding([12, 18])
            .into()
        };

        let socket_row = |ports: &'a audio::Ports, input: bool| -> El<'a, Msg> {
            ui::setting(
                c,
                "Connector",
                if input {
                    "Which microphone on this device"
                } else {
                    "Speakers, headphones or another socket on this device"
                },
                picker(&ports.choices, &ports.active, move |p| {
                    Msg::Port(input, ports.device.clone(), p)
                }),
            )
        };

        let mut out = vec![];
        match d.default_sink() {
            Some(sink) => {
                out.push(ui::setting(
                    c,
                    "Output",
                    "Where sound plays",
                    pick_list(
                        devices(&d.sinks),
                        Some(Device {
                            id: sink.id,
                            name: sink.name.clone(),
                        }),
                        Msg::Default,
                    )
                    .padding([6, 12])
                    .text_size(13)
                    .width(320),
                ));
                if let Some(p) = &d.output_ports {
                    out.push(socket_row(p, false));
                }
                out.push(volume_row(
                    sink,
                    "Volume".into(),
                    if sink.muted {
                        "Muted".into()
                    } else {
                        String::new()
                    },
                    Icon::Volume2,
                    Icon::VolumeX,
                ));
                out.push(ui::setting(
                    c,
                    "Test",
                    "Play a short sound on this output",
                    ui::icon_text_btn(c, Icon::Play, "Play", Kind::Secondary, Some(Msg::Test)),
                ));
            }
            None => out.push(ui::empty(c, "No outputs found.")),
        }
        let mut inp = vec![];
        match d.default_source() {
            Some(src) => {
                inp.push(ui::setting(
                    c,
                    "Input",
                    "The microphone apps hear",
                    pick_list(
                        devices(&d.sources),
                        Some(Device {
                            id: src.id,
                            name: src.name.clone(),
                        }),
                        Msg::Default,
                    )
                    .padding([6, 12])
                    .text_size(13)
                    .width(320),
                ));
                if let Some(p) = &d.input_ports {
                    inp.push(socket_row(p, true));
                }
                inp.push(volume_row(
                    src,
                    "Input level".into(),
                    if src.muted {
                        "Muted".into()
                    } else {
                        String::new()
                    },
                    Icon::Mic,
                    Icon::MicOff,
                ));
            }
            None => inp.push(ui::empty(c, "No microphones found.")),
        }
        let apps: Vec<El<'a, Msg>> = if d.streams.is_empty() {
            vec![ui::empty(c, "No app is playing or recording.")]
        } else {
            d.streams
                .iter()
                .map(|s| {
                    volume_row(
                        s,
                        s.name.clone(),
                        if s.input {
                            "Recording".into()
                        } else {
                            "Playing".into()
                        },
                        if s.input { Icon::Mic } else { Icon::Volume2 },
                        if s.input { Icon::MicOff } else { Icon::VolumeX },
                    )
                })
                .collect()
        };
        let mut body = column![
            ui::section(c, "Output", out),
            ui::section(c, "Input", inp),
            ui::section(c, "Apps", apps),
        ]
        .spacing(26);
        if !d.cards.is_empty() {
            let cards = d
                .cards
                .iter()
                .map(|card| {
                    ui::setting(
                        c,
                        card.label.clone(),
                        if card.name.starts_with("bluez_card") {
                            "Mode: headsets have one for music and one for calls"
                        } else {
                            "Mode: which outputs and inputs it offers"
                        },
                        picker(&card.profiles, &card.active, move |p| {
                            Msg::Profile(card.name.clone(), p)
                        }),
                    )
                })
                .collect();
            body = body.push(ui::section(c, "Sound cards", cards));
        }
        body.into()
    }
}
