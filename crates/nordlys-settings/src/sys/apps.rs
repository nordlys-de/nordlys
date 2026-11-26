//! Installed apps from their .desktop files: which opens what (xdg-mime)
//! and which start when you log in (XDG autostart).

use super::{config_home, run};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    /// `firefox.desktop`
    pub id: String,
    pub name: String,
    pub exec: String,
    pub comment: String,
    pub mime: Vec<String>,
    pub categories: Vec<String>,
    pub hidden: bool,
    pub path: PathBuf,
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.to_lowercase().cmp(&other.name.to_lowercase())
    }
}

impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// The `[Desktop Entry]` group of a .desktop file. None for files that
/// aren't apps.
pub fn parse_entry(id: &str, path: &Path, text: &str) -> Option<Entry> {
    let lang = std::env::var("LANG").unwrap_or_default();
    let lang = lang.split(['.', '@']).next().unwrap_or("");
    let short = lang.split('_').next().unwrap_or("");
    let mut e = Entry {
        id: id.to_owned(),
        path: path.to_owned(),
        ..Entry::default()
    };
    let mut localized_name = None;
    let mut in_main = false;
    let mut kind = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "Type" => kind = v.to_owned(),
            "Name" => e.name = v.to_owned(),
            k if k == format!("Name[{lang}]")
                || (localized_name.is_none() && k == format!("Name[{short}]")) =>
            {
                localized_name = Some(v.to_owned())
            }
            "Exec" => e.exec = v.to_owned(),
            "Comment" => e.comment = v.to_owned(),
            "MimeType" => e.mime = list(v),
            "Categories" => e.categories = list(v),
            "Hidden" => e.hidden |= v == "true",
            "NoDisplay" => {}
            "X-GNOME-Autostart-enabled" => e.hidden |= v == "false",
            _ => {}
        }
    }
    if let Some(n) = localized_name {
        e.name = n;
    }
    (kind == "Application" && !e.name.is_empty()).then_some(e)
}

fn list(v: &str) -> Vec<String> {
    v.split(';')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| super::home().join(".local/share"));
    let system =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    std::iter::once(home)
        .chain(
            system
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        )
        .collect()
}

/// Every app, the first folder winning for each id, as the spec says.
pub fn installed() -> Vec<Entry> {
    let mut seen: BTreeMap<String, Entry> = BTreeMap::new();
    for dir in data_dirs() {
        scan(&dir.join("applications"), "", &mut seen);
    }
    let mut v: Vec<Entry> = seen.into_values().filter(|e| !e.hidden).collect();
    v.sort();
    v
}

fn scan(dir: &Path, prefix: &str, seen: &mut BTreeMap<String, Entry>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for f in rd.flatten() {
        let path = f.path();
        let name = f.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            scan(&path, &format!("{prefix}{name}-"), seen);
        } else if let Some(stem) = name.strip_suffix(".desktop") {
            let id = format!("{prefix}{stem}.desktop");
            if seen.contains_key(&id) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Some(e) = parse_entry(&id, &path, &text)
            {
                seen.insert(id, e);
            }
        }
    }
}

/// A kind of file or link, and the types that make it up.
pub struct Kind {
    pub label: &'static str,
    pub mimes: &'static [&'static str],
}

pub const KINDS: &[Kind] = &[
    Kind {
        label: "Web browser",
        mimes: &[
            "x-scheme-handler/https",
            "x-scheme-handler/http",
            "text/html",
        ],
    },
    Kind {
        label: "Email",
        mimes: &["x-scheme-handler/mailto"],
    },
    Kind {
        label: "Files",
        mimes: &["inode/directory"],
    },
    Kind {
        label: "Text editor",
        mimes: &["text/plain"],
    },
    Kind {
        label: "Images",
        mimes: &["image/png", "image/jpeg", "image/webp", "image/gif"],
    },
    Kind {
        label: "Video",
        mimes: &["video/mp4", "video/x-matroska", "video/webm"],
    },
    Kind {
        label: "Music",
        mimes: &["audio/mpeg", "audio/flac", "audio/ogg"],
    },
    Kind {
        label: "PDF documents",
        mimes: &["application/pdf"],
    },
    Kind {
        label: "Archives",
        mimes: &[
            "application/zip",
            "application/x-tar",
            "application/x-7z-compressed",
        ],
    },
];

pub fn default_for(mime: &str) -> Option<String> {
    run("xdg-mime", &["query", "default", mime])
        .ok()
        .filter(|s| !s.is_empty())
}

pub fn set_default(id: &str, mimes: &[&str]) -> Result<(), String> {
    for m in mimes {
        run("xdg-mime", &["default", id, m])?;
    }
    Ok(())
}

/// Apps that say they open `mime` (or its group, like `image/*`).
pub fn candidates<'a>(apps: &'a [Entry], mime: &str) -> Vec<&'a Entry> {
    let group = mime.split('/').next().unwrap_or("");
    apps.iter()
        .filter(|e| {
            e.mime
                .iter()
                .any(|m| m == mime || m.strip_suffix("/*") == Some(group))
        })
        .collect()
}

