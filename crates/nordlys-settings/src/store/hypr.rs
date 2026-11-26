//! Hyprland settings. Each change is applied live with `hyprctl eval` (which
//! also checks it: Hyprland refuses keys it doesn't know) and kept in
//! `~/.config/nordlys/settings.lua`, which the session's hyprland.lua runs
//! before your own hyprland.lua. `settings.json` beside it remembers what
//! was set, so the Lua is always rewritten whole and never parsed.

use crate::sys;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum V {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl V {
    pub fn bool(&self) -> bool {
        match self {
            V::Bool(b) => *b,
            V::Int(i) => *i != 0,
            V::Float(f) => *f != 0.0,
            V::Str(s) => s == "true" || s == "1",
        }
    }
    pub fn int(&self) -> i64 {
        match self {
            V::Bool(b) => *b as i64,
            V::Int(i) => *i,
            V::Float(f) => f.round() as i64,
            V::Str(s) => s
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0),
        }
    }
    pub fn float(&self) -> f64 {
        match self {
            V::Float(f) => *f,
            V::Int(i) => *i as f64,
            V::Bool(b) => *b as i64 as f64,
            V::Str(s) => s.parse().unwrap_or(0.0),
        }
    }
    pub fn str(&self) -> String {
        match self {
            V::Str(s) => s.clone(),
            V::Bool(b) => b.to_string(),
            V::Int(i) => i.to_string(),
            V::Float(f) => f.to_string(),
        }
    }

    pub fn lua(&self) -> String {
        match self {
            V::Bool(b) => b.to_string(),
            V::Int(i) => i.to_string(),
            V::Float(f) => {
                let s = format!("{f:.3}");
                let s = s.trim_end_matches('0');
                if s.ends_with('.') {
                    format!("{s}0")
                } else {
                    s.to_owned()
                }
            }
            V::Str(s) => lua_string(s),
        }
    }
}

fn lua_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// What the Nordlys session sets (session/hyprland.lua), or Hyprland's own
/// default for what it leaves alone. Shown until you change something.
pub fn default(key: &str) -> V {
    use V::*;
    match key {
        "general:gaps_in" => Int(6),
        "general:gaps_out" => Int(12),
        "general:border_size" => Int(1),
        "general:resize_on_border" => Bool(true),
        "general:layout" => Str("monocle".into()),
        "decoration:rounding" => Int(14),
        "decoration:shadow:enabled" => Bool(true),
        "decoration:blur:enabled" => Bool(true),
        "decoration:blur:size" => Int(10),
        "decoration:blur:passes" => Int(3),
        "decoration:dim_inactive" => Bool(false),
        "decoration:inactive_opacity" => Float(1.0),
        "animations:enabled" => Bool(true),
        "master:mfact" => Float(0.68),
        "misc:vrr" => Int(0),
        "misc:focus_on_activate" => Bool(false),
        "cursor:hide_on_key_press" => Bool(false),
        "cursor:inactive_timeout" => Float(0.0),
        "input:kb_layout" => Str("us".into()),
        "input:kb_variant" => Str(String::new()),
        "input:kb_options" => Str(String::new()),
        "input:repeat_rate" => Int(25),
        "input:repeat_delay" => Int(600),
        "input:numlock_by_default" => Bool(false),
        "input:follow_mouse" => Int(1),
        "input:sensitivity" => Float(0.0),
        "input:accel_profile" => Str(String::new()),
        "input:natural_scroll" => Bool(false),
        "input:left_handed" => Bool(false),
        "input:scroll_factor" => Float(1.0),
        "input:touchpad:natural_scroll" => Bool(true),
        "input:touchpad:tap-to-click" => Bool(true),
        "input:touchpad:disable_while_typing" => Bool(true),
        "input:touchpad:scroll_factor" => Float(1.0),
        "input:touchpad:clickfinger_behavior" => Bool(false),
        "general:allow_tearing" => Bool(false),
        "render:direct_scanout" => Int(0),
        "cursor:no_hardware_cursors" => Int(2),
        "xwayland:enabled" => Bool(true),
        "xwayland:force_zero_scaling" => Bool(false),
        "debug:disable_logs" => Bool(true),
        _ => Int(0),
    }
}

