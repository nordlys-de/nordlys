//! The notification daemon: Nordlys's own (`nordlys-shell notify`) over its
//! control interface, else dunst through dunstctl.

use super::run;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const CONTROL: &str = "de.nordlys.Notifications";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Notices {
    /// Who answers notifications: "Nordlys", "dunst", another, or None.
    pub daemon: Option<String>,
    pub dnd: bool,
    pub waiting: u32,
    pub history: u32,
    pub recent: Vec<Recent>,
    /// The daemon can do everything the page offers.
    pub controllable: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recent {
    pub app: String,
    pub summary: String,
    pub body: String,
}

fn bus() -> Option<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()
        .ok()?
        .method_timeout(std::time::Duration::from_millis(1500))
        .build()
        .ok()
}

fn owned(bus: &zbus::blocking::Connection, name: &str) -> bool {
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(bus) else {
        return false;
    };
    zbus::names::BusName::try_from(name)
        .ok()
        .and_then(|n| proxy.name_has_owner(n).ok())
        .unwrap_or(false)
}

fn server_name(bus: &zbus::blocking::Connection) -> Option<String> {
    let (name, _vendor, _version, _spec): (String, String, String, String) = bus
        .call_method(Some(NAME), PATH, Some(NAME), "GetServerInformation", &())
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    Some(name)
}

fn call<B>(method: &str, body: &B) -> Result<zbus::Message, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let bus = bus().ok_or("no session bus")?;
    if !owned(&bus, NAME) {
        return Err("No notification daemon is running".into());
    }
    bus.call_method(Some(NAME), PATH, Some(CONTROL), method, body)
        .map_err(|e| e.to_string())
}

pub fn read() -> Result<Notices, String> {
    let Some(bus) = bus() else {
        return Ok(Notices::default());
    };
    if !owned(&bus, NAME) {
        return Ok(Notices::default());
    }
    if let Ok(m) = call("Status", &()) {
        let (waiting, history, dnd): (u32, u32, bool) =
            m.body().deserialize().map_err(|e| e.to_string())?;
        let recent: Vec<(u32, String, String, String)> = call("Recent", &20u32)
            .ok()
            .and_then(|m| m.body().deserialize().ok())
            .unwrap_or_default();
        return Ok(Notices {
            daemon: Some("Nordlys".into()),
            dnd,
            waiting,
            history,
            recent: recent
                .into_iter()
                .map(|(_, app, summary, body)| Recent { app, summary, body })
                .collect(),
            controllable: true,
        });
    }
    let name = server_name(&bus).unwrap_or_else(|| "Unknown".into());
    if name == "dunst" {
        let count = |what| {
            run("dunstctl", &["count", what])
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        };
        return Ok(Notices {
            daemon: Some(name),
            dnd: run("dunstctl", &["is-paused"]).is_ok_and(|s| s == "true"),
            waiting: count("waiting"),
            history: count("history"),
            recent: vec![],
            controllable: true,
        });
    }
    Ok(Notices {
        daemon: Some(name),
        ..Notices::default()
    })
}

pub fn set_dnd(on: bool) -> Result<(), String> {
    if call("SetDnd", &on).is_ok() {
        return Ok(());
    }
    run(
        "dunstctl",
        &["set-paused", if on { "true" } else { "false" }],
    )
    .map(|_| ())
}

pub fn show_last() -> Result<(), String> {
    if call("ShowLast", &()).is_ok() {
        return Ok(());
    }
    run("dunstctl", &["history-pop"]).map(|_| ())
}

pub fn clear() -> Result<(), String> {
    if call("Clear", &()).is_ok() {
        return Ok(());
    }
    run("dunstctl", &["history-clear"]).map(|_| ())
}

pub fn send_test() -> Result<(), String> {
    run(
        "notify-send",
        &[
            "-a",
            "Nordlys Settings",
            "Notifications work",
            "This is what a notification looks like.",
        ],
    )
    .map(|_| ())
}
