//! What the launcher can find (apps, Nordlys commands, files, clipboard
//! history), how a query ranks them, and what happens on Enter.

use crate::apps::{self, App};
use crate::files;
use crate::icons;
use nordlys_config::Config;
use nordlys_ipc::{Request, Response};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    App,
    Command,
    File,
    Clip,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::App => "Apps",
            Kind::Command => "Commands",
            Kind::File => "Files",
            Kind::Clip => "Clipboard",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All,
    Commands,
    Files,
    /// Only here: copied text never shows in an ordinary search.
    Clipboard,
}

impl Mode {
    /// `> two` searches commands, `/ toml` files, `# link` the clipboard.
    pub fn split(query: &str) -> (Mode, &str) {
        let q = query.trim_start();
        if let Some(rest) = q.strip_prefix('>') {
            (Mode::Commands, rest.trim())
        } else if let Some(rest) = q.strip_prefix('/') {
            (Mode::Files, rest.trim())
        } else if let Some(rest) = q.strip_prefix('#') {
            (Mode::Clipboard, rest.trim())
        } else {
            (Mode::All, q.trim())
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    App(usize),
    Ipc(Request),
    Group(String),
    Shell(String),
    /// A shell command that asks first: the launcher runs it on a second
    /// Enter. `verb` finishes "Press Enter again to …".
    Confirm {
        cmd: String,
        verb: &'static str,
    },
    Open(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub desc: String,
    pub key: Option<String>,
    pub action: Action,
    /// Icon for commands and files; apps get their letter tile.
    pub icon: Option<char>,
    /// Extra lower-case words to match on (keywords, generic name, id).
    terms: Vec<String>,
}

impl Item {
    /// Stable key for launch history.
    pub fn history_key(&self) -> String {
        match &self.action {
            Action::App(_) => format!("app:{}", self.name),
            // Copied text stays out of the launch history.
            _ if self.kind == Kind::Clip => String::new(),
            Action::Open(p) => format!("file:{}", p.display()),
            _ => format!("cmd:{}", self.name),
        }
    }
}

pub struct Catalog {
    pub apps: Vec<App>,
    items: Vec<Item>,
    files: Vec<PathBuf>,
    /// Clipboard history, newest first: cliphist's id and a preview.
    clips: Vec<(u64, String)>,
    history: HashMap<String, u32>,
    config: Config,
}

impl Catalog {
    pub fn load(config: &Config) -> Self {
        Self::new(apps::installed(), config.clone(), load_history())
    }

    pub fn new(apps: Vec<App>, config: Config, history: HashMap<String, u32>) -> Self {
        let mut items: Vec<Item> = apps
            .iter()
            .enumerate()
            .map(|(i, a)| Item {
                kind: Kind::App,
                name: a.name.clone(),
                desc: a
                    .comment
                    .clone()
                    .or_else(|| a.generic.clone())
                    .unwrap_or_default(),
                key: None,
                action: Action::App(i),
                icon: None,
                terms: a
                    .keywords
                    .iter()
                    .cloned()
                    .chain(a.generic.clone())
                    .chain([a.short_id()])
                    .map(|s| s.to_lowercase())
                    .collect(),
            })
            .collect();
        items.extend(commands(&config));
        Self {
            apps,
            items,
            files: vec![],
            clips: vec![],
            history,
            config,
        }
    }

    pub fn set_files(&mut self, files: Vec<PathBuf>) {
        self.files = files;
    }

    pub fn files_ready(&self) -> bool {
        !self.files.is_empty()
    }

    pub fn set_clips(&mut self, clips: Vec<(u64, String)>) {
        self.clips = clips;
    }

    /// Ranked results, grouped Apps → Commands → Files.
    pub fn search(&self, query: &str) -> Vec<Item> {
        let (mode, q) = Mode::split(query);
        let q = q.to_lowercase();
        if mode == Mode::Clipboard {
            return self.best_clips(&q, 10);
        }
        let (apps, cmds, file_cap) = match mode {
            Mode::All => (4, 3, if q.chars().count() >= 2 { 3 } else { 0 }),
            Mode::Commands => (0, 9, 0),
            Mode::Files | Mode::Clipboard => (0, 0, 9),
        };
        let mut out = self.best(Kind::App, &q, apps);
        out.extend(self.best(Kind::Command, &q, cmds));
        if file_cap > 0 {
            out.extend(self.best_files(&q, file_cap));
        }
        out
    }

    fn best(&self, kind: Kind, q: &str, cap: usize) -> Vec<Item> {
        if cap == 0 {
            return vec![];
        }
        let mut scored: Vec<(i32, &Item)> = self
            .items
            .iter()
            .filter(|it| it.kind == kind)
            .filter_map(|it| {
                let s = if q.is_empty() {
                    Some(0)
                } else {
                    item_score(q, it)
                }?;
                let used = self.history.get(&it.history_key()).copied().unwrap_or(0) as i32;
                Some((s + (used * 4).min(40), it))
            })
            .collect();
        // Higher score first; ties keep alphabetical (apps) or declared (commands) order.
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored
            .into_iter()
            .take(cap)
            .map(|(_, it)| it.clone())
            .collect()
    }

    fn best_files(&self, q: &str, cap: usize) -> Vec<Item> {
        if q.is_empty() {
            return vec![];
        }
        let mut scored: Vec<(i32, &PathBuf)> = self
            .files
            .iter()
            .filter_map(|p| {
                let name = p.file_name()?.to_string_lossy().to_lowercase();
                // Shallow paths win ties: ~/notes.md before ~/a/b/c/notes.md.
                score(q, &name).map(|s| (s - p.components().count() as i32, p))
            })
            .collect();
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored
            .into_iter()
            .take(cap)
            .map(|(_, p)| Item {
                kind: Kind::File,
                name: p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                desc: files::pretty_dir(p),
                key: None,
                action: Action::Open(p.clone()),
                icon: Some(icons::FILE),
                terms: vec![],
            })
            .collect()
    }

    /// Newest first; a query filters them, best match first.
    fn best_clips(&self, q: &str, cap: usize) -> Vec<Item> {
        let mut scored: Vec<(i32, usize)> = self
            .clips
            .iter()
            .enumerate()
            .filter_map(|(i, (_, text))| {
                let lower = text.to_lowercase();
                let s = if q.is_empty() {
                    0
                } else if lower.contains(q) {
                    100
                } else {
                    score(q, &lower)?
                };
                // Recency breaks ties.
                Some((s, i))
            })
            .collect();
        scored.sort_by_key(|&(s, i)| (std::cmp::Reverse(s), i));
        scored
            .into_iter()
            .take(cap)
            .map(|(_, i)| {
                let (id, text) = &self.clips[i];
                let image = text.starts_with("[[ binary data");
                Item {
                    kind: Kind::Clip,
                    name: one_line(text),
                    desc: if image {
                        "Image".into()
                    } else {
                        format!("{} characters", text.chars().count())
                    },
                    key: None,
                    // The id is a number, so it's safe in a shell command.
                    action: Action::Shell(format!(
                        "printf '%s\\t\\n' {id} | cliphist decode | wl-copy"
                    )),
                    icon: Some(if image { icons::EYE } else { icons::FILE }),
                    terms: vec![],
                }
            })
            .collect()
    }

    /// Carry out an item. `alt` is Ctrl+Enter: open a file's folder instead.
    pub fn run(&mut self, item: &Item, alt: bool) -> Result<(), String> {
        let result = match &item.action {
            Action::App(i) => self.apps[*i].launch().map_err(|e| e.to_string()),
            Action::Ipc(req) => stage_request(req.clone()),
            Action::Group(name) => self.launch_group(name),
            Action::Shell(cmd) | Action::Confirm { cmd, .. } => {
                apps::spawn_shell(cmd).map_err(|e| e.to_string())
            }
            Action::Open(p) => {
                let target = if alt { p.parent().unwrap_or(p) } else { p };
                apps::open_path(target).map_err(|e| e.to_string())
            }
        };
        if result.is_ok() && item.kind != Kind::Clip {
            *self.history.entry(item.history_key()).or_default() += 1;
            save_history(&self.history);
        }
        result
    }

    /// Ask nordlys-comp to launch the group; without it, start the apps in order.
    fn launch_group(&self, name: &str) -> Result<(), String> {
        if ipc(Request::Group {
            name: name.to_owned(),
        })
        .is_ok()
        {
            return Ok(());
        }
        let group = self
            .config
            .group(name)
            .ok_or(format!("no group {name:?}"))?;
        // On Hyprland, a new workspace means its first empty one; the apps
        // then map there. Elsewhere they open where you are.
        if group.open_in == nordlys_config::OpenIn::NewWorkspace {
            crate::sources::Compositor::detect().go_to_empty_workspace();
        }
        let dir = group.dir_path();
        for id in &group.apps {
            let wanted = id.to_lowercase();
            match self
                .apps
                .iter()
                .find(|a| a.short_id() == wanted || a.name.to_lowercase() == wanted)
            {
                Some(app) => app.launch_in(dir.as_deref()).map_err(|e| e.to_string())?,
                None => apps::spawn_shell_in(id, dir.as_deref()).map_err(|e| e.to_string())?,
            }
        }
        Ok(())
    }
}

fn commands(config: &Config) -> Vec<Item> {
    let cmd =
        |icon: char, name: &str, desc: &str, key: &str, action: Action, terms: &[&str]| Item {
            kind: Kind::Command,
            icon: Some(icon),
            name: name.into(),
            desc: desc.into(),
            key: (!key.is_empty()).then(|| key.into()),
            action,
            terms: terms.iter().map(|t| t.to_string()).collect(),
        };
    let mut out: Vec<Item> = config
        .groups
        .iter()
        .map(|g| {
            let first = g.apps.first().cloned().unwrap_or_default();
            let rest = g.apps[1.min(g.apps.len())..].join(", ");
            let desc = if rest.is_empty() {
                format!("{first} on stage")
            } else {
                format!("{first} on stage · {rest} on the shelf")
            };
            let key = g
                .key
                .map(|k| format!("Super G {}", k.to_ascii_uppercase()))
                .unwrap_or_default();
            cmd(
                icons::GRID,
                &format!("Launch group: {}", g.name),
                &desc,
                &key,
                Action::Group(g.name.clone()),
                &["group", "workspace"],
            )
        })
        .collect();
    let config_path = nordlys_config::default_path();
    out.extend([
        cmd(
            icons::TWO_UP,
            "Two-up stage",
            "Pull the latest shelf window beside the stage",
            "Super T",
            Action::Ipc(Request::TwoUp),
            &["split", "side"],
        ),
        cmd(
            icons::SHELF,
            "Send window to shelf",
            "Park the stage window; it waits most-recent-first",
            "Super S",
            Action::Ipc(Request::SendToShelf),
            &["park", "hide", "minimize"],
        ),
        cmd(
            icons::HISTORY,
            "Previous window",
            "Bring back the window you just left",
            "Super K",
            Action::Ipc(Request::Previous),
            &["back", "last", "switch"],
        ),
        cmd(
            icons::SWAP,
            "Swap panes",
            "Trade places between main and side",
            "Super Tab",
            Action::Ipc(Request::Swap),
            &["switch", "two-up"],
        ),
        cmd(
            icons::REFRESH,
            "Reload config",
            "~/.config/nordlys/nordlys.toml",
            "",
            Action::Ipc(Request::Reload),
            &["refresh", "settings"],
        ),
        cmd(
            icons::SETTINGS,
            "Edit Nordlys config",
            &files::pretty_dir(&config_path.join("x")),
            "",
            Action::Open(config_path.clone()),
            &["settings", "toml", "preferences"],
        ),
        cmd(
            icons::LOCK,
            "Lock screen",
            "Lock now; your password unlocks",
            "Super L",
            Action::Shell(lock_command()),
            &["away", "screensaver"],
        ),
        cmd(
            icons::HISTORY,
            "Clipboard history",
            "Copy something you copied before",
            "Super V",
            Action::Shell(format!("{} --query '# '", launcher_command())),
            &["paste", "copy", "clipboard"],
        ),
        cmd(
            icons::MONITOR,
            "Record a region",
            "Pick part of the screen; the same key or the bar's dot stops it",
            "Super R",
            Action::Shell("nordlys-record region".into()),
            &["screen recording", "video", "capture", "screencast"],
        ),
        cmd(
            icons::MONITOR,
            "Record the screen",
            "Saved to ~/Videos/Recordings",
            "Super Shift R",
            Action::Shell("nordlys-record screen".into()),
            &["screen recording", "video", "capture", "screencast"],
        ),
        cmd(
            icons::WINDOW,
            "Welcome and keys",
            "The Stage, the shelf and the keys that move them",
            "Super F1",
            Action::Shell(format!("{} welcome", shell_exe())),
            &["help", "tour", "shortcuts", "keyboard", "intro"],
        ),
        cmd(
            icons::SETTINGS,
            "Settings",
            "Network, sound, display, appearance and more",
            "Super ,",
            Action::Shell(settings_command(&config_path)),
            &["preferences", "control", "system"],
        ),
    ]);
    let power = |icon, name: &str, desc: &str, cmd: &str, verb, terms: &[&str]| Item {
        kind: Kind::Command,
        icon: Some(icon),
        name: name.into(),
        desc: desc.into(),
        key: None,
        action: Action::Confirm {
            cmd: cmd.into(),
            verb,
        },
        terms: terms.iter().map(|t| t.to_string()).collect(),
    };
    out.extend([
        power(
            icons::CLOSE,
            "Clear clipboard history",
            "Forget everything copied so far",
            "cliphist wipe",
            "clear the clipboard history",
            &["wipe", "forget", "paste", "clipboard"],
        ),
        power(
            icons::ARROW_RIGHT,
            "Log out",
            "End the session; unsaved work is lost",
            LOG_OUT,
            "log out",
            &["logout", "sign out", "exit", "session"],
        ),
        power(
            icons::MOON,
            "Suspend",
            "Sleep now; the lock screen greets you",
            "systemctl suspend",
            "suspend",
            &["sleep", "power"],
        ),
        power(
            icons::RESTART,
            "Restart",
            "Reboot the computer",
            "systemctl reboot",
            "restart",
            &["reboot", "power"],
        ),
        power(
            icons::POWER,
            "Shut down",
            "Power off the computer",
            "systemctl poweroff",
            "shut down",
            &["power off", "poweroff", "halt", "turn off"],
        ),
    ]);
    out
}

/// Hyprland's own exit when it's there, else ask logind to end the session.
const LOG_OUT: &str = "if [ -n \"$HYPRLAND_INSTANCE_SIGNATURE\" ]; \
     then hyprctl eval 'hl.dispatch(hl.dsp.exit())'; \
     else loginctl terminate-session \"$XDG_SESSION_ID\"; fi";

/// The Settings app when it's installed, else the config file.
fn settings_command(config: &std::path::Path) -> String {
    format!(
        "command -v nordlys-settings >/dev/null && exec nordlys-settings || exec xdg-open {}",
        apps::shell_quote(&config.to_string_lossy())
    )
}

/// This same binary, quoted, so it works without anything on PATH.
fn shell_exe() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "nordlys-shell".into());
    apps::shell_quote(&exe)
}

fn lock_command() -> String {
    format!("{} lock", shell_exe())
}

fn launcher_command() -> String {
    format!("{} launcher", shell_exe())
}

/// Clipboard history from cliphist, newest first. Empty without it.
pub fn clip_history() -> Vec<(u64, String)> {
    let Ok(out) = std::process::Command::new("cliphist")
        .arg("list")
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (id, text) = l.split_once('\t')?;
            Some((id.trim().parse().ok()?, text.to_owned()))
        })
        .take(500)
        .collect()
}