/// Every option the pages show, for reading them back from Hyprland.
pub const KEYS: &[&str] = &[
    "general:gaps_in",
    "general:gaps_out",
    "general:border_size",
    "general:resize_on_border",
    "general:layout",
    "decoration:rounding",
    "decoration:shadow:enabled",
    "decoration:blur:enabled",
    "decoration:blur:size",
    "decoration:blur:passes",
    "decoration:dim_inactive",
    "decoration:inactive_opacity",
    "animations:enabled",
    "master:mfact",
    "misc:vrr",
    "misc:focus_on_activate",
    "cursor:hide_on_key_press",
    "cursor:inactive_timeout",
    "input:kb_layout",
    "input:kb_variant",
    "input:kb_options",
    "input:repeat_rate",
    "input:repeat_delay",
    "input:numlock_by_default",
    "input:follow_mouse",
    "input:sensitivity",
    "input:accel_profile",
    "input:natural_scroll",
    "input:left_handed",
    "input:scroll_factor",
    "input:touchpad:natural_scroll",
    "input:touchpad:tap-to-click",
    "input:touchpad:disable_while_typing",
    "input:touchpad:scroll_factor",
    "input:touchpad:clickfinger_behavior",
    "general:allow_tearing",
    "render:direct_scanout",
    "cursor:no_hardware_cursors",
    "xwayland:enabled",
    "xwayland:force_zero_scaling",
    "debug:disable_logs",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MonitorRule {
    pub mode: String,
    pub position: String,
    pub scale: f64,
    pub transform: u8,
    pub vrr: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct State {
    options: BTreeMap<String, V>,
    monitors: BTreeMap<String, MonitorRule>,
}

pub struct Hypr {
    state: State,
    /// What Hyprland reported, for options not set here.
    live: HashMap<String, V>,
    dir: PathBuf,
}

impl Hypr {
    pub fn load() -> Self {
        Self::load_from(nordlys_config::config_dir())
    }

    pub fn load_from(dir: PathBuf) -> Self {
        let state = std::fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            state,
            live: HashMap::new(),
            dir,
        }
    }

    pub fn lua_path(&self) -> PathBuf {
        self.dir.join("settings.lua")
    }

    pub fn take_live(&mut self, live: HashMap<String, V>) {
        self.live = live;
    }

    pub fn get(&self, key: &str) -> V {
        self.state
            .options
            .get(key)
            .or_else(|| self.live.get(key))
            .cloned()
            .unwrap_or_else(|| default(key))
    }

    /// The options changed here, with their values.
    pub fn changed(&self) -> Vec<(&str, &V)> {
        self.state
            .options
            .iter()
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }

    /// Apply now and keep. Refused values change nothing.
    pub fn set(&mut self, key: &str, v: V) -> Result<(), String> {
        if sys::on_hyprland() {
            sys::hypr_eval(&format!("hl.config({})", lua_table(&[(key, &v)])))?;
        }
        self.state.options.insert(key.to_owned(), v.clone());
        self.live.insert(key.to_owned(), v);
        self.save()
    }

    /// Forget your value for `key`. The session's own takes over at the
    /// next login (or now, for those it sets).
    pub fn reset(&mut self, key: &str) -> Result<(), String> {
        if self.state.options.remove(key).is_some() {
            let d = default(key);
            if sys::on_hyprland() {
                let _ = sys::hypr_eval(&format!("hl.config({})", lua_table(&[(key, &d)])));
            }
            self.live.insert(key.to_owned(), d);
        }
        self.save()
    }

    pub fn reset_all(&mut self) -> Result<(), String> {
        self.state = State::default();
        let _ = std::fs::remove_file(self.dir.join("settings.json"));
        let _ = std::fs::remove_file(self.lua_path());
        if sys::on_hyprland() {
            let _ = sys::run("hyprctl", &["reload"]);
        }
        Ok(())
    }

    pub fn monitor(&self, name: &str) -> Option<&MonitorRule> {
        self.state.monitors.get(name)
    }

    /// Apply a screen's rule now. Saved separately with [`Self::keep_monitor`],
    /// after you've said it looks right.
    pub fn apply_monitor(&self, name: &str, rule: &MonitorRule) -> Result<(), String> {
        if !sys::on_hyprland() {
            return Err("Screens can be set on Hyprland only".into());
        }
        sys::hypr_eval(&monitor_lua(name, rule))
    }

    pub fn keep_monitor(&mut self, name: &str, rule: MonitorRule) -> Result<(), String> {
        self.state.monitors.insert(name.to_owned(), rule);
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.state).map_err(|e| e.to_string())?;
        sys::write_atomic(&self.dir.join("settings.json"), &json)?;
        sys::write_atomic(&self.lua_path(), &self.lua())
    }

    pub fn lua(&self) -> String {
        let mut out = String::from(
            "-- Written by Nordlys Settings; changes made here are overwritten.\n\
             -- Put your own Lua in hyprland.lua beside this file: it runs after.\n",
        );
        if !self.state.options.is_empty() {
            let pairs: Vec<(&str, &V)> = self
                .state
                .options
                .iter()
                .map(|(k, v)| (k.as_str(), v))
                .collect();
            out.push_str(&format!("hl.config({})\n", lua_table(&pairs)));
        }
        for (name, rule) in &self.state.monitors {
            out.push_str(&monitor_lua(name, rule));
            out.push('\n');
        }
        out
    }
}

