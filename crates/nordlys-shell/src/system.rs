//! Quick settings and media. Media, Wi-Fi and Bluetooth go over D-Bus
//! (`crate::dbus`); when a service can't be reached there, and for volume
//! (PipeWire has no simple D-Bus interface), the standard command-line
//! tools step in: playerctl, nmcli, bluetoothctl, wpctl and dunstctl.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long any tool may take. Some never return: `bluetoothctl` waits
/// forever when bluetoothd isn't running.
const PATIENCE: Duration = Duration::from_millis(800);

/// Run blocking work (these tools, D-Bus calls) on a thread of its own, so
/// the UI never waits on it. None only if the work panicked.
pub async fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.await.ok()
}

/// One background read at a time: asks that arrive while a read is under
/// way are folded into a single read once it lands.
#[derive(Debug, Default)]
pub struct Reads {
    busy: bool,
    again: bool,
}

impl Reads {
    /// True if a read should start now.
    pub fn start(&mut self) -> bool {
        if self.busy {
            self.again = true;
            false
        } else {
            self.busy = true;
            true
        }
    }

    /// A read landed. True if another one was asked for meanwhile.
    pub fn done(&mut self) -> bool {
        self.busy = false;
        std::mem::take(&mut self.again)
    }

    /// Ask for a fresh read after the current one, if one is running.
    pub fn invalidate(&mut self) {
        self.again |= self.busy;
    }
}

/// Everything the ambient rail shows, read in one go.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub playing: Option<NowPlaying>,
    pub notices: Option<Notices>,
    /// Only read while the panel is open.
    pub controls: Option<Controls>,
}

/// The quick settings in the ambient panel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Controls {
    pub volume: Option<Volume>,
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
}

impl Snapshot {
    /// Ask every tool at once, so the whole read takes as long as the
    /// slowest tool rather than the sum of them. Blocking: call it through
    /// [`background`].
    pub fn read(controls: bool) -> Self {
        std::thread::scope(|s| {
            let playing = s.spawn(now_playing);
            let notices = s.spawn(notices);
            let controls = controls.then(|| (s.spawn(volume), s.spawn(wifi), s.spawn(bluetooth)));
            Snapshot {
                playing: playing.join().ok().flatten(),
                notices: notices.join().ok().flatten(),
                controls: controls.map(|(v, w, b)| Controls {
                    volume: v.join().ok().flatten(),
                    wifi: w.join().ok().flatten(),
                    bluetooth: b.join().ok().flatten(),
                }),
            }
        })
    }
}

/// Wait for `child`, killing it once `limit` has passed.
fn wait_within(mut child: Child, limit: Duration) -> Option<(ExitStatus, Child)> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some((status, child)),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// A tool's output, if it succeeds in time. Outputs here are a few lines,
/// well within a pipe's buffer, so reading after exit can't deadlock.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let (status, mut child) = wait_within(child, PATIENCE)?;
    let mut out = Vec::new();
    child.stdout.take()?.read_to_end(&mut out).ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&out).trim().to_owned())
}

fn fire(cmd: &str, args: &[&str]) {
    let child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(child) = child {
        let _ = wait_within(child, PATIENCE);
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

pub fn now_playing() -> Option<NowPlaying> {
    if let Some(answer) = crate::dbus::now_playing() {
        return answer;
    }
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
    if crate::dbus::player_call("PlayPause").is_some() {
        return;
    }
    fire("playerctl", &["play-pause"]);
}

pub fn next_track() {
    if crate::dbus::player_call("Next").is_some() {
        return;
    }
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
    crate::dbus::wifi().or_else(|| run("nmcli", &["radio", "wifi"]).map(|s| s == "enabled"))
}

/// How the computer is online, for the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Wifi,
    Wired,
    /// Connected some other way (VPN only, mobile broadband…).
    Other,
    /// Connecting, or only on the local network.
    Limited,
    Offline,
}

/// None when NetworkManager isn't there.
pub fn link() -> Option<Link> {
    let (state, kind) = crate::dbus::network()?;
    Some(match state {
        70 => match kind.as_str() {
            "802-11-wireless" => Link::Wifi,
            "802-3-ethernet" => Link::Wired,
            _ => Link::Other,
        },
        40..=69 => Link::Limited,
        _ => Link::Offline,
    })
}

/// The bar's indicators: network, Bluetooth and volume.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Status {
    pub link: Option<Link>,
    /// None without an adapter.
    pub bluetooth: Option<bool>,
    pub volume: Option<Volume>,
}

impl Status {
    /// All at once. Blocking: call it through [`background`].
    pub fn read() -> Self {
        std::thread::scope(|s| {
            let link = s.spawn(link);
            let bluetooth = s.spawn(bluetooth);
            let volume = s.spawn(volume);
            Status {
                link: link.join().ok().flatten(),
                bluetooth: bluetooth.join().ok().flatten(),
                volume: volume.join().ok().flatten(),
            }
        })
    }
}

pub fn set_wifi(on: bool) {
    if crate::dbus::set_wifi(on).is_some() {
        return;
    }
    fire("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]);
}

pub fn bluetooth() -> Option<bool> {
    if let Some(answer) = crate::dbus::bluetooth() {
        return answer;
    }
    // No adapter (most VMs): don't even ask.
    let adapters = std::fs::read_dir("/sys/class/bluetooth").ok()?;
    adapters.flatten().next()?;
    parse_bluetooth(&run("bluetoothctl", &["show"])?)
}

fn parse_bluetooth(show: &str) -> Option<bool> {
    show.lines()
        .find_map(|l| l.trim().strip_prefix("Powered:"))
        .map(|v| v.trim() == "yes")
}

pub fn set_bluetooth(on: bool) {
    if crate::dbus::set_bluetooth(on).is_some() {
        return;
    }
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
    fn reads_fold_together() {
        let mut r = Reads::default();
        assert!(r.start());
        assert!(!r.start());
        assert!(!r.start());
        assert!(r.done(), "one more read for the asks that came meanwhile");
        assert!(r.start());
        assert!(!r.done());
    }

    #[test]
    fn a_hung_tool_is_given_up_on() {
        let start = Instant::now();
        assert_eq!(run("sleep", &["30"]), None);
        fire("sleep", &["30"]);
        assert!(start.elapsed() < Duration::from_secs(4));
        assert_eq!(run("echo", &["hi"]).as_deref(), Some("hi"));
    }

    #[test]
    fn bluetooth_power() {
        let show = "Controller 00:11:22:33:44:55 (public)\n\tName: box\n\tPowered: yes\n";
        assert_eq!(parse_bluetooth(show), Some(true));
        assert_eq!(parse_bluetooth("\tPowered: no"), Some(false));
        assert_eq!(parse_bluetooth("No default controller available"), None);
    }
}
