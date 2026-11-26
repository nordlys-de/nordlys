//! Batteries from sysfs, power profiles from power-profiles-daemon, and
//! the session's power actions through logind.

use super::run;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Power {
    pub batteries: Vec<Battery>,
    pub on_ac: Option<bool>,
    pub profile: Option<String>,
    pub profiles: Vec<String>,
    pub can_hibernate: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Battery {
    pub name: String,
    pub model: String,
    pub percent: u8,
    pub status: String,
    /// Full charge now against when new, in percent.
    pub health: Option<u8>,
    pub cycles: Option<u32>,
    /// Hours left to empty or to full, when the battery can tell.
    pub hours_left: Option<f32>,
    pub watts: Option<f32>,
}

pub fn read() -> Result<Power, String> {
    let mut p = Power::default();
    if let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") {
        let mut entries: Vec<_> = dir.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            match attr(&path, "type").as_deref() {
                Some("Battery") if attr(&path, "scope").as_deref() != Some("Device") => {
                    p.batteries.push(battery(&path))
                }
                Some("Mains") => {
                    let online = attr(&path, "online").as_deref() == Some("1");
                    p.on_ac = Some(p.on_ac.unwrap_or(false) || online);
                }
                _ => {}
            }
        }
    }
    if let Ok(list) = run("powerprofilesctl", &["list"]) {
        let (current, all) = parse_profiles(&list);
        p.profile = current;
        p.profiles = all;
    }
    p.can_hibernate = logind_can("CanHibernate");
    Ok(p)
}

fn attr(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|s| s.trim().to_owned())
}

fn num(dir: &Path, name: &str) -> Option<f64> {
    attr(dir, name)?.parse().ok()
}

fn battery(dir: &Path) -> Battery {
    let status = attr(dir, "status").unwrap_or_else(|| "Unknown".into());
    // Some report charge in µAh, others energy in µWh.
    let (now, full, design, rate) = match num(dir, "energy_now") {
        Some(e) => (
            Some(e),
            num(dir, "energy_full"),
            num(dir, "energy_full_design"),
            num(dir, "power_now"),
        ),
        None => (
            num(dir, "charge_now"),
            num(dir, "charge_full"),
            num(dir, "charge_full_design"),
            num(dir, "current_now"),
        ),
    };
    let percent = num(dir, "capacity")
        .or_else(|| Some(now? / full? * 100.0))
        .unwrap_or(0.0)
        .clamp(0.0, 100.0) as u8;
    let hours_left = match (now, full, rate) {
        (Some(n), _, Some(r)) if r > 0.0 && status == "Discharging" => Some((n / r) as f32),
        (Some(n), Some(f), Some(r)) if r > 0.0 && status == "Charging" => {
            Some(((f - n) / r) as f32)
        }
        _ => None,
    };
    let watts = num(dir, "power_now")
        .or_else(|| Some(num(dir, "current_now")? * num(dir, "voltage_now")? / 1e6))
        .map(|w| (w / 1e6) as f32)
        .filter(|w| *w > 0.0);
    Battery {
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        model: [attr(dir, "manufacturer"), attr(dir, "model_name")]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        percent,
        status,
        health: match (full, design) {
            (Some(f), Some(d)) if d > 0.0 => Some((f / d * 100.0).clamp(0.0, 100.0) as u8),
            _ => None,
        },
        cycles: num(dir, "cycle_count").map(|c| c as u32).filter(|c| *c > 0),
        hours_left,
        watts,
    }
}

/// `powerprofilesctl list`: profiles end in `:`, the current one has `*`.
fn parse_profiles(s: &str) -> (Option<String>, Vec<String>) {
    let mut current = None;
    let mut all = vec![];
    for line in s.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_suffix(':') {
            let star = name.starts_with('*');
            let name = name.trim_start_matches('*').trim().to_owned();
            if star {
                current = Some(name.clone());
            }
            all.push(name);
        }
    }
    (current, all)
}

pub fn set_profile(name: &str) -> Result<(), String> {
    run("powerprofilesctl", &["set", name]).map(|_| ())
}

fn logind_can(method: &str) -> bool {
    let Ok(bus) = zbus::blocking::Connection::system() else {
        return false;
    };
    bus.call_method(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1",
        Some("org.freedesktop.login1.Manager"),
        method,
        &(),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<String>().ok())
    .is_some_and(|a| a == "yes")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Lock,
    Suspend,
    Hibernate,
    LogOut,
    Restart,
    ShutDown,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Lock => "Lock",
            Action::Suspend => "Suspend",
            Action::Hibernate => "Hibernate",
            Action::LogOut => "Log out",
            Action::Restart => "Restart",
            Action::ShutDown => "Shut down",
        }
    }

    /// Ends the session or turns the machine off: ask first.
    pub fn needs_confirming(self) -> bool {
        matches!(self, Action::LogOut | Action::Restart | Action::ShutDown)
    }
}

pub fn act(a: Action) -> Result<(), String> {
    match a {
        Action::Lock => super::spawn(
            "sh",
            &[
                "-c",
                "pgrep -f 'nordlys-shell lock' >/dev/null || exec nordlys-shell lock",
            ],
        ),
        Action::Suspend => run("systemctl", &["suspend"]).map(|_| ()),
        Action::Hibernate => run("systemctl", &["hibernate"]).map(|_| ()),
        Action::Restart => run("systemctl", &["reboot"]).map(|_| ()),
        Action::ShutDown => run("systemctl", &["poweroff"]).map(|_| ()),
        Action::LogOut if super::on_hyprland() => super::hypr_eval("hl.dispatch(hl.dsp.exit())"),
        Action::LogOut => {
            let id = std::env::var("XDG_SESSION_ID").map_err(|_| "no session to end".to_owned())?;
            run("loginctl", &["terminate-session", &id]).map(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_list() {
        let s = "* performance:\n    CpuDriver:\tintel_pstate\n\n  balanced:\n    CpuDriver:\tintel_pstate\n\n  power-saver:\n";
        let (cur, all) = parse_profiles(s);
        assert_eq!(cur.as_deref(), Some("performance"));
        assert_eq!(all, ["performance", "balanced", "power-saver"]);
    }

    #[test]
    fn battery_from_sysfs_files() {
        let dir = std::env::temp_dir().join(format!("nordlys-bat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (k, v) in [
            ("status", "Discharging"),
            ("energy_now", "30000000"),
            ("energy_full", "45000000"),
            ("energy_full_design", "50000000"),
            ("power_now", "10000000"),
            ("capacity", "66"),
            ("cycle_count", "120"),
        ] {
            std::fs::write(dir.join(k), v).unwrap();
        }
        let b = battery(&dir);
        assert_eq!(b.percent, 66);
        assert_eq!(b.health, Some(90));
        assert_eq!(b.cycles, Some(120));
        assert!((b.hours_left.unwrap() - 3.0).abs() < 0.01);
        assert!((b.watts.unwrap() - 10.0).abs() < 0.01);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
