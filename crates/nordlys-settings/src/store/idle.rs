//! What happens when you're away: `~/.config/nordlys/hypridle.conf`, which
//! the session's idle service runs instead of the Nordlys default.

use crate::sys;
use std::path::PathBuf;

/// Timeouts in seconds; None turns a step off.
#[derive(Debug, Clone, PartialEq)]
pub struct Idle {
    pub dim: Option<u32>,
    /// Backlight while dimmed, in percent.
    pub dim_level: u8,
    pub lock: Option<u32>,
    pub screen_off: Option<u32>,
    pub suspend: Option<u32>,
    pub lock_before_sleep: bool,
}

impl Default for Idle {
    /// session/hypridle.conf.
    fn default() -> Self {
        Self {
            dim: Some(240),
            dim_level: 30,
            lock: Some(300),
            screen_off: Some(360),
            suspend: Some(1800),
            lock_before_sleep: true,
        }
    }
}

pub fn user_path() -> PathBuf {
    nordlys_config::config_dir().join("hypridle.conf")
}

/// Yours if you have one, else the installed default.
pub fn load() -> Idle {
    let path = user_path();
    let text = std::fs::read_to_string(&path)
        .ok()
        .or_else(|| std::fs::read_to_string(sys::data_dir()?.join("hypridle.conf")).ok());
    text.map(|t| parse(&t)).unwrap_or_default()
}

/// Read hypridle's listeners back by what each one runs.
pub fn parse(text: &str) -> Idle {
    let mut idle = Idle {
        dim: None,
        lock: None,
        screen_off: None,
        suspend: None,
        lock_before_sleep: text
            .lines()
            .any(|l| l.trim().starts_with("before_sleep_cmd") && l.contains("lock")),
        ..Idle::default()
    };
    let mut timeout = None;
    let mut action = String::new();
    let mut in_listener = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("listener") {
            in_listener = true;
            timeout = None;
            action.clear();
        } else if in_listener && t == "}" {
            in_listener = false;
            if let Some(secs) = timeout {
                if action.contains("brightnessctl") {
                    idle.dim = Some(secs);
                    if let Some(p) = action
                        .split_whitespace()
                        .find_map(|w| w.strip_suffix('%'))
                        .and_then(|p| p.parse().ok())
                    {
                        idle.dim_level = p;
                    }
                } else if action.contains("lock") {
                    idle.lock = Some(secs);
                } else if action.contains("dpms") {
                    idle.screen_off = Some(secs);
                } else if action.contains("suspend") || action.contains("hibernate") {
                    idle.suspend = Some(secs);
                }
            }
        } else if in_listener {
            if let Some(v) = t.strip_prefix("timeout") {
                timeout = v.trim_start_matches([' ', '=']).trim().parse().ok();
            } else if let Some(v) = t.strip_prefix("on-timeout") {
                action = v.trim_start_matches([' ', '=']).to_owned();
            }
        }
    }
    idle
}

pub fn render(i: &Idle) -> String {
    let mut s = String::from(
        "# Written by Nordlys Settings (Lock & idle). Edit it there, or by hand:\n\
         # the next change in Settings rewrites this file.\n\n\
         general {\n    lock_cmd = pgrep -f \"nordlys-shell lock\" || nordlys-shell lock\n",
    );
    if i.lock_before_sleep {
        s.push_str("    before_sleep_cmd = loginctl lock-session\n");
    }
    s.push_str("    after_sleep_cmd = hyprctl dispatch 'hl.dsp.dpms({ action = \"on\" })'\n}\n");
    if let Some(t) = i.dim {
        s.push_str(&format!(
            "\nlistener {{\n    timeout = {t}\n    on-timeout = brightnessctl -s set {}%\n    on-resume = brightnessctl -r\n}}\n",
            i.dim_level.clamp(1, 100)
        ));
    }
    if let Some(t) = i.lock {
        s.push_str(&format!(
            "\nlistener {{\n    timeout = {t}\n    on-timeout = loginctl lock-session\n}}\n"
        ));
    }
    if let Some(t) = i.screen_off {
        s.push_str(&format!(
            "\nlistener {{\n    timeout = {t}\n    on-timeout = hyprctl dispatch 'hl.dsp.dpms({{ action = \"off\" }})'\n    on-resume = hyprctl dispatch 'hl.dsp.dpms({{ action = \"on\" }})'\n}}\n"
        ));
    }
    if let Some(t) = i.suspend {
        s.push_str(&format!(
            "\nlistener {{\n    timeout = {t}\n    on-timeout = systemctl suspend\n}}\n"
        ));
    }
    s
}

/// Write the file and restart hypridle on it.
pub fn save(i: &Idle) -> Result<(), String> {
    let path = user_path();
    sys::write_atomic(&path, &render(i))?;
    let p = path.to_string_lossy().into_owned();
    // The service reads NORDLYS_IDLE, set at login only when the file
    // already existed; point it here now.
    let unit = sys::run(
        "systemctl",
        &["--user", "is-active", "nordlys-idle.service"],
    );
    if unit.as_deref() == Ok("active") {
        sys::run(
            "systemctl",
            &["--user", "set-environment", &format!("NORDLYS_IDLE={p}")],
        )?;
        sys::run("systemctl", &["--user", "restart", "nordlys-idle.service"])?;
    } else if sys::run("pgrep", &["-x", "hypridle"]).is_ok() {
        let _ = sys::run("pkill", &["-x", "hypridle"]);
        std::thread::sleep(std::time::Duration::from_millis(200));
        sys::spawn("hypridle", &["-c", &p])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = include_str!("../../../../session/hypridle.conf");

    #[test]
    fn the_session_default_reads_back() {
        assert_eq!(parse(DEFAULT), Idle::default());
    }

    #[test]
    fn written_files_read_back() {
        let i = Idle {
            dim: None,
            dim_level: 40,
            lock: Some(600),
            screen_off: Some(660),
            suspend: None,
            lock_before_sleep: false,
        };
        let back = parse(&render(&i));
        assert_eq!(back.lock, Some(600));
        assert_eq!(back.screen_off, Some(660));
        assert_eq!(back.dim, None);
        assert_eq!(back.suspend, None);
        assert!(!back.lock_before_sleep);
        let i = Idle {
            dim: Some(120),
            ..i
        };
        assert_eq!(parse(&render(&i)).dim_level, 40);
    }
}