/// The first line, trimmed and cut short, for a row.
fn one_line(s: &str) -> String {
    let line = s.trim().lines().next().unwrap_or("").trim();
    let mut out: String = line.chars().take(90).collect();
    if out.chars().count() < line.chars().count() || s.trim().lines().count() > 1 {
        out.push('…');
    }
    if out.is_empty() {
        "(blank)".into()
    } else {
        out
    }
}

fn item_score(q: &str, it: &Item) -> Option<i32> {
    let name = score(q, &it.name.to_lowercase()).map(|s| s + 20);
    let terms = it.terms.iter().filter_map(|t| score(q, t)).max();
    let desc = it.desc.to_lowercase().contains(q).then_some(4);
    [name, terms, desc].into_iter().flatten().max()
}

/// Fuzzy subsequence score: every query character must appear in order.
/// Rewards exact and prefix matches, word starts and runs of adjacent hits.
pub fn score(q: &str, s: &str) -> Option<i32> {
    if q.is_empty() {
        return Some(0);
    }
    if s == q {
        return Some(200);
    }
    if s.starts_with(q) {
        return Some(150 - s.len().min(50) as i32);
    }
    let sc: Vec<char> = s.chars().collect();
    let mut total = 0;
    let mut pos = 0;
    let mut prev: Option<usize> = None;
    for qc in q.chars() {
        let found = (pos..sc.len()).find(|&i| sc[i] == qc)?;
        let word_start = found == 0 || !sc[found - 1].is_alphanumeric();
        total += 1;
        if word_start {
            total += 15;
        }
        match prev {
            Some(p) if found == p + 1 => total += 10,
            Some(p) => total -= ((found - p - 1) as i32).min(5),
            None => total -= (found as i32).min(10),
        }
        prev = Some(found);
        pos = found + 1;
    }
    Some(total)
}