/// `{ general = { gaps_in = 4 }, input = { touchpad = { … } } }`
fn lua_table(pairs: &[(&str, &V)]) -> String {
    enum Node {
        Leaf(String),
        Table(BTreeMap<String, Node>),
    }
    fn insert(t: &mut BTreeMap<String, Node>, path: &[&str], v: String) {
        let key = path[0].replace('-', "_");
        if path.len() == 1 {
            t.insert(key, Node::Leaf(v));
        } else if let Node::Table(sub) =
            t.entry(key).or_insert_with(|| Node::Table(BTreeMap::new()))
        {
            insert(sub, &path[1..], v);
        }
    }
    fn print(t: &BTreeMap<String, Node>) -> String {
        let items: Vec<String> = t
            .iter()
            .map(|(k, n)| match n {
                Node::Leaf(v) => format!("{k} = {v}"),
                Node::Table(sub) => format!("{k} = {}", print(sub)),
            })
            .collect();
        format!("{{ {} }}", items.join(", "))
    }
    let mut root = BTreeMap::new();
    for (k, v) in pairs {
        let path: Vec<&str> = k.split([':', '.']).collect();
        insert(&mut root, &path, v.lua());
    }
    print(&root)
}

fn monitor_lua(name: &str, r: &MonitorRule) -> String {
    if r.disabled {
        return format!(
            "hl.monitor({{ output = {}, disabled = true }})",
            lua_string(name)
        );
    }
    format!(
        "hl.monitor({{ output = {}, mode = {}, position = {}, scale = {}, transform = {}, vrr = {} }})",
        lua_string(name),
        lua_string(&r.mode),
        lua_string(&r.position),
        V::Float(r.scale).lua(),
        r.transform,
        r.vrr as u8
    )
}

/// Ask Hyprland for every option the pages show, in one call.
pub fn read_live() -> HashMap<String, V> {
    if !sys::on_hyprland() {
        return HashMap::new();
    }
    let batch: Vec<String> = KEYS.iter().map(|k| format!("j/getoption {k}")).collect();
    sys::run("hyprctl", &["--batch", &batch.join(";")])
        .map(|s| parse_options(&s))
        .unwrap_or_default()
}

