//! Where the bar's live data comes from. Nothing here knows about iced.
//!
//! Workspaces and the focused window come from whichever compositor is
//! running: nordlys-comp (our IPC socket), Hyprland (its request socket),
//! or a fixed demo set when neither is there (preview window on X11).

use serde_json::Value;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub active: bool,
    pub windows: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Desk {
    pub workspaces: Vec<Workspace>,
    /// What's on stage, e.g. "Helix" or "Helix + Firefox".
    pub stage: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compositor {
    Nordlys(PathBuf),
    Hyprland(PathBuf),
    Demo,
}

impl Compositor {
    pub fn detect() -> Self {
        let nordlys = nordlys_ipc::socket_path();
        if UnixStream::connect(&nordlys).is_ok() {
            return Compositor::Nordlys(nordlys);
        }
        if let Some(sig) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
            let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
            let candidates = runtime
                .map(|r| r.join("hypr").join(&sig).join(".socket.sock"))
                .into_iter()
                .chain([PathBuf::from("/tmp/hypr").join(&sig).join(".socket.sock")]);
            for p in candidates {
                if p.exists() {
                    return Compositor::Hyprland(p);
                }
            }
        }
        Compositor::Demo
    }

    pub fn label(&self) -> &'static str {
        match self {
            Compositor::Nordlys(_) => "nordlys",
            Compositor::Hyprland(_) => "hyprland",
            Compositor::Demo => "demo",
        }
    }

    pub fn desk(&self) -> Desk {
        match self {
            Compositor::Nordlys(p) => nordlys_desk(p).unwrap_or_else(demo_desk),
            Compositor::Hyprland(p) => hypr_desk(p).unwrap_or_default(),
            Compositor::Demo => demo_desk(),
        }
    }

    pub fn switch_to(&self, ws: &Workspace) {
        if let Compositor::Hyprland(p) = self {
            hypr_dispatch(
                p,
                &format!("hl.dsp.focus({{ workspace = {} }})", ws.id),
                &format!("workspace {}", ws.id),
            );
        }
        // nordlys-comp has one headless stage for now: no workspaces yet.
    }

    /// Windows waiting on this workspace's shelf, most recent first.
    ///
    /// On nordlys-comp this is the real shelf. On Hyprland it's the
    /// closest match: this workspace's windows in focus order, minus the
    /// focused one.
    pub fn shelf(&self) -> Vec<ShelfWindow> {
        match self {
            Compositor::Nordlys(p) => nordlys_state(p)
                .map(|s| {
                    s.shelf
                        .into_iter()
                        .map(|id| ShelfWindow {
                            app: pretty_app(&id),
                            title: id.clone(),
                            id,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Compositor::Hyprland(p) => hypr_shelf(p).unwrap_or_default(),
            Compositor::Demo => demo_shelf(),
        }
    }

    /// Put a shelf window on stage.
    pub fn bring(&self, w: &ShelfWindow) {
        match self {
            Compositor::Nordlys(p) => {
                let req = nordlys_ipc::Request::Bring { id: w.id.clone() };
                if let Ok(mut s) = UnixStream::connect(p) {
                    let _ = s.write_all(nordlys_ipc::to_line(&req).as_bytes());
                }
            }
            Compositor::Hyprland(p) => {
                hypr_dispatch(
                    p,
                    &format!("hl.dsp.focus({{ window = \"address:{}\" }})", w.id),
                    &format!("focuswindow address:{}", w.id),
                );
            }
            Compositor::Demo => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfWindow {
    /// Compositor's handle: window id on nordlys-comp, address on Hyprland.
    pub id: String,
    pub app: String,
    pub title: String,
}

fn demo_shelf() -> Vec<ShelfWindow> {
    let w = |id: &str, app: &str, title: &str| ShelfWindow {
        id: id.into(),
        app: app.into(),
        title: title.into(),
    };
    vec![
        w("term", "Terminal", "zsh · ~/code/nordlys"),
        w("docs", "Firefox", "Space — smithay docs"),
        w("files", "Files", "Home › code › nordlys"),
    ]
}

fn hypr_shelf(sock: &Path) -> Option<Vec<ShelfWindow>> {
    let clients: Value = serde_json::from_str(&hypr_request(sock, "j/clients")?).ok()?;
    let active: Value = serde_json::from_str(&hypr_request(sock, "j/activeworkspace")?).ok()?;
    Some(parse_hypr_shelf(
        &clients,
        active.get("id").and_then(Value::as_i64)?,
    ))
}

fn parse_hypr_shelf(clients: &Value, workspace: i64) -> Vec<ShelfWindow> {
    let mut list: Vec<(i64, ShelfWindow)> = clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c.get("mapped").and_then(Value::as_bool).unwrap_or(true))
        .filter(|c| !c.get("hidden").and_then(Value::as_bool).unwrap_or(false))
        .filter(|c| c.pointer("/workspace/id").and_then(Value::as_i64) == Some(workspace))
        .filter_map(|c| {
            let order = c.get("focusHistoryID")?.as_i64()?;
            if order <= 0 {
                return None; // 0 is the focused window: it's on stage
            }
            let class = c.get("class").and_then(Value::as_str).unwrap_or_default();
            Some((
                order,
                ShelfWindow {
                    id: c.get("address")?.as_str()?.to_owned(),
                    app: if class.is_empty() {
                        "Window".into()
                    } else {
                        pretty_app(class)
                    },
                    title: c
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                },
            ))
        })
        .collect();
    list.sort_by_key(|(order, _)| *order);
    list.into_iter().map(|(_, w)| w).collect()
}

fn nordlys_state(sock: &Path) -> Option<nordlys_ipc::StageState> {
    use nordlys_ipc::{Request, Response};
    let mut s = UnixStream::connect(sock).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    s.write_all(nordlys_ipc::to_line(&Request::State).as_bytes())
        .ok()?;
    let mut buf = String::new();
    let mut byte = [0u8; 1];
    while s.read(&mut byte).ok()? == 1 && byte[0] != b'\n' {
        buf.push(byte[0] as char);
    }
    match nordlys_ipc::from_line(&buf).ok()? {
        Response::Ok { state } => Some(state),
        Response::Error { .. } => None,
    }
}

fn demo_desk() -> Desk {
    let ws = |id, name: &str, active, windows| Workspace {
        id,
        name: name.into(),
        active,
        windows,
    };
    Desk {
        workspaces: vec![
            ws(1, "nordlys", true, 4),
            ws(2, "web", false, 2),
            ws(3, "notes", false, 1),
        ],
        stage: Some("Helix".into()),
    }
}

fn nordlys_desk(sock: &Path) -> Option<Desk> {
    let state = nordlys_state(sock)?;
    let mut d = demo_desk();
    d.stage = match (state.main, state.side) {
        (Some(m), Some(s)) => Some(format!("{m} + {s}")),
        (m, _) => m,
    };
    Some(d)
}

/// One request on Hyprland's request socket. It answers and hangs up.
fn hypr_request(sock: &Path, cmd: &str) -> Option<String> {
    let mut s = UnixStream::connect(sock).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    s.write_all(cmd.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

/// Run a dispatcher. Hyprland 0.55+ with a Lua config takes Lua
/// (`hl.dsp.focus({ workspace = 2 })`); older ones take the classic
/// form (`workspace 2`). Try Lua first and fall back.
fn hypr_dispatch(sock: &Path, lua: &str, classic: &str) {
    let ok = |r: Option<String>| r.is_some_and(|r| r.trim() == "ok");
    if !ok(hypr_request(sock, &format!("dispatch {lua}"))) {
        let _ = hypr_request(sock, &format!("dispatch {classic}"));
    }
}

fn hypr_desk(sock: &Path) -> Option<Desk> {
    let workspaces: Value = serde_json::from_str(&hypr_request(sock, "j/workspaces")?).ok()?;
    let active: Value = serde_json::from_str(&hypr_request(sock, "j/activeworkspace")?).ok()?;
    let window: Value = hypr_request(sock, "j/activewindow")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    Some(parse_hypr(&workspaces, &active, &window))
}

fn parse_hypr(workspaces: &Value, active: &Value, window: &Value) -> Desk {
    let active_id = active.get("id").and_then(Value::as_i64);
    let mut list: Vec<Workspace> = workspaces
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let id = w.get("id")?.as_i64()?;
            if id < 0 {
                return None; // special / scratchpad workspaces
            }
            Some(Workspace {
                id,
                name: w
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                active: Some(id) == active_id,
                windows: w.get("windows").and_then(Value::as_u64).unwrap_or(0) as u32,
            })
        })
        .collect();
    list.sort_by_key(|w| w.id);
    let stage = window
        .get("class")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(pretty_app);
    Desk {
        workspaces: list,
        stage,
    }
}

/// "org.gnome.Nautilus" → "Nautilus", "firefox" → "Firefox".
fn pretty_app(class: &str) -> String {
    let last = class.rsplit('.').next().unwrap_or(class);
    let mut c = last.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

/// First battery under /sys/class/power_supply, if the machine has one.
pub fn battery() -> Option<Battery> {
    battery_in(Path::new("/sys/class/power_supply"))
}

fn battery_in(root: &Path) -> Option<Battery> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery"))
        .collect();
    dirs.sort();
    let dir = dirs.first()?;
    let percent = std::fs::read_to_string(dir.join("capacity"))
        .ok()?
        .trim()
        .parse::<u8>()
        .ok()?;
    let status = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
    Some(Battery {
        percent: percent.min(100),
        charging: matches!(status.trim(), "Charging" | "Full"),
    })
}

/// "Wed 25 Nov  15:41", the format on the design canvas.
pub fn clock_text(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%a %-d %b  %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hyprland_json_becomes_workspaces() {
        let ws = json!([
            {"id": 3, "name": "notes", "windows": 1},
            {"id": 1, "name": "1", "windows": 4},
            {"id": -98, "name": "special:scratch", "windows": 2}
        ]);
        let d = parse_hypr(
            &ws,
            &json!({"id": 1}),
            &json!({"class": "org.gnome.Nautilus"}),
        );
        assert_eq!(
            d.workspaces.iter().map(|w| w.id).collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(d.workspaces[0].active && !d.workspaces[1].active);
        assert_eq!(d.stage.as_deref(), Some("Nautilus"));
    }

    #[test]
    fn empty_desktop_has_no_stage_title() {
        let d = parse_hypr(&json!([]), &json!({"id": 1}), &json!({}));
        assert_eq!(d.stage, None);
    }

    #[test]
    fn reads_a_fake_battery() {
        let root = std::env::temp_dir().join(format!("nordlys-bat-{}", std::process::id()));
        let bat = root.join("BAT0");
        let ac = root.join("ADP0");
        std::fs::create_dir_all(&bat).unwrap();
        std::fs::create_dir_all(&ac).unwrap();
        std::fs::write(ac.join("type"), "Mains\n").unwrap();
        std::fs::write(bat.join("type"), "Battery\n").unwrap();
        std::fs::write(bat.join("capacity"), "82\n").unwrap();
        std::fs::write(bat.join("status"), "Discharging\n").unwrap();
        assert_eq!(
            battery_in(&root),
            Some(Battery {
                percent: 82,
                charging: false
            })
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hyprland_shelf_is_this_workspace_in_focus_order() {
        let clients = json!([
            {"address": "0xa", "class": "foot", "title": "zsh", "workspace": {"id": 1}, "focusHistoryID": 2, "mapped": true},
            {"address": "0xb", "class": "helix", "title": "main.rs", "workspace": {"id": 1}, "focusHistoryID": 0, "mapped": true},
            {"address": "0xc", "class": "firefox", "title": "docs", "workspace": {"id": 1}, "focusHistoryID": 1, "mapped": true},
            {"address": "0xd", "class": "mpv", "title": "film", "workspace": {"id": 2}, "focusHistoryID": 3, "mapped": true},
            {"address": "0xe", "class": "x", "title": "hidden", "workspace": {"id": 1}, "focusHistoryID": 4, "hidden": true}
        ]);
        let shelf = parse_hypr_shelf(&clients, 1);
        let ids: Vec<&str> = shelf.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["0xc", "0xa"]);
        assert_eq!(shelf[0].app, "Firefox");
    }

    #[test]
    fn clock_matches_the_design() {
        use chrono::TimeZone;
        let t = chrono::Local
            .with_ymd_and_hms(2026, 11, 25, 15, 41, 0)
            .unwrap();
        assert_eq!(clock_text(t), "Wed 25 Nov  15:41");
    }
}