/// A Stage command: for nordlys-comp, or on Hyprland for the session's
/// own Lua functions (session/hyprland.lua).
fn stage_request(req: Request) -> Result<(), String> {
    let lua = hyprland_stage(&req);
    match (ipc(req), lua) {
        (Err(_), Some(lua)) if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() => {
            hyprctl(&lua)
        }
        (result, _) => result,
    }
}

/// The `hyprctl` arguments that do the same as `req`.
fn hyprland_stage(req: &Request) -> Option<[&'static str; 2]> {
    let call = match req {
        Request::TwoUp => "nordlys_stage.two_up()",
        Request::SendToShelf => "nordlys_stage.send_to_shelf()",
        Request::Previous => "nordlys_stage.previous()",
        Request::Swap => "nordlys_stage.swap()",
        Request::Reload => return Some(["reload", "config-only"]),
        _ => return None,
    };
    Some(["eval", call])
}

fn hyprctl(args: &[&str; 2]) -> Result<(), String> {
    let out = std::process::Command::new("hyprctl")
        .args(args)
        .output()
        .map_err(|e| format!("hyprctl: {e}"))?;
    let reply = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if out.status.success() && (reply.is_empty() || reply == "ok") {
        Ok(())
    } else {
        Err(format!("Hyprland said: {reply}"))
    }
}

