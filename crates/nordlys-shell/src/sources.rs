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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

    /// Workspaces and the stage, as `screen` sees them (its own active
    /// workspace), or as the focused screen does.
    pub fn desk(&self, screen: Option<&str>) -> Desk {
        match self {
            Compositor::Nordlys(p) => nordlys_desk(p, screen).unwrap_or_else(demo_desk),
            Compositor::Hyprland(p) => hypr_desk(p, screen).unwrap_or_default(),
            Compositor::Demo => demo_desk(),
        }
    }

    /// The screens' names, e.g. ["DP-1", "eDP-1"]. None in the demo, or
    /// from a nordlys-comp too old to list them.
    pub fn screens(&self) -> Option<Vec<String>> {
        match self {
            Compositor::Hyprland(p) => {
                let monitors: Value = serde_json::from_str(&hypr_request(p, "j/monitors")?).ok()?;
                Some(screen_names(&monitors))
            }
            Compositor::Nordlys(p) => {
                let names: Vec<String> = nordlys_state(p)?
                    .screens
                    .into_iter()
                    .map(|s| s.name)
                    .collect();
                (!names.is_empty()).then_some(names)
            }
            Compositor::Demo => None,
        }
    }

    pub fn switch_to(&self, ws: &Workspace) {
        match self {
            Compositor::Hyprland(p) => hypr_dispatch(
                p,
                &format!("hl.dsp.focus({{ workspace = {} }})", ws.id),
                &format!("workspace {}", ws.id),
            ),
            Compositor::Nordlys(p) => {
                if let Ok(n) = u32::try_from(ws.id) {
                    nordlys_send(p, &nordlys_ipc::Request::Workspace { n });
                }
            }
            Compositor::Demo => {}
        }
    }

    /// Go to the first empty workspace (Hyprland only; nordlys-comp places
    /// app groups itself).
    pub fn go_to_empty_workspace(&self) {
        if let Compositor::Hyprland(p) = self {
            hypr_dispatch(
                p,
                "hl.dsp.focus({ workspace = \"empty\" })",
                "workspace empty",
            );
        }
    }

    /// Windows waiting on this workspace's shelf, most recent first.
    ///
    /// On nordlys-comp this is the real shelf. On Hyprland it's the
    /// closest match: this workspace's windows in focus order, minus the
    /// focused one.
    pub fn shelf(&self, screen: Option<&str>) -> Vec<ShelfWindow> {
        match self {
            Compositor::Nordlys(p) => nordlys_state(p)
                .map(|s| {
                    on_screen(s, screen)
                        .shelf
                        .into_iter()
                        .map(|id| ShelfWindow {
                            app: pretty_app(&id),
                            title: id.clone(),
                            preview: Some(nordlys_ipc::preview_path(
                                &nordlys_ipc::preview_dir(p),
                                &id,
                            )),
                            id,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Compositor::Hyprland(p) => hypr_shelf(p, screen).unwrap_or_default(),
            Compositor::Demo => demo_shelf(),
        }
    }

    /// The focused window, measured on its own screen. None on a
    /// fullscreen or empty screen, or off Hyprland.
    pub fn active_window(&self) -> Option<Active> {
        let Compositor::Hyprland(p) = self else {
            return None;
        };
        let json =
            |cmd: &str| -> Option<Value> { serde_json::from_str(&hypr_request(p, cmd)?).ok() };
        parse_active(&json("j/activewindow")?, &json("j/monitors")?)
    }

    /// The pointer, in global coordinates. Hyprland only.
    pub fn pointer(&self) -> Option<(i32, i32)> {
        let Compositor::Hyprland(p) = self else {
            return None;
        };
        let cursor: Value = serde_json::from_str(&hypr_request(p, "j/cursorpos")?).ok()?;
        let int = |k| cursor.get(k).and_then(Value::as_i64).map(|n| n as i32);
        Some((int("x")?, int("y")?))
    }

    /// Ask a window to close.
    pub fn close(&self, id: &str) {
        match self {
            Compositor::Nordlys(p) => {
                nordlys_send(p, &nordlys_ipc::Request::Close { id: id.to_owned() })
            }
            Compositor::Hyprland(p) => hypr_dispatch(
                p,
                &format!("hl.dsp.window.close({{ window = \"address:{id}\" }})"),
                &format!("closewindow address:{id}"),
            ),
            Compositor::Demo => {}
        }
    }

    /// Put a shelf window on stage.
    pub fn bring(&self, w: &ShelfWindow) {
        match self {
            Compositor::Nordlys(p) => {
                nordlys_send(p, &nordlys_ipc::Request::Bring { id: w.id.clone() })
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

/// Where a window is on its screen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Frame {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The focused window and where its screen starts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Active {
    /// Relative to its screen.
    pub frame: Frame,
    /// The screen's top-left corner, in global coordinates.
    pub origin: (i32, i32),
    /// Floating windows move without telling anyone, so they're re-read.
    pub floating: bool,
    /// The screen's name, e.g. "DP-1".
    pub screen: String,
}

impl Active {
    /// A global pointer position, relative to this window's screen.
    pub fn local(&self, (x, y): (i32, i32)) -> (i32, i32) {
        (x - self.origin.0, y - self.origin.1)
    }
}

fn parse_active(window: &Value, monitors: &Value) -> Option<Active> {
    let int = |v: Option<&Value>| v.and_then(Value::as_i64).map(|n| n as i32);
    // `fullscreen` is a number (0 = no) or, on older Hyprland, a bool.
    let fullscreen = window.get("fullscreen");
    if fullscreen.is_some_and(|f| f.as_i64().is_some_and(|n| n != 0) || f.as_bool() == Some(true)) {
        return None;
    }
    let monitor = window.get("monitor")?.as_i64()?;
    let screen = monitors
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(Value::as_i64) == Some(monitor))?;
    let (sx, sy) = (int(screen.get("x"))?, int(screen.get("y"))?);
    Some(Active {
        frame: Frame {
            id: window.get("address")?.as_str()?.to_owned(),
            x: int(window.pointer("/at/0"))? - sx,
            y: int(window.pointer("/at/1"))? - sy,
            width: int(window.pointer("/size/0"))?,
            height: int(window.pointer("/size/1"))?,
        },
        origin: (sx, sy),
        floating: window.get("floating").and_then(Value::as_bool) == Some(true),
        screen: screen
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShelfWindow {
    /// Compositor's handle: window id on nordlys-comp, address on Hyprland.
    pub id: String,
    pub app: String,
    pub title: String,
    /// Where nordlys-comp keeps its live picture (see
    /// [`nordlys_ipc::preview_path`]); none elsewhere.
    pub preview: Option<PathBuf>,
}

fn demo_shelf() -> Vec<ShelfWindow> {
    let w = |id: &str, app: &str, title: &str| ShelfWindow {
        id: id.into(),
        app: app.into(),
        title: title.into(),
        preview: None,
    };
    vec![
        w("term", "Terminal", "zsh · ~/code/nordlys"),
        w("docs", "Firefox", "Space — smithay docs"),
        w("files", "Files", "Home › code › nordlys"),
    ]
}

fn hypr_shelf(sock: &Path, screen: Option<&str>) -> Option<Vec<ShelfWindow>> {
    let clients: Value = serde_json::from_str(&hypr_request(sock, "j/clients")?).ok()?;
    Some(parse_hypr_shelf(
        &clients,
        hypr_active_workspace(sock, screen)?,
    ))
}

/// The workspace on show: on `screen`, or on the focused screen.
fn hypr_active_workspace(sock: &Path, screen: Option<&str>) -> Option<i64> {
    match screen {
        Some(name) => {
            let monitors: Value = serde_json::from_str(&hypr_request(sock, "j/monitors")?).ok()?;
            workspace_on(&monitors, name)
        }
        None => {
            let active: Value =
                serde_json::from_str(&hypr_request(sock, "j/activeworkspace")?).ok()?;
            active.get("id").and_then(Value::as_i64)
        }
    }
}

fn workspace_on(monitors: &Value, screen: &str) -> Option<i64> {
    monitors
        .as_array()?
        .iter()
        .find(|m| m.get("name").and_then(Value::as_str) == Some(screen))?
        .pointer("/activeWorkspace/id")?
        .as_i64()
}

fn screen_names(monitors: &Value) -> Vec<String> {
    monitors
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("name")?.as_str().map(str::to_owned))
        .collect()
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
                    preview: None,
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
    // Read just the one line: window names can be any UTF-8.
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(s), &mut line).ok()?;
    match nordlys_ipc::from_line(&line).ok()? {
        Response::Ok { state } => Some(state),
        _ => None,
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

fn nordlys_desk(sock: &Path, screen: Option<&str>) -> Option<Desk> {
    Some(desk_from_stage(on_screen(nordlys_state(sock)?, screen)))
}

/// The state as `screen` sees it: its workspace taken as the active one,
/// and its own stage and shelf. Unchanged for no screen, or one the
/// compositor doesn't list.
fn on_screen(mut state: nordlys_ipc::StageState, screen: Option<&str>) -> nordlys_ipc::StageState {
    let Some(s) = screen.and_then(|name| state.screens.iter().find(|s| s.name == name)) else {
        return state;
    };
    state.workspace = s.workspace;
    state.main = s.main.clone();
    state.side = s.side.clone();
    state.shelf = s.shelf.clone();
    // An empty workspace on screen isn't listed with the busy ones.
    if !state.workspaces.iter().any(|w| w.n == s.workspace) {
        let at = state.workspaces.partition_point(|w| w.n < s.workspace);
        state.workspaces.insert(
            at,
            nordlys_ipc::WorkspaceState {
                n: s.workspace,
                windows: 0,
            },
        );
    }
    state
}

fn desk_from_stage(state: nordlys_ipc::StageState) -> Desk {
    let mut workspaces: Vec<Workspace> = state
        .workspaces
        .iter()
        .map(|w| Workspace {
            id: w.n.into(),
            name: w.n.to_string(),
            active: w.n == state.workspace,
            windows: w.windows,
        })
        .collect();
    // An older nordlys-comp doesn't list workspaces.
    if workspaces.is_empty() {
        workspaces.push(Workspace {
            id: state.workspace.into(),
            name: state.workspace.to_string(),
            active: true,
            windows: u32::from(state.main.is_some()) + state.shelf.len() as u32,
        });
    }
    Desk {
        workspaces,
        stage: match (state.main, state.side) {
            (Some(m), Some(s)) => Some(format!("{} + {}", pretty_app(&m), pretty_app(&s))),
            (m, _) => m.as_deref().map(pretty_app),
        },
    }
}

/// Send one request to nordlys-comp and forget the answer.
fn nordlys_send(sock: &Path, req: &nordlys_ipc::Request) {
    if let Ok(mut s) = UnixStream::connect(sock) {
        let _ = s.write_all(nordlys_ipc::to_line(req).as_bytes());
    }
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

fn hypr_desk(sock: &Path, screen: Option<&str>) -> Option<Desk> {
    let workspaces: Value = serde_json::from_str(&hypr_request(sock, "j/workspaces")?).ok()?;
    let active = hypr_active_workspace(sock, screen);
    let window: Value = hypr_request(sock, "j/activewindow")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    Some(parse_hypr(&workspaces, active, &window))
}

fn parse_hypr(workspaces: &Value, active_id: Option<i64>, window: &Value) -> Desk {
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
    // nordlys-comp numbers a second window of an app: "foot#2".
    let class = class.split('#').next().unwrap_or(class);
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
pub fn clock_text(now: chrono::DateTime<chrono::Local>, format: &str) -> String {
    now.format(format).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_active_window_is_measured_from_its_screen() {
        let window = json!({"address": "0xab", "monitor": 1, "fullscreen": 0,
                            "at": [2020, 62], "size": [800, 600]});
        let monitors = json!([{"id": 0, "x": 0, "y": 0}, {"id": 1, "x": 1920, "y": 0}]);
        let a = parse_active(&window, &monitors).unwrap();
        let f = &a.frame;
        assert_eq!((f.x, f.y, f.width, f.height), (100, 62, 800, 600));
        assert_eq!(a.local((2700, 80)), (780, 80));
        assert!(!a.floating);
        // Nothing focused, or fullscreen: no button.
        assert!(parse_active(&json!({}), &monitors).is_none());
        let full = json!({"address": "0xab", "monitor": 1, "fullscreen": 2,
                          "at": [1920, 0], "size": [800, 600]});
        assert!(parse_active(&full, &monitors).is_none());
    }

    #[test]
    fn hyprland_json_becomes_workspaces() {
        let ws = json!([
            {"id": 3, "name": "notes", "windows": 1},
            {"id": 1, "name": "1", "windows": 4},
            {"id": -98, "name": "special:scratch", "windows": 2}
        ]);
        let d = parse_hypr(&ws, Some(1), &json!({"class": "org.gnome.Nautilus"}));
        assert_eq!(
            d.workspaces.iter().map(|w| w.id).collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(d.workspaces[0].active && !d.workspaces[1].active);
        assert_eq!(d.stage.as_deref(), Some("Nautilus"));
    }

    #[test]
    fn nordlys_state_becomes_workspaces() {
        use nordlys_ipc::{StageState, WorkspaceState};
        let d = desk_from_stage(StageState {
            main: Some("helix".into()),
            side: Some("foot#2".into()),
            shelf: vec![],
            workspace: 2,
            workspaces: vec![
                WorkspaceState { n: 1, windows: 3 },
                WorkspaceState { n: 2, windows: 2 },
            ],
            screens: vec![],
        });
        assert_eq!(d.workspaces.len(), 2);
        assert!(d.workspaces[1].active && !d.workspaces[0].active);
        assert_eq!(d.stage.as_deref(), Some("Helix + Foot"));
    }

    #[test]
    fn each_nordlys_screen_sees_its_own_workspace() {
        use nordlys_ipc::{ScreenState, StageState, WorkspaceState};
        let state = StageState {
            main: Some("helix".into()),
            side: None,
            shelf: vec!["foot".into()],
            workspace: 1,
            workspaces: vec![WorkspaceState { n: 1, windows: 2 }],
            screens: vec![
                ScreenState {
                    name: "eDP-1".into(),
                    workspace: 1,
                    focused: true,
                    main: Some("helix".into()),
                    side: None,
                    shelf: vec!["foot".into()],
                },
                ScreenState {
                    name: "DP-1".into(),
                    workspace: 2,
                    focused: false,
                    main: None,
                    side: None,
                    shelf: vec![],
                },
            ],
        };
        let d = desk_from_stage(on_screen(state.clone(), Some("DP-1")));
        assert_eq!(
            d.workspaces.iter().map(|w| w.id).collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(d.workspaces[1].active && !d.workspaces[0].active);
        assert_eq!(d.stage, None);
        assert!(on_screen(state.clone(), Some("DP-1")).shelf.is_empty());
        assert_eq!(on_screen(state.clone(), None), state);
        assert_eq!(on_screen(state.clone(), Some("HDMI-A-9")), state);
    }

    #[test]
    fn each_screen_has_its_own_workspace() {
        let monitors = json!([
            {"id": 0, "name": "eDP-1", "activeWorkspace": {"id": 1, "name": "1"}},
            {"id": 1, "name": "DP-2", "activeWorkspace": {"id": 4, "name": "4"}}
        ]);
        assert_eq!(screen_names(&monitors), ["eDP-1", "DP-2"]);
        assert_eq!(workspace_on(&monitors, "DP-2"), Some(4));
        assert_eq!(workspace_on(&monitors, "HDMI-A-1"), None);
    }

    #[test]
    fn empty_desktop_has_no_stage_title() {
        let d = parse_hypr(&json!([]), Some(1), &json!({}));
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
        let mut clock = nordlys_config::Clock::default();
        assert_eq!(clock_text(t, &clock.bar_format()), "Wed 25 Nov  15:41");
        clock.twelve_hour = true;
        clock.date = false;
        assert_eq!(clock_text(t, &clock.bar_format()), "3:41 PM");
    }
}
