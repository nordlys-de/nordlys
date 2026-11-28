//! The bar's network, Bluetooth and sound panels: what the icons open.
//!
//! * Network: Wi-Fi on and off, nearby networks (join, leave), wired
//!   links and VPNs (on and off). A new secured network needs a password,
//!   which the bar can't take (it never has the keyboard), so that opens
//!   Settings on the Network page.
//! * Bluetooth: on and off, and the paired devices (connect, disconnect).
//!   Pairing a new one is in Settings.
//! * Sound: output and input levels and mute, which device is used, and
//!   each sound card's profile (stereo, surround, HDMI, duplex, off).
//!
//! Reads and changes run off the UI thread. While a panel is open the bar
//! reads it again every few seconds, so it follows changes made elsewhere.

pub mod sys;

use crate::bar::{INTER, INTER_SEMIBOLD, color};
use crate::icons::{self};
use iced::alignment::Vertical;
use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, slider, text, toggler,
};
use iced::{Alignment, Background, Border, Color, Element, Length};
use nordlys_config::Palette;
use sys::{audio, bluetooth, network};

pub const WIDTH: f32 = 340.0;
const ROW: u32 = 34;
const MAX_HEIGHT: u32 = 560;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Network,
    Bluetooth,
    Sound,
}

impl Kind {
    fn page(self) -> &'static str {
        match self {
            Kind::Network => "network",
            Kind::Bluetooth => "bluetooth",
            Kind::Sound => "sound",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    Network(network::Network),
    Bluetooth(bluetooth::Bluetooth),
    Sound(audio::Audio, Vec<sys::Card>),
}

/// Something to change, run in the background.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    WifiPower(bool),
    /// Join a saved or open network.
    Join {
        ssid: String,
        security: String,
    },
    /// Leave the Wi-Fi network: disconnect the device.
    Disconnect(String),
    Up(String),
    Down(String),
    Rescan,
    BtPower(bool),
    BtConnect(String),
    BtDisconnect(String),
    Default(u32),
    Volume(u32, f32),
    Mute(u32, bool),
    Profile(String, String),
}

#[derive(Debug, Clone)]
pub enum Msg {
    Do(Action),
    /// A slider moved; applied when it's let go.
    Slide(u32, f32),
    Release,
    Settings(&'static str),
}

pub struct Panel {
    pub kind: Kind,
    pub data: Option<Result<Data, String>>,
    /// What the last change said ("Connecting…", an error).
    pub status: Option<String>,
    /// A change is running; reads wait for it.
    pub busy: bool,
    /// A slider being dragged: node and level.
    pub sliding: Option<(u32, f32)>,
}

impl Panel {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            data: None,
            status: None,
            busy: false,
            sliding: None,
        }
    }

    /// May the bar read it again now?
    pub fn idle(&self) -> bool {
        !self.busy && self.sliding.is_none()
    }

    /// Height to grow the bar's surface by: the content's, as `view` lays
    /// it out, up to a limit (past that it scrolls).
    pub fn height(&self) -> u32 {
        const SECTION: u32 = 30;
        const NOTE: u32 = 32;
        // Rule, footer links and the panel's padding.
        const FRAME: u32 = 9 + 32 + 16 + 4;
        let n = |count: usize| count as u32;
        let body = match &self.data {
            None | Some(Err(_)) => ROW + NOTE,
            Some(Ok(Data::Network(net))) => {
                let wired = n(net.devices.iter().filter(|d| d.kind == "ethernet").count());
                let wifi = match (net.wifi_enabled, networks(net).len()) {
                    (false, _) => 0,
                    (true, 0) => NOTE,
                    (true, k) => n(k.min(8)) * ROW,
                };
                let vpn = match vpns(net).len() {
                    0 => 0,
                    k => SECTION + n(k) * ROW,
                };
                ROW + wired * ROW + wifi + vpn
            }
            Some(Ok(Data::Bluetooth(b))) => {
                if !b.available || !b.powered {
                    ROW + NOTE
                } else {
                    match paired(b).len() {
                        0 => ROW + NOTE,
                        k => ROW + n(k) * ROW,
                    }
                }
            }
            Some(Ok(Data::Sound(a, cards))) => {
                let choices = |k: usize| if k < 2 { 0 } else { n(k) * ROW };
                let level = |some: bool| if some { ROW } else { NOTE };
                ROW + SECTION
                    + level(a.default_sink().is_some())
                    + choices(a.sinks.len())
                    + SECTION
                    + level(a.default_source().is_some())
                    + choices(a.sources.len())
                    + cards
                        .iter()
                        .filter(|c| c.profiles.len() > 1)
                        .map(|c| SECTION + n(c.profiles.len()) * ROW)
                        .sum::<u32>()
            }
        };
        let status = if self.status.is_some() { NOTE } else { 0 };
        (body + status + FRAME).min(MAX_HEIGHT)
    }
}

