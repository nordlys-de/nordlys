//! Network, Bluetooth and sound, as the Settings app does them.
//!
//! The three modules are the Settings app's own files, compiled here as
//! well, so the bar and Settings can't drift apart. They expect a few
//! helpers from their parent module (`run`, `run_for`, `SLOW`,
//! `split_terse`, `spawn`), which this file provides with the same
//! behaviour. Audio profiles are only in the bar so far, below.
//!
//! TODO: move the three, with these helpers, into a crate both use.

#![allow(dead_code)]

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

#[path = "../../../nordlys-settings/src/sys/audio.rs"]
pub mod audio;
#[path = "../../../nordlys-settings/src/sys/bluetooth.rs"]
pub mod bluetooth;
#[path = "../../../nordlys-settings/src/sys/network.rs"]
pub mod network;

/// Reads: answers come back quickly or not at all.
pub const QUICK: Duration = Duration::from_millis(1500);
/// Changes that talk to hardware or the network take longer.
pub const SLOW: Duration = Duration::from_secs(45);

fn wait_within(mut child: Child, limit: Duration) -> Option<(ExitStatus, Child)> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some((status, child)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// A tool's output, or why it failed: its own message when it gave one.
pub fn run_for(limit: Duration, cmd: &str, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("{cmd} is not installed"),
            _ => format!("{cmd}: {e}"),
        })?;
    // Both pipes on threads, so a chatty tool can't fill one and stall.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut v);
        }
        v
    });
    let err_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut v);
        }
        v
    });
    let Some((status, _)) = wait_within(child, limit) else {
        return Err(format!("{cmd} took too long"));
    };
    let out = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
    if status.success() {
        Ok(out.trim_end().to_owned())
    } else {
        let msg = [err.trim(), out.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("failed")
            .lines()
            .last()
            .unwrap_or("failed")
            .to_owned();
        Err(format!("{cmd}: {msg}"))
    }
}

pub fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    run_for(QUICK, cmd, args)
}

/// Start a program and leave it running on its own.
pub fn spawn(cmd: &str, args: &[&str]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(cmd);
    crate::gpu::restore(&mut command);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{cmd}: {e}"))
}

/// Split nmcli's terse output: `:` separates fields, `\:` is a literal colon.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next()
                    && let Some(f) = fields.last_mut()
                {
                    f.push(n);
                }
            }
            ':' => fields.push(String::new()),
            c => {
                if let Some(f) = fields.last_mut() {
                    f.push(c);
                }
            }
        }
    }
    fields
}

// ---- Audio profiles ----

/// A sound card and the profiles it can use right now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Card {
    pub name: String,
    pub description: String,
    pub active: String,
    /// (name, description), available ones only, "Off" last.
    pub profiles: Vec<(String, String)>,
}

/// Cards from PipeWire's pulse layer (`pactl`). Empty without it.
pub fn cards() -> Vec<Card> {
    let Ok(json) = run("pactl", &["-f", "json", "list", "cards"]) else {
        return vec![];
    };
    parse_cards(&json)
}

fn parse_cards(json: &str) -> Vec<Card> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(json) else {
        return vec![];
    };
    list.iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?.to_owned();
            let description = c
                .pointer("/properties/device.description")
                .and_then(|d| d.as_str())
                .unwrap_or(&name)
                .to_owned();
            let active = c.get("active_profile")?.as_str().unwrap_or("").to_owned();
            let mut profiles: Vec<(i64, String, String)> = c
                .get("profiles")?
                .as_object()?
                .iter()
                .filter(|(n, p)| {
                    *n == &active
                        || p.get("available")
                            .and_then(|a| a.as_bool())
                            .unwrap_or(false)
                })
                .map(|(n, p)| {
                    let desc = p
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or(n)
                        .to_owned();
                    let priority = p.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);
                    // Off sorts last.
                    let rank = if n == "off" { i64::MIN } else { priority };
                    (rank, n.clone(), desc)
                })
                .collect();
            profiles.sort_by_key(|p| std::cmp::Reverse(p.0));
            Some(Card {
                name,
                description,
                active,
                profiles: profiles.into_iter().map(|(_, n, d)| (n, d)).collect(),
            })
        })
        .collect()
}

pub fn set_profile(card: &str, profile: &str) -> Result<(), String> {
    run("pactl", &["set-card-profile", card, profile]).map(|_| ())
}

#[cfg(test)]
mod card_tests {
    use super::*;

    #[test]
    fn cards_keep_available_profiles() {
        let json = r#"[{"name":"alsa_card.x","properties":{"device.description":"Built-in Audio"},
            "active_profile":"output:analog-stereo",
            "profiles":{
              "off":{"description":"Off","priority":0,"available":true},
              "output:analog-stereo":{"description":"Analog Stereo Output","priority":6500,"available":true},
              "output:hdmi-stereo":{"description":"HDMI","priority":5900,"available":false},
              "output:analog-stereo+input:analog-stereo":{"description":"Analog Stereo Duplex","priority":6565,"available":true}}}]"#;
        let cards = parse_cards(json);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].description, "Built-in Audio");
        let names: Vec<&str> = cards[0].profiles.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "output:analog-stereo+input:analog-stereo",
                "output:analog-stereo",
                "off"
            ]
        );
    }
}
