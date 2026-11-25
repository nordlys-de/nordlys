//! Quick settings and media, through the standard command-line tools:
//! playerctl (MPRIS), wpctl (PipeWire), nmcli (NetworkManager),
//! bluetoothctl (BlueZ) and dunstctl (notifications).
//!
//! Shelling out keeps this simple and works on any distro that has the
//! tools. Talking to D-Bus directly is the later, lighter replacement.

use std::process::{Command, Stdio};

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn fire(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

pub fn now_playing() -> Option<NowPlaying> {
    let status = run("playerctl", &["status"])?;
    let meta = run(
        "playerctl",
        &["metadata", "--format", "{{title}}\t{{artist}}"],
    )
    .unwrap_or_default();
    parse_now_playing(&status, &meta)
}

fn parse_now_playing(status: &str, meta: &str) -> Option<NowPlaying> {
    if !matches!(status, "Playing" | "Paused") {
        return None;
    }
    let (title, artist) = meta.split_once('\t').unwrap_or((meta, ""));
    Some(NowPlaying {
        title: if title.is_empty() {
            "Unknown track".into()
        } else {
            title.into()
        },
        artist: artist.into(),
        playing: status == "Playing",
    })
}

pub fn play_pause() {
    fire("playerctl", &["play-pause"]);
}

pub fn next_track() {
    fire("playerctl", &["next"]);
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Volume {
    pub level: f32,
    pub muted: bool,
}

pub fn volume() -> Option<Volume> {
    parse_volume(&run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])?)
}

/// "Volume: 0.37" or "Volume: 0.37 [MUTED]".
fn parse_volume(s: &str) -> Option<Volume> {
    let rest = s.strip_prefix("Volume:")?.trim();
    let level = rest.split_whitespace().next()?.parse::<f32>().ok()?;
    Some(Volume {
        level: level.clamp(0.0, 1.5),
        muted: rest.contains("[MUTED]"),
    })
}

pub fn set_volume(level: f32) {
    let v = format!("{:.2}", level.clamp(0.0, 1.0));
    fire("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &v]);
}

pub fn toggle_mute() {
    fire("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
}

/// None when NetworkManager isn't there.
pub fn wifi() -> Option<bool> {
    run("nmcli", &["radio", "wifi"]).map(|s| s == "enabled")
}

pub fn set_wifi(on: bool) {
    fire("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
}

pub fn bluetooth() -> Option<bool> {
    parse_bluetooth(&run("bluetoothctl", &["show"])?)
}

fn parse_bluetooth(show: &str) -> Option<bool> {
    show.lines()
        .find_map(|l| l.trim().strip_prefix("Powered:"))
        .map(|v| v.trim() == "yes")
}

pub fn set_bluetooth(on: bool) {
    fire("bluetoothctl", &["power", if on { "on" } else { "off" }]);
}

/// One recent notification, for the ambient panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recent {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notices {
    /// Waiting while Do not disturb is on.
    pub waiting: u32,
    /// In history, ready to show again.
    pub history: u32,
    pub dnd: bool,
    /// Newest first. Only the Nordlys daemon lists them; empty with dunst.
    pub recent: Vec<Recent>,
}

/// Notifications come from the Nordlys daemon (`nordlys-shell notify`)
/// when it runs, otherwise from dunst.
pub fn notices() -> Option<Notices> {
    nordlys_notices().or_else(dunst_notices)
}

fn nordlys_notices() -> Option<Notices> {
    let (waiting, history, dnd): (u32, u32, bool) =
        nordlys_call("Status", &())?.body().deserialize().ok()?;
    let recent: Vec<(u32, String, String, String)> =
        nordlys_call("Recent", &20u32)?.body().deserialize().ok()?;
    Some(Notices {
        waiting,
        history,
        dnd,
        recent: recent
            .into_iter()
            .map(|(id, app, summary, body)| Recent {
                id,
                app,
                summary,
                body,
            })
            .collect(),
    })
}

fn dunst_notices() -> Option<Notices> {
    let dnd = run("dunstctl", &["is-paused"])? == "true";
    let count = |what| {
        run("dunstctl", &["count", what])
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    };
    Some(Notices {
        waiting: count("waiting"),
        history: count("history"),
        dnd,
        recent: vec![],
    })
}

/// Call the Nordlys daemon's control interface. None when it isn't the one
/// running, so callers fall back to dunst.
fn nordlys_call<B>(method: &str, body: &B) -> Option<zbus::Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    use crate::notify::{CONTROL, NAME, PATH};
    use std::sync::OnceLock;
    static BUS: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();
    let bus = BUS
        .get_or_init(|| zbus::blocking::Connection::session().ok())
        .as_ref()?;
    // Ask first, so a call never starts a daemon through D-Bus activation.
    let proxy = zbus::blocking::fdo::DBusProxy::new(bus).ok()?;
    let name = zbus::names::BusName::try_from(NAME).ok()?;
    if !proxy.name_has_owner(name).unwrap_or(false) {
        return None;
    }
    bus.call_method(Some(NAME), PATH, Some(CONTROL), method, body)
        .ok()
}

pub fn set_dnd(on: bool) {
    if nordlys_call("SetDnd", &on).is_none() {
        fire(
            "dunstctl",
            &["set-paused", if on { "true" } else { "false" }],
        );
    }
}

/// Show the most recent notification again.
pub fn pop_notification() {
    if nordlys_call("ShowLast", &()).is_none() {
        fire("dunstctl", &["history-pop"]);
    }
}

pub fn clear_notifications() {
    if nordlys_call("Clear", &()).is_none() {
        fire("dunstctl", &["history-clear"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_lines() {
        assert_eq!(
            parse_volume("Volume: 0.37"),
            Some(Volume {
                level: 0.37,
                muted: false
            })
        );
        assert_eq!(
            parse_volume("Volume: 0.80 [MUTED]"),
            Some(Volume {
                level: 0.8,
                muted: true
            })
        );
        assert_eq!(parse_volume("nope"), None);
    }

    #[test]
    fn media_status() {
        let np = parse_now_playing("Paused", "Aurora\tSomeone").unwrap();
        assert_eq!(
            (np.title.as_str(), np.artist.as_str(), np.playing),
            ("Aurora", "Someone", false)
        );
        assert_eq!(parse_now_playing("Stopped", "x\ty"), None);
        assert_eq!(
            parse_now_playing("Playing", "").unwrap().title,
            "Unknown track"
        );
    }

    #[test]
    fn bluetooth_power() {
        let show = "Controller 00:11:22:33:44:55 (public)\n\tName: box\n\tPowered: yes\n";
        assert_eq!(parse_bluetooth(show), Some(true));
        assert_eq!(parse_bluetooth("\tPowered: no"), Some(false));
        assert_eq!(parse_bluetooth("No default controller available"), None);
    }
}