/// Read a panel's state. Blocking: run it in the background.
pub fn read(kind: Kind) -> Result<Data, String> {
    match kind {
        Kind::Network => network::read().map(Data::Network),
        Kind::Bluetooth => bluetooth::read().map(Data::Bluetooth),
        Kind::Sound => {
            let (a, cards) = std::thread::scope(|s| {
                let cards = s.spawn(sys::cards);
                (audio::read(), cards.join().unwrap_or_default())
            });
            a.map(|a| Data::Sound(a, cards))
        }
    }
}

/// Carry out a change. Blocking. Ok holds what to say, if anything.
pub fn perform(action: Action) -> Result<Option<String>, String> {
    match action {
        Action::WifiPower(on) => network::set_wifi(on).map(|_| None),
        Action::Join { ssid, security } => network::connect(&ssid, &security, None).map(Some),
        Action::Disconnect(dev) => network::disconnect(&dev).map(|_| None),
        Action::Up(uuid) => network::up(&uuid).map(|_| None),
        Action::Down(uuid) => network::down(&uuid).map(|_| None),
        Action::Rescan => network::rescan().map(|_| None),
        Action::BtPower(on) => bluetooth::set_power(on).map(|_| None),
        Action::BtConnect(a) => bluetooth::connect(&a).map(|_| None),
        Action::BtDisconnect(a) => bluetooth::disconnect(&a).map(|_| None),
        Action::Default(id) => audio::set_default(id).map(|_| None),
        Action::Volume(id, v) => audio::set_volume(id, v).map(|_| None),
        Action::Mute(id, m) => audio::set_mute(id, m).map(|_| None),
        Action::Profile(card, p) => sys::set_profile(&card, &p).map(|_| None),
    }
}

/// What to say while a change runs, for the slow ones.
pub fn working(action: &Action) -> Option<String> {
    match action {
        Action::Join { ssid, .. } => Some(format!("Connecting to {ssid}…")),
        Action::Up(_) => Some("Connecting…".into()),
        Action::Rescan => Some("Looking for networks…".into()),
        Action::BtConnect(_) => Some("Connecting…".into()),
        _ => None,
    }
}

/// Nearby networks, one row per name: the one in use first, then by signal.
fn networks(n: &network::Network) -> Vec<&network::AccessPoint> {
    let mut out: Vec<&network::AccessPoint> = vec![];
    for ap in &n.access_points {
        if ap.ssid.is_empty() {
            continue;
        }
        match out.iter_mut().find(|o| o.ssid == ap.ssid) {
            Some(o) => {
                if ap.in_use || (!o.in_use && ap.signal > o.signal) {
                    *o = ap;
                }
            }
            None => out.push(ap),
        }
    }
    out.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    out
}

fn vpns(n: &network::Network) -> Vec<&network::Saved> {
    n.saved.iter().filter(|s| s.kind == "VPN").collect()
}

fn paired(b: &bluetooth::Bluetooth) -> Vec<&bluetooth::Device> {
    b.devices.iter().filter(|d| d.paired).collect()
}