// ---- Autostart ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Startup {
    pub entry: Entry,
    pub enabled: bool,
    /// Lives in your own folder (so it can be removed outright).
    pub yours: bool,
}

fn user_autostart() -> PathBuf {
    config_home().join("autostart")
}

/// Startup entries: yours override the system's of the same name.
pub fn autostart() -> Vec<Startup> {
    let mut by_id: BTreeMap<String, Startup> = BTreeMap::new();
    let system_dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
    let dirs = std::iter::once((user_autostart(), true)).chain(
        system_dirs
            .split(':')
            .filter(|s| !s.is_empty())
            .map(|d| (PathBuf::from(d).join("autostart"), false)),
    );
    for (dir, yours) in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for f in rd.flatten() {
            let id = f.file_name().to_string_lossy().into_owned();
            if !id.ends_with(".desktop") || by_id.contains_key(&id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(f.path()) else {
                continue;
            };
            // Entries meant only for other desktops don't start here.
            if !shown_here(&text) {
                continue;
            }
            if let Some(entry) = parse_entry(&id, &f.path(), &text) {
                by_id.insert(
                    id,
                    Startup {
                        enabled: !entry.hidden,
                        entry,
                        yours,
                    },
                );
            }
        }
    }
    let mut v: Vec<Startup> = by_id.into_values().collect();
    v.sort_by(|a, b| a.entry.cmp(&b.entry));
    v
}

fn shown_here(text: &str) -> bool {
    let desk = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "Hyprland".into());
    let ours: Vec<&str> = desk.split(':').chain(["Nordlys"]).collect();
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .map(|v| list(v.trim_start_matches('=')))
    };
    if let Some(only) = field("OnlyShowIn=") {
        return only.iter().any(|d| ours.contains(&d.as_str()));
    }
    if let Some(not) = field("NotShowIn=") {
        return !not.iter().any(|d| ours.contains(&d.as_str()));
    }
    true
}

/// Switch an entry on or off with a copy in your folder, as the spec
/// says, so the system's file stays untouched.
pub fn set_autostart(s: &Startup, on: bool) -> Result<(), String> {
    let text = std::fs::read_to_string(&s.entry.path).map_err(|e| e.to_string())?;
    let mut out = String::new();
    let mut in_main = false;
    let mut wrote = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_main && !wrote {
                out.push_str(&format!("Hidden={on_s}\n", on_s = !on));
                wrote = true;
            }
            in_main = t == "[Desktop Entry]";
        } else if in_main
            && (t.starts_with("Hidden=") || t.starts_with("X-GNOME-Autostart-enabled="))
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !wrote {
        out.push_str(&format!("Hidden={}\n", !on));
    }
    super::write_atomic(&user_autostart().join(&s.entry.id), &out)
}

/// A new startup command of your own.
pub fn add_autostart(name: &str, command: &str) -> Result<(), String> {
    let name = name.trim();
    let command = command.trim();
    if name.is_empty() || command.is_empty() {
        return Err("Give it a name and a command".into());
    }
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let path = user_autostart().join(format!("nordlys-{slug}.desktop"));
    if path.exists() {
        return Err(format!("There's already a startup entry called {name}"));
    }
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={command}\nComment=Added in Nordlys Settings\nX-Nordlys-Added=true\n"
    );
    super::write_atomic(&path, &text)
}

pub fn remove_autostart(s: &Startup) -> Result<(), String> {
    if !s.yours {
        return set_autostart(s, false);
    }
    std::fs::remove_file(&s.entry.path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "[Desktop Entry]\nType=Application\nName=Firefox\nName[de]=Firefox-Browser\n\
        Exec=firefox %u\nMimeType=text/html;x-scheme-handler/https;\nCategories=Network;WebBrowser;\n\n\
        [Desktop Action new]\nName=New Window\n";

    #[test]
    fn entries() {
        let e = parse_entry("firefox.desktop", Path::new("/x"), FIREFOX).unwrap();
        assert_eq!(e.name, "Firefox");
        assert_eq!(e.mime, ["text/html", "x-scheme-handler/https"]);
        assert!(
            parse_entry(
                "l.desktop",
                Path::new("/x"),
                "[Desktop Entry]\nType=Link\nName=L"
            )
            .is_none()
        );
        let apps = vec![e];
        assert_eq!(candidates(&apps, "x-scheme-handler/https").len(), 1);
        assert_eq!(candidates(&apps, "image/png").len(), 0);
    }

    #[test]
    fn desktops_filter_autostart() {
        // SAFETY: tests in this module don't read the variable concurrently.
        unsafe { std::env::set_var("XDG_CURRENT_DESKTOP", "Hyprland") };
        assert!(shown_here("[Desktop Entry]\nName=x"));
        assert!(!shown_here("[Desktop Entry]\nOnlyShowIn=GNOME;"));
        assert!(shown_here("[Desktop Entry]\nOnlyShowIn=Hyprland;KDE;"));
        assert!(!shown_here("[Desktop Entry]\nNotShowIn=Hyprland;"));
    }
}
