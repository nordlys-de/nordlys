//! Bluetooth through BlueZ's `bluetoothctl`.

use super::{SLOW, run, run_for};
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bluetooth {
    /// False when there's no adapter or bluetoothd isn't running.
    pub available: bool,
    pub name: String,
    pub address: String,
    pub powered: bool,
    pub discoverable: bool,
    pub pairable: bool,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Device {
    pub address: String,
    pub name: String,
    pub icon: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub battery: Option<u8>,
}

impl Device {
    /// A word for what it is, from BlueZ's icon name.
    pub fn kind(&self) -> &'static str {
        match self.icon.as_str() {
            i if i.contains("headset") || i.contains("headphone") => "Headphones",
            i if i.contains("audio") => "Speaker",
            i if i.contains("keyboard") => "Keyboard",
            i if i.contains("mouse") => "Mouse",
            i if i.contains("phone") => "Phone",
            i if i.contains("computer") => "Computer",
            i if i.contains("gaming") => "Controller",
            _ => "Device",
        }
    }
}

pub fn read() -> Result<Bluetooth, String> {
    // No adapter (most VMs): don't wake bluetoothctl at all.
    let has_adapter = std::fs::read_dir("/sys/class/bluetooth")
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    if !has_adapter {
        return Ok(Bluetooth::default());
    }
    let show = run("bluetoothctl", &["show"])?;
    if show.contains("No default controller") {
        return Ok(Bluetooth::default());
    }
    let mut bt = parse_show(&show);
    bt.available = true;
    let list = run("bluetoothctl", &["devices"]).unwrap_or_default();
    // Ask about every device at once.
    let addresses: Vec<String> = parse_devices(&list);
    bt.devices = std::thread::scope(|s| {
        let handles: Vec<_> = addresses
            .iter()
            .map(|a| {
                s.spawn(move || {
                    run("bluetoothctl", &["info", a])
                        .ok()
                        .map(|i| parse_info(a, &i))
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok().flatten())
            .collect()
    });
    bt.devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(bt)
}

fn field<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    s.lines()
        .find_map(|l| l.trim().strip_prefix(key))
        .map(|v| v.trim_start_matches(':').trim())
}

fn yes(s: &str, key: &str) -> bool {
    field(s, key) == Some("yes")
}

fn parse_show(s: &str) -> Bluetooth {
    Bluetooth {
        available: true,
        address: s
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or_default()
            .to_owned(),
        name: field(s, "Alias:")
            .or(field(s, "Name:"))
            .unwrap_or_default()
            .to_owned(),
        powered: yes(s, "Powered:"),
        discoverable: yes(s, "Discoverable:"),
        pairable: yes(s, "Pairable:"),
        devices: vec![],
    }
}

fn parse_devices(s: &str) -> Vec<String> {
    s.lines()
        .filter_map(|l| l.strip_prefix("Device "))
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

fn parse_info(address: &str, s: &str) -> Device {
    Device {
        address: address.to_owned(),
        name: field(s, "Alias:")
            .or(field(s, "Name:"))
            .unwrap_or(address)
            .to_owned(),
        icon: field(s, "Icon:").unwrap_or_default().to_owned(),
        paired: yes(s, "Paired:"),
        trusted: yes(s, "Trusted:"),
        connected: yes(s, "Connected:"),
        // "Battery Percentage: 0x5a (90)"
        battery: field(s, "Battery Percentage:")
            .and_then(|v| v.split_once('(')?.1.strip_suffix(')')?.parse().ok()),
    }
}

fn ok(r: Result<String, String>, what: &str) -> Result<(), String> {
    let out = r?;
    if out.contains("Failed") || out.contains("not available") {
        Err(format!(
            "{what}: {}",
            out.lines().last().unwrap_or("failed")
        ))
    } else {
        Ok(())
    }
}

pub fn set_power(on: bool) -> Result<(), String> {
    if on {
        let _ = run("rfkill", &["unblock", "bluetooth"]);
    }
    ok(
        run("bluetoothctl", &["power", if on { "on" } else { "off" }]),
        "Bluetooth",
    )
}

pub fn set_discoverable(on: bool) -> Result<(), String> {
    ok(
        run(
            "bluetoothctl",
            &["discoverable", if on { "on" } else { "off" }],
        ),
        "Visibility",
    )
}

/// Look for devices for a few seconds; they're in the next read.
pub fn scan(seconds: u64) -> Result<(), String> {
    run_for(
        Duration::from_secs(seconds + 3),
        "bluetoothctl",
        &["--timeout", &seconds.to_string(), "scan", "on"],
    )
    .map(|_| ())
}

pub fn connect(address: &str) -> Result<(), String> {
    ok(
        run_for(SLOW, "bluetoothctl", &["connect", address]),
        "Connect",
    )
}

pub fn disconnect(address: &str) -> Result<(), String> {
    ok(
        run_for(SLOW, "bluetoothctl", &["disconnect", address]),
        "Disconnect",
    )
}

/// Pair, trust (so it reconnects by itself) and connect.
pub fn pair(address: &str) -> Result<(), String> {
    ok(run_for(SLOW, "bluetoothctl", &["pair", address]), "Pairing")?;
    let _ = run("bluetoothctl", &["trust", address]);
    connect(address)
}

pub fn set_trusted(address: &str, on: bool) -> Result<(), String> {
    ok(
        run(
            "bluetoothctl",
            &[if on { "trust" } else { "untrust" }, address],
        ),
        "Trust",
    )
}

pub fn remove(address: &str) -> Result<(), String> {
    ok(run("bluetoothctl", &["remove", address]), "Forget")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_and_devices() {
        let show = "Controller 40:74:E0:A6:47:A2 (public)\n\tName: box\n\tAlias: My box\n\
                    \tPowered: yes\n\tDiscoverable: no\n\tPairable: yes\n";
        let bt = parse_show(show);
        assert_eq!(bt.address, "40:74:E0:A6:47:A2");
        assert_eq!(bt.name, "My box");
        assert!(bt.powered && !bt.discoverable && bt.pairable);

        let list = "Device AA:BB:CC:DD:EE:FF WH-1000XM4\nDevice 11:22:33:44:55:66 Mouse\n";
        assert_eq!(
            parse_devices(list),
            ["AA:BB:CC:DD:EE:FF", "11:22:33:44:55:66"]
        );

        let info = "Device AA:BB (public)\n\tName: WH-1000XM4\n\tAlias: Headphones\n\
                    \tIcon: audio-headset\n\tPaired: yes\n\tTrusted: yes\n\tConnected: no\n\
                    \tBattery Percentage: 0x5a (90)\n";
        let d = parse_info("AA:BB", info);
        assert_eq!(d.name, "Headphones");
        assert_eq!(d.kind(), "Headphones");
        assert!(d.paired && d.trusted && !d.connected);
        assert_eq!(d.battery, Some(90));
    }
}