/// Colours on the panel's dark or light glass.
#[derive(Clone, Copy)]
struct Look {
    /// Multiplies corner radii drawn at the default roundness.
    scale: f32,
    ink: Color,
    muted: Color,
    dim: Color,
    accent: Color,
    hover: Color,
    line: Color,
    alert: Color,
}

impl Look {
    fn corner(&self, base: f32) -> f32 {
        base * self.scale
    }

    fn of(p: &Palette) -> Self {
        let (hover, line) = if p.dark {
            (
                Color::from_rgba8(255, 255, 255, 0.10),
                Color::from_rgba8(255, 255, 255, 0.08),
            )
        } else {
            (
                Color::from_rgba8(20, 24, 33, 0.08),
                Color::from_rgba8(20, 24, 33, 0.10),
            )
        };
        Look {
            scale: p.corner(1.0),
            ink: color(p.text),
            muted: color(p.muted),
            dim: color(p.dim),
            accent: color(p.accent),
            hover,
            line,
            alert: color(p.ansi[1]),
        }
    }
}

type El<'a> = Element<'a, Msg>;

/// A title with an optional on/off switch.
/// A header toggle: its state, and the message it sends.
type Switch = (bool, fn(bool) -> Msg);

fn header<'a>(title: &'a str, switch: Option<Switch>, g: Look) -> El<'a> {
    let mut r = row![
        text(title)
            .font(INTER_SEMIBOLD)
            .size(14)
            .color(g.ink)
            .width(Length::Fill)
    ]
    .align_y(Alignment::Center)
    .height(ROW as f32)
    .padding([0, 10]);
    if let Some((on, f)) = switch {
        let accent = g.accent;
        let off = g.hover;
        r = r.push(toggler(on).on_toggle(f).size(18).style(move |_, status| {
            let lit = matches!(
                status,
                toggler::Status::Active { is_toggled: true }
                    | toggler::Status::Hovered { is_toggled: true }
            );
            toggler::Style {
                background: Background::Color(if lit { accent } else { off }),
                background_border_width: 0.0,
                background_border_color: Color::TRANSPARENT,
                foreground: Background::Color(Color::WHITE),
                foreground_border_width: 0.0,
                foreground_border_color: Color::TRANSPARENT,
                text_color: None,
                border_radius: None,
                padding_ratio: 0.2,
            }
        }));
    }
    r.into()
}

fn section<'a>(label: &'a str, g: Look) -> El<'a> {
    container(
        text(label.to_uppercase())
            .font(INTER_SEMIBOLD)
            .size(10)
            .color(g.dim),
    )
    .padding([10, 10])
    .into()
}

/// One clickable row: an icon, a label with a detail under or beside it,
/// and a word on the right ("Connected").
fn item<'a>(
    glyph: impl Into<icons::Face>,
    tone: Color,
    label: String,
    detail: String,
    trailing: &'a str,
    on: Option<Msg>,
    g: Look,
) -> El<'a> {
    let mut words = row![
        text(label)
            .font(INTER)
            .size(13)
            .color(g.ink)
            .wrapping(iced::widget::text::Wrapping::None)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if !detail.is_empty() {
        words = words.push(
            text(detail)
                .font(INTER)
                .size(11)
                .color(g.dim)
                .wrapping(iced::widget::text::Wrapping::None),
        );
    }
    let content = row![
        container(icons::draw(glyph, 14.0, tone)).width(20),
        container(words).width(Length::Fill).clip(true),
        text(trailing).font(INTER).size(11).color(g.accent),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    button(content)
        .width(Length::Fill)
        .height(ROW as f32)
        .padding([0, 10])
        .on_press_maybe(on)
        .style(move |_, status| button::Style {
            background: (status == button::Status::Hovered).then_some(Background::Color(g.hover)),
            text_color: g.ink,
            border: Border {
                radius: g.corner(8.0).into(),
                ..Border::default()
            },
            ..button::Style::default()
        })
        .into()
}

fn link<'a>(label: &'a str, on: Msg, g: Look) -> El<'a> {
    button(text(label).font(INTER).size(12).color(g.muted))
        .padding([6, 10])
        .on_press(on)
        .style(move |_, status| button::Style {
            background: (status == button::Status::Hovered).then_some(Background::Color(g.hover)),
            border: Border {
                radius: g.corner(7.0).into(),
                ..Border::default()
            },
            ..button::Style::default()
        })
        .into()
}