fn parse_options(s: &str) -> HashMap<String, V> {
    let mut out = HashMap::new();
    for v in serde_json::Deserializer::from_str(s).into_iter::<serde_json::Value>() {
        let Ok(v) = v else { break };
        let Some(name) = v.get("option").and_then(|n| n.as_str()) else {
            continue;
        };
        let val = if let Some(b) = v.get("bool").and_then(|b| b.as_bool()) {
            V::Bool(b)
        } else if let Some(i) = v.get("int").and_then(|i| i.as_i64()) {
            V::Int(i)
        } else if let Some(f) = v.get("float").and_then(|f| f.as_f64()) {
            V::Float(f)
        } else if let Some(s) = v.get("str").and_then(|s| s.as_str()) {
            V::Str(if s == "[[EMPTY]]" {
                String::new()
            } else {
                s.to_owned()
            })
        } else if let Some(css) = v.get("css").and_then(|s| s.as_str()) {
            // Gaps come as "top right bottom left"; one number stands for all.
            V::Int(
                css.split_whitespace()
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0),
            )
        } else {
            continue;
        };
        out.insert(name.to_owned(), val);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_lua_tables() {
        let t = lua_table(&[
            ("general:gaps_in", &V::Int(4)),
            ("input:touchpad:tap-to-click", &V::Bool(false)),
            ("input:kb_layout", &V::Str("us,de".into())),
            ("master:mfact", &V::Float(0.6)),
        ]);
        assert_eq!(
            t,
            "{ general = { gaps_in = 4 }, input = { kb_layout = \"us,de\", touchpad = { tap_to_click = false } }, master = { mfact = 0.6 } }"
        );
    }

    #[test]
    fn strings_are_quoted_safely() {
        assert_eq!(lua_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn hyprctl_batch_answers() {
        let s = "{\"option\": \"general:gaps_in\", \"css\": \"2 2 2 2\", \"set\": true }\n\n\n\
                 {\"option\": \"input:sensitivity\", \"float\": -0.25, \"set\": true }\n\n\
                 {\"option\": \"input:kb_variant\", \"str\": \"[[EMPTY]]\", \"set\": false }\n\
                 {\"option\": \"animations:enabled\", \"bool\": true, \"set\": true }";
        let m = parse_options(s);
        assert_eq!(m["general:gaps_in"], V::Int(2));
        assert_eq!(m["input:sensitivity"], V::Float(-0.25));
        assert_eq!(m["input:kb_variant"], V::Str(String::new()));
        assert_eq!(m["animations:enabled"], V::Bool(true));
    }

    #[test]
    fn lua_file_holds_options_and_screens() {
        let dir = std::env::temp_dir().join(format!("nordlys-hypr-{}", std::process::id()));
        let mut h = Hypr::load_from(dir.clone());
        // Not on Hyprland in tests: set() only saves.
        // SAFETY: no other test reads this variable.
        unsafe { std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE") };
        h.set("decoration:rounding", V::Int(8)).unwrap();
        h.keep_monitor(
            "DP-1",
            MonitorRule {
                mode: "2560x1440@144".into(),
                position: "0x0".into(),
                scale: 1.25,
                transform: 0,
                vrr: true,
                disabled: false,
            },
        )
        .unwrap();
        let lua = std::fs::read_to_string(dir.join("settings.lua")).unwrap();
        assert!(lua.contains("hl.config({ decoration = { rounding = 8 } })"));
        assert!(lua.contains("hl.monitor({ output = \"DP-1\", mode = \"2560x1440@144\", position = \"0x0\", scale = 1.25, transform = 0, vrr = 1 })"));
        let again = Hypr::load_from(dir.clone());
        assert_eq!(again.get("decoration:rounding"), V::Int(8));
        assert!(again.monitor("DP-1").is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
