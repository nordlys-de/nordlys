//! Installed applications, read from freedesktop `.desktop` files, and
//! starting programs so they outlive the launcher.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// Desktop file id, e.g. `org.gnome.Nautilus.desktop`.
    pub id: String,
    pub name: String,
    pub generic: Option<String>,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    /// Exec line with field codes (%f, %U…) removed.
    pub exec: String,
    pub terminal: bool,
}

impl App {
    /// Short app id for matching config groups: "org.gnome.Nautilus.desktop" → "nautilus".
    pub fn short_id(&self) -> String {
        let stem = self.id.strip_suffix(".desktop").unwrap_or(&self.id);
        stem.rsplit('.').next().unwrap_or(stem).to_lowercase()
    }

    pub fn launch(&self) -> std::io::Result<()> {
        if self.terminal {
            let term = terminal().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no terminal found; set $TERMINAL",
                )
            })?;
            spawn_shell(&format!("{term} -e sh -c {}", shell_quote(&self.exec)))
        } else {
            spawn_shell(&self.exec)
        }
    }
}

/// Every visible application, user entries overriding system ones, sorted by name.
pub fn installed() -> Vec<App> {
    let mut by_id: HashMap<String, App> = HashMap::new();
    // Highest priority first; the first entry seen for an id wins.
    for dir in data_dirs() {
        let root = dir.join("applications");
        let mut files = vec![];
        collect(&root, &mut files, 0);
        for path in files {
            let Some(id) = desktop_id(&root, &path) else {
                continue;
            };
            if by_id.contains_key(&id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // A hidden or invalid entry still claims its id, so it hides
            // the system copy of the same app.
            match parse(&id, &text) {
                Some(app) => by_id.insert(id, app),
                None => by_id.insert(id.clone(), App::placeholder(id)),
            };
        }
    }
    let mut apps: Vec<App> = by_id.into_values().filter(|a| !a.exec.is_empty()).collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

impl App {
    fn placeholder(id: String) -> Self {
        App {
            id,
            name: String::new(),
            generic: None,
            comment: None,
            keywords: vec![],
            exec: String::new(),
            terminal: false,
        }
    }
}

fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".local/share")));
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_home
        .into_iter()
        .chain(
            system
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        )
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 3 {
            collect(&p, out, depth + 1);
        } else if p.extension().is_some_and(|x| x == "desktop") {
            out.push(p);
        }
    }
}

/// `applications/kde/foo.desktop` has the id `kde-foo.desktop`.
fn desktop_id(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("-"),
    )
}

/// Parse the `[Desktop Entry]` group. None for anything that shouldn't be listed.
pub fn parse(id: &str, text: &str) -> Option<App> {
    let mut in_entry = false;
    let mut kv: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            kv.entry(k.trim()).or_insert(v.trim());
        }
    }
    let yes = |k: &str| kv.get(k).is_some_and(|v| *v == "true");
    if kv.get("Type").copied() != Some("Application") || yes("NoDisplay") || yes("Hidden") {
        return None;
    }
    if let Some(try_exec) = kv.get("TryExec")
        && !on_path(try_exec)
    {
        return None;
    }
    let exec = strip_field_codes(kv.get("Exec")?);
    let opt = |k: &str| kv.get(k).filter(|v| !v.is_empty()).map(|v| v.to_string());
    Some(App {
        id: id.to_owned(),
        name: opt("Name")?,
        generic: opt("GenericName"),
        comment: opt("Comment"),
        keywords: kv
            .get("Keywords")
            .map(|k| {
                k.split(';')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        exec,
        terminal: yes("Terminal"),
    })
}

/// Remove %f, %U and friends; `%%` becomes `%`.
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            // Any other code is dropped.
            if chars.next() == Some('%') {
                out.push('%');
            }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn on_path(prog: &str) -> bool {
    if prog.contains('/') {
        return Path::new(prog).exists();
    }
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
}

fn terminal() -> Option<String> {
    if let Ok(t) = std::env::var("TERMINAL")
        && !t.is_empty()
    {
        return Some(t);
    }
    [
        "foot",
        "ghostty",
        "kitty",
        "alacritty",
        "wezterm",
        "konsole",
        "xterm",
    ]
    .into_iter()
    .find(|t| on_path(t))
    .map(str::to_owned)
}

/// Run a command line detached: own process group, no inherited stdio.
pub fn spawn_shell(cmdline: &str) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    Command::new("sh")
        .arg("-c")
        .arg(format!("exec {cmdline}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
}

/// Open a file or folder with the user's default handler.
pub fn open_path(path: &Path) -> std::io::Result<()> {
    spawn_shell(&format!(
        "xdg-open {}",
        shell_quote(&path.to_string_lossy())
    ))
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "\
[Desktop Entry]
Name=Firefox
GenericName=Web Browser
Comment=Browse the World Wide Web
Keywords=Internet;WWW;Browser;
Exec=/usr/lib/firefox/firefox %u
Type=Application

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window %u
";

    #[test]
    fn parses_the_main_entry_only() {
        let a = parse("firefox.desktop", FIREFOX).unwrap();
        assert_eq!(a.name, "Firefox");
        assert_eq!(a.exec, "/usr/lib/firefox/firefox");
        assert_eq!(a.keywords, ["Internet", "WWW", "Browser"]);
        assert_eq!(a.generic.as_deref(), Some("Web Browser"));
    }

    #[test]
    fn hides_nodisplay_and_non_apps() {
        assert!(
            parse(
                "x",
                "[Desktop Entry]\nName=X\nExec=x\nType=Application\nNoDisplay=true"
            )
            .is_none()
        );
        assert!(
            parse(
                "x",
                "[Desktop Entry]\nName=X\nType=Link\nURL=https://example.org"
            )
            .is_none()
        );
        assert!(
            parse(
                "x",
                "[Desktop Entry]\nName=X\nExec=x\nType=Application\nTryExec=/nope/x"
            )
            .is_none()
        );
    }

    #[test]
    fn strips_field_codes() {
        assert_eq!(strip_field_codes("foo %F --bar %% %i"), "foo --bar %");
    }

    #[test]
    fn ids_follow_the_spec() {
        let root = Path::new("/usr/share/applications");
        assert_eq!(
            desktop_id(root, &root.join("kde/foo.desktop")).unwrap(),
            "kde-foo.desktop"
        );
        let a = App {
            id: "org.gnome.Nautilus.desktop".into(),
            ..App::placeholder(String::new())
        };
        assert_eq!(a.short_id(), "nautilus");
    }

    #[test]
    fn quoting_survives_apostrophes() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