/// Send one request to nordlys-comp.
fn ipc(req: Request) -> Result<(), String> {
    let path = nordlys_ipc::socket_path();
    let mut s = UnixStream::connect(&path).map_err(|_| "nordlys-comp isn't running".to_owned())?;
    s.set_read_timeout(Some(Duration::from_millis(500))).ok();
    s.write_all(nordlys_ipc::to_line(&req).as_bytes())
        .map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(s)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    match nordlys_ipc::from_line::<Response>(&line).map_err(|e| e.to_string())? {
        Response::Ok { .. } | Response::Version { .. } => Ok(()),
        Response::Error { message } => Err(message),
    }
}

fn history_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("nordlys").join("launcher-history"))
}

fn load_history() -> HashMap<String, u32> {
    let Some(text) = history_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|l| {
            let (n, key) = l.split_once('\t')?;
            Some((key.to_owned(), n.parse().ok()?))
        })
        .collect()
}

fn save_history(h: &HashMap<String, u32>) {
    let Some(path) = history_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut rows: Vec<_> = h.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    let body: String = rows
        .iter()
        .take(500)
        .map(|(k, n)| format!("{n}\t{k}\n"))
        .collect();
    let _ = std::fs::write(path, body);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, id: &str, keywords: &[&str]) -> App {
        App {
            id: format!("{id}.desktop"),
            name: name.into(),
            generic: None,
            comment: Some(format!("{name} app")),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            exec: id.into(),
            terminal: false,
            path: None,
        }
    }

    fn catalog() -> Catalog {
        let config = Config::from_toml(include_str!("../../../config/nordlys.toml")).unwrap();
        let apps = vec![
            app("Files", "org.gnome.Nautilus", &["folder", "explorer"]),
            app("Firefox", "firefox", &["browser", "web"]),
            app("foot", "foot", &["terminal"]),
            app("Helix", "helix", &["editor", "text"]),
        ];
        Catalog::new(apps, config, HashMap::new())
    }

    fn names(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.name.as_str()).collect()
    }

    #[test]
    fn prefix_beats_scattered() {
        assert!(score("fi", "firefox") > score("fi", "nautilus files"));
        assert!(score("ff", "firefox").is_some());
        assert_eq!(score("zz", "firefox"), None);
    }

    #[test]
    fn keywords_find_apps() {
        let c = catalog();
        assert_eq!(names(&c.search("browser"))[0], "Firefox");
        assert_eq!(names(&c.search("terminal"))[0], "foot");
        assert_eq!(names(&c.search("nautilus"))[0], "Files");
    }

    #[test]
    fn prefixes_pick_a_mode() {
        let c = catalog();
        let r = c.search("> two");
        assert!(r.iter().all(|i| i.kind == Kind::Command));
        assert_eq!(r[0].name, "Two-up stage");
        assert_eq!(Mode::split(" / notes"), (Mode::Files, "notes"));
    }

    #[test]
    fn clipboard_only_in_its_mode() {
        let mut c = catalog();
        c.set_clips(vec![
            (3, "https://example.org".into()),
            (2, "firefox config".into()),
        ]);
        assert!(c.search("firefox").iter().all(|i| i.kind != Kind::Clip));
        let r = c.search("#");
        assert_eq!(names(&r), vec!["https://example.org", "firefox config"]);
        let r = c.search("# fire");
        assert_eq!(r[0].name, "firefox config");
        assert_eq!(
            r[0].action,
            Action::Shell("printf '%s\\t\\n' 2 | cliphist decode | wl-copy".into())
        );
        assert_eq!(r[0].history_key(), "");
    }

    #[test]
    fn groups_become_commands_with_their_keys() {
        let c = catalog();
        let g = c.search("> nordlys dev").into_iter().next().unwrap();
        assert_eq!(g.name, "Launch group: Nordlys dev");
        assert_eq!(g.key.as_deref(), Some("Super G D"));
        assert_eq!(g.desc, "helix on stage · foot, nautilus on the shelf");
    }

    #[test]
    fn history_lifts_favourites() {
        let mut c = catalog();
        assert_eq!(names(&c.search("fi"))[0], "Files");
        c.history.insert("app:Firefox".into(), 9);
        assert_eq!(names(&c.search("fi"))[0], "Firefox");
    }

    #[test]
    fn files_need_two_letters_in_all_mode() {
        let mut c = catalog();
        c.set_files(vec![
            PathBuf::from("/home/u/notes.md"),
            PathBuf::from("/home/u/a/b/notes.md"),
        ]);
        assert!(c.search("n").iter().all(|i| i.kind != Kind::File));
        let r = c.search("/ notes");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].action, Action::Open(PathBuf::from("/home/u/notes.md")));
    }

    #[test]
    fn power_commands_ask_before_they_run() {
        let config = Config::from_toml(include_str!("../../../config/nordlys.toml")).unwrap();
        let all = commands(&config);
        for name in ["Log out", "Suspend", "Restart", "Shut down"] {
            let it = all.iter().find(|i| i.name == name).expect(name);
            assert!(
                matches!(it.action, Action::Confirm { .. }),
                "{name} runs without asking"
            );
        }
        let settings = all.iter().find(|i| i.name == "Settings").unwrap();
        assert!(matches!(&settings.action, Action::Shell(c) if c.contains("nordlys-settings")));
    }
}
