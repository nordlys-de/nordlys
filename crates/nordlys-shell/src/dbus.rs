//! Media, Wi-Fi and Bluetooth straight over D-Bus: MPRIS players on the
//! session bus, NetworkManager and BlueZ on the system bus. No processes
//! are started, so the ambient rail can read them as often as it likes.
//!
//! Every function returns None when the service can't be reached, so
//! `system` falls back to the command-line tools.

use crate::system::NowPlaying;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// No call may take longer than this; a stuck service is treated as gone.
const PATIENCE: Duration = Duration::from_millis(800);

const PROPS: &str = "org.freedesktop.DBus.Properties";
const MPRIS: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";

fn connect(system: bool) -> Option<Connection> {
    let builder = if system {
        zbus::blocking::connection::Builder::system()
    } else {
        zbus::blocking::connection::Builder::session()
    };
    builder.ok()?.method_timeout(PATIENCE).build().ok()
}

fn session() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| connect(false)).as_ref()
}

fn system() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| connect(true)).as_ref()
}

fn get(bus: &Connection, dest: &str, path: &str, iface: &str, prop: &str) -> Option<OwnedValue> {
    bus.call_method(Some(dest), path, Some(PROPS), "Get", &(iface, prop))
        .ok()?
        .body()
        .deserialize::<OwnedValue>()
        .ok()
}

fn set(bus: &Connection, dest: &str, path: &str, iface: &str, prop: &str, on: bool) -> Option<()> {
    bus.call_method(
        Some(dest),
        path,
        Some(PROPS),
        "Set",
        &(iface, prop, Value::from(on)),
    )
    .ok()
    .map(|_| ())
}

/// Names with an owner right now. Activatable-only names are left out, so
/// asking never starts a service.
fn owned_names(bus: &Connection) -> Option<Vec<String>> {
    bus.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ListNames",
        &(),
    )
    .ok()?
    .body()
    .deserialize()
    .ok()
}

// ---- Media (MPRIS) ----

struct Player {
    name: String,
    status: String,
}

/// The player to show and control: playerctld when it runs (it follows
/// the one you used last), else one that's playing, else a paused one.
fn player(bus: &Connection) -> Option<Option<Player>> {
    let names: Vec<String> = owned_names(bus)?
        .into_iter()
        .filter(|n| n.starts_with(MPRIS))
        .collect();
    let status = |name: &str| -> Option<String> {
        String::try_from(get(bus, name, MPRIS_PATH, PLAYER, "PlaybackStatus")?).ok()
    };
    let players: Vec<Player> = names
        .iter()
        .filter_map(|n| {
            Some(Player {
                name: n.clone(),
                status: status(n)?,
            })
        })
        .collect();
    Some(pick(players))
}

fn pick(players: Vec<Player>) -> Option<Player> {
    let live = |p: &Player| matches!(p.status.as_str(), "Playing" | "Paused");
    let mut players: Vec<Player> = players.into_iter().filter(live).collect();
    let rank = |p: &Player| {
        if p.name.ends_with(".playerctld") {
            0
        } else if p.status == "Playing" {
            1
        } else {
            2
        }
    };
    players.sort_by_key(rank);
    players.into_iter().next()
}

/// Some(None): the bus works and nothing is playing.
pub fn now_playing() -> Option<Option<NowPlaying>> {
    let bus = session()?;
    let Some(p) = player(bus)? else {
        return Some(None);
    };
    let meta: HashMap<String, OwnedValue> = get(bus, &p.name, MPRIS_PATH, PLAYER, "Metadata")
        .and_then(|v| HashMap::try_from(v).ok())
        .unwrap_or_default();
    let text = |k: &str| {
        meta.get(k)
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default()
    };
    let artists: Vec<String> = meta
        .get("xesam:artist")
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default();
    Some(Some(now_playing_from(
        &p.status,
        &text("xesam:title"),
        &artists,
    )))
}

fn now_playing_from(status: &str, title: &str, artists: &[String]) -> NowPlaying {
    NowPlaying {
        title: if title.is_empty() {
            "Unknown track".into()
        } else {
            title.into()
        },
        artist: artists.join(", "),
        playing: status == "Playing",
    }
}

