//! Screens from Hyprland, backlight through `brightnessctl`, night light
//! through `hyprsunset`.

use super::run;
use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Displays {
    pub monitors: Vec<Monitor>,
    pub backlights: Vec<Backlight>,
    /// Colour temperature in kelvin while night light is on.
    pub night_light: Option<u32>,
    pub can_night_light: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f32,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    pub transform: u8,
    pub focused: bool,
    pub disabled: bool,
    pub vrr: bool,
    pub available_modes: Vec<String>,
    pub physical_width: u32,
    pub physical_height: u32,
}

impl Monitor {
    /// The mode as Hyprland takes it: `3840x2160@60`.
    /// The mode in use; for a screen that's off (0×0), the one it would
    /// come back on.
    pub fn mode(&self) -> String {
        if self.width == 0 || self.height == 0 {
            return "preferred".into();
        }
        format!(
            "{}x{}@{}",
            self.width,
            self.height,
            trim_rate(self.refresh_rate)
        )
    }

    /// Size in inches across, when the screen says how big it is.
    pub fn diagonal(&self) -> Option<f32> {
        (self.physical_width > 0 && self.physical_height > 0).then(|| {
            let w = self.physical_width as f32;
            let h = self.physical_height as f32;
            (w * w + h * h).sqrt() / 25.4
        })
    }

    /// Modes as Hyprland takes them, best first and without repeats.
    pub fn modes(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for m in &self.available_modes {
            let m = m.trim_end_matches("Hz");
            let m = match m.split_once('@') {
                Some((res, rate)) => format!("{res}@{}", trim_rate(rate.parse().unwrap_or(60.0))),
                None => m.to_owned(),
            };
            if !out.contains(&m) {
                out.push(m);
            }
        }
        let current = self.mode();
        if !out.contains(&current) {
            out.insert(0, current);
        }
        out
    }
}

/// 60.00 → "60", 59.94 → "59.94".
fn trim_rate(r: f32) -> String {
    let s = format!("{r:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Backlight {
    pub device: String,
    pub percent: u8,
}

pub fn read() -> Result<Displays, String> {
    let monitors = if super::on_hyprland() {
        serde_json::from_str::<Vec<Monitor>>(&run("hyprctl", &["monitors", "all", "-j"])?)
            .map_err(|e| format!("hyprctl monitors: {e}"))?
    } else {
        vec![]
    };
    Ok(Displays {
        monitors,
        backlights: backlights(),
        night_light: night_light(),
        can_night_light: super::has("hyprsunset"),
    })
}

fn backlights() -> Vec<Backlight> {
    run("brightnessctl", &["-m", "-l", "-c", "backlight"])
        .map(|s| parse_backlights(&s))
        .unwrap_or_default()
}

/// `intel_backlight,backlight,26666,100%,26666`
fn parse_backlights(s: &str) -> Vec<Backlight> {
    s.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (f.get(1) == Some(&"backlight")).then(|| Backlight {
                device: f[0].to_owned(),
                percent: f
                    .get(3)
                    .and_then(|p| p.trim_end_matches('%').parse().ok())
                    .unwrap_or(0),
            })
        })
        .collect()
}

pub fn set_brightness(device: &str, percent: u8) -> Result<(), String> {
    let p = format!("{}%", percent.clamp(1, 100));
    run("brightnessctl", &["-d", device, "set", &p]).map(|_| ())
}

fn night_light() -> Option<u32> {
    let pid = run("pgrep", &["-x", "hyprsunset"]).ok()?;
    let pid = pid.lines().next()?;
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = cmdline
        .split(|b| *b == 0)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let t = args
        .iter()
        .position(|a| a == "-t" || a == "--temperature")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok());
    Some(t.unwrap_or(6000))
}

pub fn set_night_light(kelvin: Option<u32>) -> Result<(), String> {
    let _ = run("pkill", &["-x", "hyprsunset"]);
    if let Some(k) = kelvin {
        std::thread::sleep(std::time::Duration::from_millis(150));
        super::spawn("hyprsunset", &["-t", &k.clamp(1000, 6500).to_string()])?;
    }
    Ok(())
}

pub const TRANSFORMS: [&str; 4] = ["Normal", "90°", "180°", "270°"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitors_from_hyprctl() {
        let json = r#"[{"name":"DP-1","width":2560,"height":1440,"refreshRate":143.998,
            "x":0,"y":0,"scale":1.25,"transform":0,"focused":true,"disabled":false,"vrr":false,
            "availableModes":["2560x1440@144.00Hz","2560x1440@60.00Hz","1920x1080@60.00Hz","2560x1440@144.00Hz"],
            "physicalWidth":600,"physicalHeight":340,"extra":"ignored"}]"#;
        let m: Vec<Monitor> = serde_json::from_str(json).unwrap();
        assert_eq!(m[0].mode(), "2560x1440@144");
        assert_eq!(
            m[0].modes(),
            ["2560x1440@144", "2560x1440@60", "1920x1080@60"]
        );
        assert!((m[0].diagonal().unwrap() - 27.2).abs() < 0.2);
    }

    #[test]
    fn rates_print_short() {
        assert_eq!(trim_rate(60.0), "60");
        assert_eq!(trim_rate(59.94), "59.94");
        assert_eq!(trim_rate(143.998), "144");
    }

    #[test]
    fn backlight_lines() {
        let s = "intel_backlight,backlight,26666,63%,26666\ninput9::kana,leds,0,0%,1";
        assert_eq!(
            parse_backlights(s),
            [Backlight {
                device: "intel_backlight".into(),
                percent: 63
            }]
        );
    }
}