fn note<'a>(s: String, tone: Color) -> El<'a> {
    container(text(s).font(INTER).size(12).color(tone))
        .padding([8, 10])
        .into()
}

fn rule<'a>(g: Look) -> El<'a> {
    container(
        container(Space::new().width(Length::Fill).height(1.0)).style(move |_| container::Style {
            background: Some(Background::Color(g.line)),
            ..container::Style::default()
        }),
    )
    .padding([4, 6])
    .into()
}

fn signal_tone(signal: u8, g: Look) -> Color {
    if signal >= 40 { g.ink } else { g.muted }
}

pub fn view<'a>(p: &Palette, panel: &'a Panel) -> El<'a> {
    let g = Look::of(p);
    let mut body = column![].spacing(1);
    match &panel.data {
        None => {
            body = body
                .push(header(title(panel.kind), None, g))
                .push(note("Loading…".into(), g.dim));
        }
        Some(Err(e)) => {
            body = body
                .push(header(title(panel.kind), None, g))
                .push(note(e.clone(), g.alert));
        }
        Some(Ok(Data::Network(n))) => body = network_view(body, n, g),
        Some(Ok(Data::Bluetooth(b))) => body = bluetooth_view(body, b, g),
        Some(Ok(Data::Sound(a, cards))) => body = sound_view(body, a, cards, panel.sliding, g),
    }
    if let Some(s) = &panel.status {
        let tone = if panel.busy { g.muted } else { g.alert };
        body = body.push(note(s.clone(), tone));
    }
    body = body.push(rule(g));
    let mut foot = row![].spacing(4);
    if panel.kind == Kind::Network
        && matches!(&panel.data, Some(Ok(Data::Network(n))) if n.wifi_enabled)
    {
        foot = foot.push(link("Scan", Msg::Do(Action::Rescan), g));
    }
    foot = foot.push(Space::new().width(Length::Fill)).push(link(
        settings_label(panel.kind),
        Msg::Settings(panel.kind.page()),
        g,
    ));
    body = body.push(foot);
    scrollable(body).height(Length::Fill).into()
}

fn title(kind: Kind) -> &'static str {
    match kind {
        Kind::Network => "Network",
        Kind::Bluetooth => "Bluetooth",
        Kind::Sound => "Sound",
    }
}

fn settings_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Network => "Network settings…",
        Kind::Bluetooth => "Pair a device…",
        Kind::Sound => "Sound settings…",
    }
}