/// Run a player method (PlayPause, Next). None if there's no bus.
pub fn player_call(method: &str) -> Option<()> {
    let bus = session()?;
    if let Some(p) = player(bus)? {
        let _ = bus.call_method(Some(p.name.as_str()), MPRIS_PATH, Some(PLAYER), method, &());
    }
    Some(())
}

// ---- Wi-Fi (NetworkManager) ----

pub fn wifi() -> Option<bool> {
    bool::try_from(get(system()?, NM, NM_PATH, NM, "WirelessEnabled")?).ok()
}

pub fn set_wifi(on: bool) -> Option<()> {
    set(system()?, NM, NM_PATH, NM, "WirelessEnabled", on)
}

/// NetworkManager's overall state (70 is fully connected) and the kind of
/// its main connection ("802-11-wireless", "802-3-ethernet"…).
pub fn network() -> Option<(u32, String)> {
    let bus = system()?;
    let state = u32::try_from(get(bus, NM, NM_PATH, NM, "State")?).ok()?;
    let kind = get(bus, NM, NM_PATH, NM, "PrimaryConnectionType")
        .and_then(|v| String::try_from(v).ok())
        .unwrap_or_default();
    Some((state, kind))
}

// ---- Bluetooth (BlueZ) ----

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// The first adapter's object path.
fn adapter(bus: &Connection) -> Option<String> {
    let objects: Objects = bus
        .call_method(
            Some(BLUEZ),
            "/",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    let mut paths: Vec<String> = objects
        .into_iter()
        .filter(|(_, ifaces)| ifaces.contains_key(ADAPTER))
        .map(|(p, _)| p.to_string())
        .collect();
    paths.sort();
    paths.into_iter().next()
}

/// Some(None): BlueZ runs but there's no adapter.
pub fn bluetooth() -> Option<Option<bool>> {
    let bus = system()?;
    owned_names(bus)?.iter().any(|n| n == BLUEZ).then_some(())?;
    let Some(path) = adapter(bus) else {
        return Some(None);
    };
    Some(bool::try_from(get(bus, BLUEZ, &path, ADAPTER, "Powered")?).ok())
}

pub fn set_bluetooth(on: bool) -> Option<()> {
    let bus = system()?;
    let path = adapter(bus)?;
    set(bus, BLUEZ, &path, ADAPTER, "Powered", on)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, status: &str) -> Player {
        Player {
            name: name.into(),
            status: status.into(),
        }
    }

    #[test]
    fn playerctld_wins_then_playing_then_paused() {
        let mpv = "org.mpris.MediaPlayer2.mpv";
        let ff = "org.mpris.MediaPlayer2.firefox.instance_1_2";
        let ctld = "org.mpris.MediaPlayer2.playerctld";
        let chosen = |v| pick(v).map(|p: Player| p.name);
        assert_eq!(
            chosen(vec![p(mpv, "Paused"), p(ff, "Playing")]).as_deref(),
            Some(ff)
        );
        assert_eq!(
            chosen(vec![p(ff, "Playing"), p(ctld, "Paused")]).as_deref(),
            Some(ctld)
        );
        assert_eq!(chosen(vec![p(mpv, "Stopped")]), None);
        assert_eq!(chosen(vec![]), None);
    }

    #[test]
    fn track_text() {
        let n = now_playing_from("Playing", "", &["A".into(), "B".into()]);
        assert_eq!(
            (n.title.as_str(), n.artist.as_str(), n.playing),
            ("Unknown track", "A, B", true)
        );
    }
}

#[cfg(test)]
mod live {
    /// Reads this machine's buses and prints what it finds; changes nothing.
    #[test]
    #[ignore = "talks to the session and system buses"]
    fn reads_this_machine() {
        eprintln!("wifi: {:?}", super::wifi());
        eprintln!("bluetooth: {:?}", super::bluetooth());
        eprintln!("now playing: {:?}", super::now_playing());
    }
}
