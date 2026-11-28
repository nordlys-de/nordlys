//! nordlys-theme: one palette, written out for every toolkit and app.
//!
//! Usage: nordlys-theme [--config PATH] [--out DIR] [--dry-run] [--quiet] [--watch]
//!
//! `--out` defaults to `$XDG_CONFIG_HOME` (usually ~/.config), so the files
//! land where GTK, Qt, foot, mpv, imv, zathura, Helix and btop look for
//! them. Files you have made your own are left alone; see `targets.rs`.
//! Writing to the default location also sets the GNOME interface settings
//! that libadwaita apps follow (dark or light, and fonts you haven't set).
//! The Nordlys session runs it at every login with `--watch`, which then
//! writes everything again whenever nordlys.toml's colours change. GTK 3
//! and GTK 4 apps that are open follow; others on their next start.

use nordlys_config::{Config, Mode, Palette};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

mod targets;
use targets::{Ctx, Kind, MARK};

struct Args {
    config: PathBuf,
    out: PathBuf,
    default_out: bool,
    dry_run: bool,
    quiet: bool,
    watch: bool,
}

fn default_out_dir() -> PathBuf {
    nordlys_config::config_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn parse_args() -> Result<Args, String> {
    let mut config = nordlys_config::default_path();
    let mut out = None;
    let mut dry_run = false;
    let mut quiet = false;
    let mut watch = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" => config = it.next().ok_or("--config needs a path")?.into(),
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a directory")?)),
            "--dry-run" => dry_run = true,
            "-q" | "--quiet" => quiet = true,
            "--watch" => watch = true,
            "-h" | "--help" => {
                println!(
                    "usage: nordlys-theme [--config PATH] [--out DIR] [--dry-run] [--quiet] [--watch]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let default_out = out.is_none();
    Ok(Args {
        config,
        out: out.unwrap_or_else(default_out_dir),
        default_out,
        dry_run,
        quiet,
        watch: watch && !dry_run,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("nordlys-theme: {e}");
            return ExitCode::from(2);
        }
    };
    let cfg = match Config::load(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nordlys-theme: {e}");
            return ExitCode::FAILURE;
        }
    };
    let ok = apply(&args, &cfg);
    if args.watch {
        watch(&args, cfg);
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Write every target and the desktop settings. False if a file failed.
fn apply(args: &Args, cfg: &Config) -> bool {
    let p: Palette = cfg.palette();
    // The other mode with the same accent, for apps that switch live.
    let mut flipped = cfg.clone();
    flipped.theme.mode = if p.dark { Mode::Light } else { Mode::Dark };
    let other = flipped.palette();
    let themes = if args.default_out {
        data_home().join("themes")
    } else {
        args.out.join("themes")
    };
    let adw_gtk3 = adw_gtk3_dir();
    let ctx = Ctx {
        out: &args.out,
        other: &other,
        themes: &themes,
        adw_gtk3: adw_gtk3.as_deref(),
    };

    let mut failed = false;
    for t in targets::all(&p, &ctx) {
        let path = args.out.join(&t.path);
        let exists = path.exists();
        let write_it = match t.kind {
            Kind::Owned => true,
            Kind::Marked => !exists || is_ours(&path),
            Kind::Seed => !exists,
        };
        if !write_it {
            if let Some(hook) = t.hook
                && !std::fs::read_to_string(&path).is_ok_and(|s| s.contains(hook))
                && !args.quiet
            {
                println!(
                    "kept {} (add `{hook}` to it for the Nordlys colours)",
                    path.display()
                );
            } else if t.kind == Kind::Marked && !args.quiet {
                println!("kept {} (yours: no \"{MARK}\" line)", path.display());
            }
            continue;
        }
        if args.dry_run {
            println!("── {} ──\n{}", path.display(), t.body);
            continue;
        }
        // Unchanged files aren't touched, so nothing reloads for nothing.
        if Kind::Seed != t.kind && std::fs::read_to_string(&path).is_ok_and(|s| s == t.body) {
            continue;
        }
        match write(&path, &t.body) {
            Ok(()) if !args.quiet => println!("wrote {}", path.display()),
            Ok(()) => {}
            Err(e) => {
                eprintln!("nordlys-theme: cannot write {}: {e}", path.display());
                failed = true;
            }
        }
    }

    if args.default_out {
        desktop_settings(&p, adw_gtk3.is_some(), args.dry_run, args.quiet);
    }
    !failed
}

/// Check nordlys.toml every second and apply it again when the palette
/// changes. Ends with the session: when its Wayland socket goes away.
fn watch(args: &Args, cfg: Config) {
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let socket = wayland_socket();
    let mut seen = modified(&args.config);
    let mut palette = cfg.palette();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if socket.as_ref().is_some_and(|s| !s.exists()) {
            return;
        }
        let now = modified(&args.config);
        if now == seen {
            continue;
        }
        seen = now;
        // A file half-saved by an editor fails to parse; the next save
        // brings it back.
        let Ok(cfg) = Config::load(&args.config) else {
            continue;
        };
        let next = cfg.palette();
        if next != palette {
            apply(args, &cfg);
            palette = next;
        }
    }
}

fn wayland_socket() -> Option<PathBuf> {
    let display = std::env::var_os("WAYLAND_DISPLAY")?;
    let display = PathBuf::from(display);
    let path = if display.is_absolute() {
        display
    } else {
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join(display)
    };
    path.exists().then_some(path)
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
}

/// Did nordlys-theme write this file (and the user leave the mark in)?
fn is_ours(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.lines().take(3).any(|l| l.contains(MARK)))
        .unwrap_or(false)
}

fn write(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write beside and rename, so an app reading it never sees half a file.
    let tmp = path.with_extension("nordlys-tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

/// The themes folder that holds adw-gtk3, which makes GTK 3 apps look
/// like libadwaita ones and takes the palette's named colours.
fn adw_gtk3_dir() -> Option<PathBuf> {
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = data_dirs
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join("themes"))
        .collect();
    if let Some(h) = home {
        dirs.push(h.join(".themes"));
        dirs.push(h.join(".local/share/themes"));
    }
    dirs.into_iter()
        .find(|d| d.join("adw-gtk3/gtk-3.0/gtk.css").is_file())
}

/// libadwaita apps take dark or light from org.gnome.desktop.interface
/// (through the settings portal), not from gtk.css. The mode always wins;
/// fonts are only filled in when you haven't picked your own.
fn desktop_settings(p: &Palette, adw_gtk3: bool, dry_run: bool, quiet: bool) {
    const SCHEMA: &str = "org.gnome.desktop.interface";
    if !have("gsettings") {
        if !quiet {
            println!("gsettings not found; libadwaita apps keep their own dark/light setting");
        }
        return;
    }
    let mut set: Vec<(&str, &str)> = vec![(
        "color-scheme",
        if p.dark {
            "prefer-dark"
        } else {
            "prefer-light"
        },
    )];
    // GTK 3 reloads its theme when the name changes. Same mode, new accent:
    // the name stays, so pass through the base theme first.
    let theme = targets::gtk3_name(p.dark);
    if !dry_run && current(SCHEMA, "gtk-theme").as_deref() == Some(theme) {
        set.push(("gtk-theme", targets::gtk3_base(p.dark, adw_gtk3)));
    }
    set.push(("gtk-theme", theme));
    // dconf prints nothing for keys still at their default.
    if have("dconf") {
        for (key, value) in [
            ("font-name", "Inter 11"),
            ("document-font-name", "Inter 11"),
            ("monospace-font-name", "JetBrains Mono 10"),
            ("cursor-theme", "Adwaita"),
        ] {
            let path = format!("/org/gnome/desktop/interface/{key}");
            let current = Command::new("dconf").args(["read", &path]).output();
            if current.is_ok_and(|o| o.status.success() && o.stdout.trim_ascii().is_empty()) {
                set.push((key, value));
            }
        }
    }
    for (key, value) in set {
        if dry_run {
            println!("would run: gsettings set {SCHEMA} {key} '{value}'");
            continue;
        }
        let ok = gsettings(SCHEMA, key, value)
            // GNOME before 47 has no prefer-light; "default" is light there.
            || (value == "prefer-light" && gsettings(SCHEMA, key, "default"));
        if !ok && !quiet {
            eprintln!("nordlys-theme: couldn't set {SCHEMA} {key}");
        }
    }
}

/// A string setting's value, without GVariant's quotes.
fn current(schema: &str, key: &str) -> Option<String> {
    let out = Command::new("gsettings")
        .args(["get", schema, key])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let v = String::from_utf8(out.stdout).ok()?;
    Some(v.trim().trim_matches('\'').to_owned())
}

fn gsettings(schema: &str, key: &str, value: &str) -> bool {
    Command::new("gsettings")
        .args(["set", schema, key, value])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn have(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(cmd).is_file()))
}