fn network_view<'a>(
    mut body: iced::widget::Column<'a, Msg>,
    n: &'a network::Network,
    g: Look,
) -> iced::widget::Column<'a, Msg> {
    body = body.push(header(
        "Wi-Fi",
        Some((n.wifi_enabled, |on| Msg::Do(Action::WifiPower(on)))),
        g,
    ));
    for d in n.devices.iter().filter(|d| d.kind == "ethernet") {
        let connected = d.state.starts_with("connected");
        body = body.push(item(
            icons::SWAP,
            if connected { g.ink } else { g.dim },
            "Wired".into(),
            if connected {
                d.address.clone()
            } else {
                d.state.clone()
            },
            if connected { "Connected" } else { "" },
            None,
            g,
        ));
    }
    let wifi_dev = n
        .devices
        .iter()
        .find(|d| d.kind == "wifi")
        .map(|d| d.name.clone());
    if n.wifi_enabled {
        let list = networks(n);
        if list.is_empty() {
            body = body.push(note("No networks found yet.".into(), g.dim));
        }
        for ap in list.into_iter().take(8) {
            let saved = n.saved.iter().any(|s| s.name == ap.ssid);
            let on = if ap.in_use {
                wifi_dev.clone().map(|d| Msg::Do(Action::Disconnect(d)))
            } else if saved || !ap.secured() {
                Some(Msg::Do(Action::Join {
                    ssid: ap.ssid.clone(),
                    security: ap.security.clone(),
                }))
            } else {
                // A new secured network needs its password typed.
                Some(Msg::Settings("network"))
            };
            let detail = match (ap.secured(), saved) {
                (_, true) if !ap.in_use => "Saved".to_owned(),
                (true, _) => "Secured".to_owned(),
                (false, _) => "Open".to_owned(),
            };
            body = body.push(item(
                icons::Symbol::Wifi,
                signal_tone(ap.signal, g),
                ap.ssid.clone(),
                detail,
                if ap.in_use { "Connected" } else { "" },
                on,
                g,
            ));
        }
    }
    let vpn = vpns(n);
    if !vpn.is_empty() {
        body = body.push(section("VPN", g));
        for v in vpn {
            let msg = if v.active {
                Msg::Do(Action::Down(v.uuid.clone()))
            } else {
                Msg::Do(Action::Up(v.uuid.clone()))
            };
            body = body.push(item(
                icons::LOCK,
                if v.active { g.accent } else { g.dim },
                v.name.clone(),
                String::new(),
                if v.active { "On" } else { "" },
                Some(msg),
                g,
            ));
        }
    }
    body
}

fn bluetooth_view<'a>(
    mut body: iced::widget::Column<'a, Msg>,
    b: &'a bluetooth::Bluetooth,
    g: Look,
) -> iced::widget::Column<'a, Msg> {
    if !b.available {
        return body
            .push(header("Bluetooth", None, g))
            .push(note("No Bluetooth adapter.".into(), g.dim));
    }
    body = body.push(header(
        "Bluetooth",
        Some((b.powered, |on| Msg::Do(Action::BtPower(on)))),
        g,
    ));
    if !b.powered {
        return body.push(note("Off.".into(), g.dim));
    }
    let devices = paired(b);
    if devices.is_empty() {
        body = body.push(note("No paired devices yet.".into(), g.dim));
    }
    for d in devices {
        let mut detail = d.kind().to_owned();
        if let Some(pct) = d.battery {
            detail = format!("{detail} · {pct}%");
        }
        let msg = if d.connected {
            Msg::Do(Action::BtDisconnect(d.address.clone()))
        } else {
            Msg::Do(Action::BtConnect(d.address.clone()))
        };
        body = body.push(item(
            icons::Symbol::Bluetooth,
            if d.connected { g.accent } else { g.dim },
            if d.name.is_empty() {
                d.address.clone()
            } else {
                d.name.clone()
            },
            detail,
            if d.connected { "Connected" } else { "" },
            Some(msg),
            g,
        ));
    }
    body
}

/// A level: a mute button and a slider. Moving shows at once; the change
/// is made when the slider is let go.
fn level<'a>(node: &'a audio::Node, sliding: Option<(u32, f32)>, input: bool, g: Look) -> El<'a> {
    let value = match sliding {
        Some((id, v)) if id == node.id => v,
        _ => node.volume.min(1.0),
    };
    let glyph = if input {
        if node.muted {
            icons::BELL_OFF
        } else {
            icons::MUSIC
        }
    } else {
        icons::volume(value, node.muted)
    };
    let id = node.id;
    let muted = node.muted;
    row![
        mouse_area(
            container(icons::draw(glyph, 15.0, if muted { g.dim } else { g.ink }))
                .width(28)
                .height(ROW as f32)
                .align_y(Vertical::Center)
        )
        .on_press(Msg::Do(Action::Mute(id, !muted)))
        .interaction(iced::mouse::Interaction::Pointer),
        slider(0.0..=1.0, value, move |v| Msg::Slide(id, v))
            .step(0.01_f32)
            .on_release(Msg::Release)
            .style(move |_, _| slider::Style {
                rail: slider::Rail {
                    backgrounds: (Background::Color(g.accent), Background::Color(g.hover)),
                    width: 4.0,
                    border: Border {
                        radius: g.corner(2.0).into(),
                        ..Border::default()
                    },
                },
                handle: slider::Handle {
                    shape: slider::HandleShape::Circle { radius: 7.0 },
                    background: Background::Color(g.accent),
                    border_width: 0.0,
                    border_color: Color::TRANSPARENT,
                },
            })
            .width(Length::Fill),
        text(format!("{:>3}%", (value * 100.0).round() as i32))
            .font(INTER)
            .size(11)
            .color(g.muted)
            .width(38),
    ]
    .spacing(8)
    .padding([0, 10])
    .align_y(Alignment::Center)
    .into()
}

