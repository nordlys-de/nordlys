//! Networking through NetworkManager's `nmcli`.

use super::{SLOW, run, run_for, split_terse};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Network {
    pub available: bool,
    pub wifi_enabled: bool,
    pub airplane: bool,
    pub hostname: String,
    pub devices: Vec<Device>,
    pub access_points: Vec<AccessPoint>,
    pub saved: Vec<Saved>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Device {
    pub name: String,
    pub kind: String,
    pub state: String,
    pub connection: String,
    pub address: String,
    pub gateway: String,
    pub dns: Vec<String>,
    pub hw: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccessPoint {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub in_use: bool,
}

impl AccessPoint {
    pub fn secured(&self) -> bool {
        !self.security.is_empty() && self.security != "--"
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Saved {
    pub name: String,
    pub uuid: String,
    pub kind: String,
    pub active: bool,
    pub autoconnect: bool,
}

pub fn read() -> Result<Network, String> {
    let radio = run("nmcli", &["-t", "-f", "WIFI,WWAN", "radio"])?;
    let radio = split_terse(&radio);
    let wifi_enabled = radio.first().is_some_and(|s| s == "enabled");
    let wwan = radio.get(1).is_some_and(|s| s == "enabled");
    let devices = parse_devices(
        &run(
            "nmcli",
            &[
                "-t",
                "-f",
                "GENERAL.DEVICE,GENERAL.TYPE,GENERAL.STATE,GENERAL.CONNECTION,GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS",
                "device",
                "show",
            ],
        )
        .unwrap_or_default(),
    );
    let access_points = if wifi_enabled {
        parse_access_points(
            &run(
                "nmcli",
                &[
                    "-t",
                    "-f",
                    "IN-USE,SSID,SIGNAL,SECURITY",
                    "device",
                    "wifi",
                    "list",
                    "--rescan",
                    "no",
                ],
            )
            .unwrap_or_default(),
        )
    } else {
        vec![]
    };
    let saved = parse_saved(
        &run(
            "nmcli",
            &[
                "-t",
                "-f",
                "NAME,UUID,TYPE,ACTIVE,AUTOCONNECT",
                "connection",
                "show",
            ],
        )
        .unwrap_or_default(),
    );
    let hostname = run("hostnamectl", &["hostname"])
        .or_else(|_| std::fs::read_to_string("/etc/hostname").map_err(|e| e.to_string()))
        .unwrap_or_default()
        .trim()
        .to_owned();
    Ok(Network {
        available: true,
        wifi_enabled,
        airplane: !wifi_enabled && !wwan && !bluetooth_on(),
        hostname,
        devices,
        access_points,
        saved,
    })
}

fn bluetooth_on() -> bool {
    run("rfkill", &["-n", "-o", "TYPE,SOFT", "list"])
        .map(|s| {
            s.lines()
                .any(|l| l.starts_with("bluetooth") && l.trim_end().ends_with("unblocked"))
        })
        .unwrap_or(false)
}

fn parse_devices(s: &str) -> Vec<Device> {
    let mut out = Vec::new();
    let mut cur: Option<Device> = None;
    for line in s.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.replace("\\:", ":");
        if key == "GENERAL.DEVICE" {
            out.extend(cur.take());
            cur = Some(Device {
                name: value,
                ..Device::default()
            });
            continue;
        }
        let Some(d) = cur.as_mut() else { continue };
        match key {
            "GENERAL.TYPE" => d.kind = value,
            "GENERAL.STATE" => {
                d.state = value
                    .split_once(' ')
                    .map(|(_, s)| s.trim_matches(['(', ')']).to_owned())
                    .unwrap_or(value)
            }
            "GENERAL.CONNECTION" => d.connection = value,
            "GENERAL.HWADDR" => d.hw = value,
            "IP4.GATEWAY" => d.gateway = value,
            k if k.starts_with("IP4.ADDRESS") && d.address.is_empty() => d.address = value,
            k if k.starts_with("IP4.DNS") => d.dns.push(value),
            _ => {}
        }
    }
    out.extend(cur);
    out.retain(|d| {
        matches!(
            d.kind.as_str(),
            "wifi" | "ethernet" | "gsm" | "wireguard" | "vpn" | "tun"
        )
    });
    out
}

fn parse_access_points(s: &str) -> Vec<AccessPoint> {
    let mut aps: Vec<AccessPoint> = s
        .lines()
        .filter_map(|l| {
            let f = split_terse(l);
            let ssid = f.get(1)?.clone();
            (!ssid.trim().is_empty() && ssid != "--").then(|| AccessPoint {
                in_use: f[0] == "*",
                ssid,
                signal: f.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
                security: f.get(3).cloned().unwrap_or_default(),
            })
        })
        .collect();
    // One row per name: the one in use, else the strongest.
    aps.sort_by(|a, b| {
        b.in_use
            .cmp(&a.in_use)
            .then(b.signal.cmp(&a.signal))
            .then(a.ssid.cmp(&b.ssid))
    });
    let mut seen = std::collections::HashSet::new();
    aps.retain(|a| seen.insert(a.ssid.clone()));
    aps
}

fn parse_saved(s: &str) -> Vec<Saved> {
    s.lines()
        .filter_map(|l| {
            let f = split_terse(l);
            let kind = match f.get(2)?.as_str() {
                "802-11-wireless" => "Wi-Fi",
                "802-3-ethernet" => "Wired",
                "vpn" | "wireguard" => "VPN",
                "gsm" => "Mobile",
                _ => return None,
            };
            Some(Saved {
                name: f[0].clone(),
                uuid: f[1].clone(),
                kind: kind.into(),
                active: f.get(3).is_some_and(|s| s == "yes"),
                autoconnect: f.get(4).is_some_and(|s| s == "yes"),
            })
        })
        .collect()
}

pub fn set_wifi(on: bool) -> Result<(), String> {
    run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]).map(|_| ())
}

pub fn set_airplane(on: bool) -> Result<(), String> {
    run("nmcli", &["radio", "all", if on { "off" } else { "on" }])?;
    let _ = run(
        "rfkill",
        &[if on { "block" } else { "unblock" }, "bluetooth"],
    );
    Ok(())
}

pub fn rescan() -> Result<(), String> {
    run_for(SLOW, "nmcli", &["device", "wifi", "rescan"]).map(|_| ())
}

/// Join a Wi-Fi network. A saved one connects as it is; a new secured one
/// takes `password`, which goes to NetworkManager over D-Bus so it never
/// shows up in a process list.
pub fn connect(ssid: &str, security: &str, password: Option<&str>) -> Result<String, String> {
    connect_with(ssid, security, password, false)
}

/// A network that doesn't broadcast its name, so it's missing from the list.
/// The password is optional: without one it's joined as an open network.
pub fn connect_hidden(ssid: &str, password: Option<&str>) -> Result<String, String> {
    let security = if password.is_some() { "WPA2" } else { "--" };
    connect_with(ssid, security, password, true)
}

fn connect_with(
    ssid: &str,
    security: &str,
    password: Option<&str>,
    hidden: bool,
) -> Result<String, String> {
    let saved = run("nmcli", &["-t", "-f", "NAME", "connection", "show"])
        .map(|s| s.lines().any(|l| split_terse(l)[0] == ssid))
        .unwrap_or(false);
    if saved && password.is_none() {
        return run_for(SLOW, "nmcli", &["connection", "up", "id", ssid])
            .map(|_| format!("Connected to {ssid}"));
    }
    if saved {
        // A new password for a saved network: start over with it.
        let _ = run("nmcli", &["connection", "delete", "id", ssid]);
    }
    let secured = !security.is_empty() && security != "--";
    if secured && password.is_none() {
        return Err("That network needs a password".into());
    }
    if security.contains("802.1X") {
        return Err("Enterprise networks need nm-connection-editor for now".into());
    }
    nm::add_and_activate(ssid, security, password, hidden)?;
    Ok(format!("Connected to {ssid}"))
}

mod nm {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    const NM: &str = "org.freedesktop.NetworkManager";
    const NM_PATH: &str = "/org/freedesktop/NetworkManager";

    fn bus() -> Result<Connection, String> {
        zbus::blocking::connection::Builder::system()
            .and_then(|b| b.method_timeout(Duration::from_secs(10)).build())
            .map_err(|e| format!("system bus: {e}"))
    }

    fn wifi_device(bus: &Connection) -> Result<OwnedObjectPath, String> {
        let devices: Vec<OwnedObjectPath> = bus
            .call_method(Some(NM), NM_PATH, Some(NM), "GetDevices", &())
            .and_then(|m| m.body().deserialize())
            .map_err(|e| format!("NetworkManager: {e}"))?;
        devices
            .into_iter()
            .find(|d| {
                bus.call_method(
                    Some(NM),
                    d.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("org.freedesktop.NetworkManager.Device", "DeviceType"),
                )
                .ok()
                .and_then(|m| m.body().deserialize::<OwnedValue>().ok())
                .and_then(|v| u32::try_from(v).ok())
                    == Some(2)
            })
            .ok_or_else(|| "There's no Wi-Fi adapter".to_owned())
    }

    pub fn add_and_activate(
        ssid: &str,
        security: &str,
        password: Option<&str>,
        hidden: bool,
    ) -> Result<(), String> {
        let bus = bus()?;
        let device = wifi_device(&bus)?;
        let mut settings: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
        settings.insert(
            "connection",
            HashMap::from([
                ("id", Value::from(ssid)),
                ("type", Value::from("802-11-wireless")),
            ]),
        );
        settings.insert(
            "802-11-wireless",
            HashMap::from([
                ("ssid", Value::from(ssid.as_bytes().to_vec())),
                ("mode", Value::from("infrastructure")),
                ("hidden", Value::from(hidden)),
            ]),
        );
        if let Some(pw) = password {
            let key_mgmt = if security.contains("WPA3") && !security.contains("WPA2") {
                "sae"
            } else if security.contains("WEP") {
                "none"
            } else {
                "wpa-psk"
            };
            let mut sec = HashMap::from([("key-mgmt", Value::from(key_mgmt))]);
            if key_mgmt == "none" {
                sec.insert("wep-key0", Value::from(pw));
            } else {
                sec.insert("psk", Value::from(pw));
            }
            settings.insert("802-11-wireless-security", sec);
        }
        let root = OwnedObjectPath::try_from("/").map_err(|e| e.to_string())?;
        let (_conn, active): (OwnedObjectPath, OwnedObjectPath) = bus
            .call_method(
                Some(NM),
                NM_PATH,
                Some(NM),
                "AddAndActivateConnection",
                &(settings, &device, &root),
            )
            .and_then(|m| m.body().deserialize())
            .map_err(|e| format!("NetworkManager: {e}"))?;
        // Wait for it to come up or give up: 2 activated, 4 deactivated.
        let deadline = Instant::now() + Duration::from_secs(40);
        while Instant::now() < deadline {
            let state = bus
                .call_method(
                    Some(NM),
                    active.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &("org.freedesktop.NetworkManager.Connection.Active", "State"),
                )
                .ok()
                .and_then(|m| m.body().deserialize::<OwnedValue>().ok())
                .and_then(|v| u32::try_from(v).ok());
            match state {
                Some(2) => return Ok(()),
                Some(3) | Some(4) | None => {
                    // It failed (or vanished): don't keep a broken profile.
                    let _ = super::run("nmcli", &["connection", "delete", "id", ssid]);
                    return Err(if password.is_some() {
                        "Couldn't connect. Is the password right?".into()
                    } else {
                        format!("Couldn't connect to {ssid}")
                    });
                }
                _ => std::thread::sleep(Duration::from_millis(300)),
            }
        }
        Err(format!("{ssid} is taking too long to answer"))
    }
}

pub fn disconnect(device: &str) -> Result<(), String> {
    run_for(SLOW, "nmcli", &["device", "disconnect", device]).map(|_| ())
}

pub fn up(uuid: &str) -> Result<(), String> {
    run_for(SLOW, "nmcli", &["connection", "up", "uuid", uuid]).map(|_| ())
}

pub fn down(uuid: &str) -> Result<(), String> {
    run_for(SLOW, "nmcli", &["connection", "down", "uuid", uuid]).map(|_| ())
}

pub fn forget(uuid: &str) -> Result<(), String> {
    run("nmcli", &["connection", "delete", "uuid", uuid]).map(|_| ())
}

pub fn set_autoconnect(uuid: &str, on: bool) -> Result<(), String> {
    run(
        "nmcli",
        &[
            "connection",
            "modify",
            "uuid",
            uuid,
            "connection.autoconnect",
            if on { "yes" } else { "no" },
        ],
    )
    .map(|_| ())
}

/// A valid hostname: letters, digits and inner hyphens, dots between labels.
pub fn valid_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

pub fn set_hostname(name: &str) -> Result<(), String> {
    if !valid_hostname(name) {
        return Err("Use letters, digits and hyphens only".into());
    }
    run_for(SLOW, "hostnamectl", &["hostname", name]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_from_nmcli() {
        let s = "GENERAL.DEVICE:wlan0\nGENERAL.TYPE:wifi\nGENERAL.STATE:100 (connected)\n\
                 GENERAL.CONNECTION:Home 5G\nGENERAL.HWADDR:AA\\:BB\nIP4.ADDRESS[1]:192.168.1.7/24\n\
                 IP4.GATEWAY:192.168.1.1\nIP4.DNS[1]:1.1.1.1\nIP4.DNS[2]:9.9.9.9\n\n\
                 GENERAL.DEVICE:lo\nGENERAL.TYPE:loopback\n";
        let d = parse_devices(s);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "wlan0");
        assert_eq!(d[0].state, "connected");
        assert_eq!(d[0].hw, "AA:BB");
        assert_eq!(d[0].address, "192.168.1.7/24");
        assert_eq!(d[0].dns, ["1.1.1.1", "9.9.9.9"]);
    }

    #[test]
    fn one_row_per_network_name() {
        let s = " :Cafe:40:WPA2\n*:Home:60:WPA2\n :Home:80:WPA2\n :Open:20:\n : :90:WPA2\n :Cafe:70:WPA2";
        let aps = parse_access_points(s);
        let names: Vec<_> = aps.iter().map(|a| (a.ssid.as_str(), a.signal)).collect();
        assert_eq!(names, [("Home", 60), ("Cafe", 70), ("Open", 20)]);
        assert!(aps[0].in_use && aps[0].secured());
        assert!(!aps[2].secured());
    }

    #[test]
    fn saved_connections() {
        let s = "Home:abc:802-11-wireless:yes:yes\nlo:x:loopback:yes:no\nWork:def:vpn:no:no";
        let v = parse_saved(s);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].kind, "Wi-Fi");
        assert!(v[0].active && v[0].autoconnect);
        assert_eq!(v[1].kind, "VPN");
    }

    #[test]
    fn hostnames() {
        assert!(valid_hostname("asterix"));
        assert!(valid_hostname("my-box.local"));
        assert!(!valid_hostname("-bad"));
        assert!(!valid_hostname("has space"));
        assert!(!valid_hostname(""));
    }
}