fn sound_view<'a>(
    mut body: iced::widget::Column<'a, Msg>,
    a: &'a audio::Audio,
    cards: &'a [sys::Card],
    sliding: Option<(u32, f32)>,
    g: Look,
) -> iced::widget::Column<'a, Msg> {
    body = body.push(header("Sound", None, g));
    let devices = |body: iced::widget::Column<'a, Msg>, nodes: &'a [audio::Node], glyph: char| {
        let mut body = body;
        // One device: nothing to choose.
        if nodes.len() < 2 {
            return body;
        }
        for n in nodes {
            body = body.push(item(
                glyph,
                if n.default { g.accent } else { g.dim },
                n.name.clone(),
                String::new(),
                if n.default { "✓" } else { "" },
                (!n.default).then_some(Msg::Do(Action::Default(n.id))),
                g,
            ));
        }
        body
    };
    body = body.push(section("Output", g));
    match a.default_sink() {
        Some(s) => body = body.push(level(s, sliding, false, g)),
        None => body = body.push(note("No output device.".into(), g.dim)),
    }
    body = devices(body, &a.sinks, icons::VOLUME);
    body = body.push(section("Input", g));
    match a.default_source() {
        Some(s) => body = body.push(level(s, sliding, true, g)),
        None => body = body.push(note("No microphone.".into(), g.dim)),
    }
    body = devices(body, &a.sources, icons::MUSIC);
    for c in cards.iter().filter(|c| c.profiles.len() > 1) {
        body = body.push(section(&c.description, g));
        for (name, desc) in &c.profiles {
            let active = *name == c.active;
            body = body.push(item(
                icons::SLIDERS,
                if active { g.accent } else { g.dim },
                desc.clone(),
                String::new(),
                if active { "✓" } else { "" },
                (!active).then(|| Msg::Do(Action::Profile(c.name.clone(), name.clone()))),
                g,
            ));
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(ssid: &str, signal: u8, in_use: bool) -> network::AccessPoint {
        network::AccessPoint {
            ssid: ssid.into(),
            signal,
            security: "WPA2".into(),
            in_use,
        }
    }

    #[test]
    fn networks_dedupe_and_put_the_current_first() {
        let n = network::Network {
            access_points: vec![
                ap("cafe", 80, false),
                ap("home", 40, true),
                ap("cafe", 90, false),
                ap("", 99, false),
            ],
            ..Default::default()
        };
        let names: Vec<(&str, u8)> = networks(&n)
            .iter()
            .map(|a| (a.ssid.as_str(), a.signal))
            .collect();
        assert_eq!(names, vec![("home", 40), ("cafe", 90)]);
    }

    #[test]
    fn panels_grow_with_content_but_stop() {
        let mut p = Panel::new(Kind::Network);
        let empty = p.height();
        p.data = Some(Ok(Data::Network(network::Network {
            wifi_enabled: true,
            access_points: (0..30).map(|i| ap(&format!("n{i}"), 50, false)).collect(),
            ..Default::default()
        })));
        assert!(p.height() > empty);
        assert!(p.height() <= MAX_HEIGHT);
    }
}
