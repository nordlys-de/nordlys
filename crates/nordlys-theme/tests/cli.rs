//! The real `nordlys-theme` process. By default it writes into the user's
//! config folder and changes GNOME settings, so every run here gets a cleared
//! environment, an empty home and a PATH holding only fake programs.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const FILES: [&str; 6] = [
    "gtk-4.0/gtk.css",
    "gtk-3.0/settings.ini",
    "qt6ct/colors/nordlys.conf",
    "foot/nordlys.ini",
    "foot/foot.ini",
    "imv/config",
];

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ntheme-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        Home(dir)
    }

    fn command(&self, args: &[&str], vars: &[(&str, PathBuf)]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_nordlys-theme"));
        c.args(args)
            .current_dir(&self.0)
            .env_clear()
            .env("HOME", &self.0)
            .env("PATH", self.0.join("bin"))
            .env("XDG_DATA_DIRS", self.0.join("data"))
            .envs(vars.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            c.env("LLVM_PROFILE_FILE", profile);
        }
        c
    }

    fn theme(&self, args: &[&str]) -> Output {
        self.command(args, &[]).output().unwrap()
    }

    /// Run, expect success, and return what it printed.
    fn ok(&self, args: &[&str]) -> String {
        let out = self.theme(args);
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    fn config_dir(&self) -> PathBuf {
        self.0.join(".config")
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.config_dir().join(rel)).unwrap()
    }

    fn put(&self, rel: &str, body: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    /// A program on the fake PATH that logs its arguments, then runs `body`.
    fn fake(&self, name: &str, body: &str) {
        let log = self.0.join("calls");
        let path = self.put(
            &format!("bin/{name}"),
            &format!(
                "#!/bin/sh\necho \"{name} $*\" >> '{}'\n{body}\n",
                log.display()
            ),
        );
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.0.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

#[test]
fn files_land_in_the_xdg_config_folder() {
    let home = Home::new();
    let xdg = home.0.join("xdg");
    let out = home
        .command(&[], &[("XDG_CONFIG_HOME", xdg.clone())])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let said = String::from_utf8(out.stdout).unwrap();
    for f in FILES {
        assert!(xdg.join(f).is_file(), "{f}");
        assert!(said.contains(&format!("wrote {}", xdg.join(f).display())));
    }
    assert!(!home.config_dir().exists());
}

#[test]
fn without_xdg_they_land_under_home_and_leave_no_temporaries() {
    let home = Home::new();
    let said = home.ok(&[]);
    for f in FILES {
        assert!(home.config_dir().join(f).is_file(), "{f}");
    }
    let leftovers: Vec<_> = files_under(&home.config_dir())
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with("nordlys-tmp"))
        .collect();
    assert_eq!(leftovers, Vec::<PathBuf>::new());
    assert!(
        said.contains("gsettings not found; libadwaita apps keep their own dark/light setting"),
        "{said}"
    );
}

#[test]
fn a_second_run_rewrites_ours_and_keeps_yours() {
    let home = Home::new();
    home.ok(&[]);
    // Still marked: rewritten. Mark deleted: yours. Owned: always rewritten.
    let marked = home.read("gtk-3.0/gtk.css");
    home.put(".config/gtk-3.0/gtk.css", &format!("{marked}/* edit */\n"));
    home.put(".config/gtk-4.0/gtk.css", "window { color: red; }\n");
    home.put(".config/foot/nordlys.ini", "old\n");
    home.put(".config/foot/foot.ini", "font=mine\n");
    home.put(".config/mpv/input.conf", "q quit\n");

    let said = home.ok(&[]);
    assert_eq!(home.read("gtk-3.0/gtk.css"), marked);
    assert_eq!(home.read("gtk-4.0/gtk.css"), "window { color: red; }\n");
    assert!(home.read("foot/nordlys.ini").contains("[colors]"));
    assert_eq!(home.read("foot/foot.ini"), "font=mine\n");
    assert_eq!(home.read("mpv/input.conf"), "q quit\n");

    let gtk4 = home.config_dir().join("gtk-4.0/gtk.css");
    assert!(said.contains(&format!(
        "kept {} (yours: no \"Generated by nordlys-theme\" line)",
        gtk4.display()
    )));
    let foot = home.config_dir().join("foot/foot.ini");
    assert!(said.contains(&format!(
        "kept {} (add `include=~/.config/foot/nordlys.ini` to it for the Nordlys colours)",
        foot.display()
    )));
    // Seeds that still have their hook, or need none, are kept silently.
    assert!(!said.contains("zathurarc"), "{said}");
    assert!(!said.contains("input.conf"), "{said}");
}

#[test]
fn the_mark_only_counts_near_the_top() {
    let home = Home::new();
    let late = "a\nb\nc\n# Generated by nordlys-theme\n";
    home.put(".config/imv/config", late);
    home.ok(&[]);
    assert_eq!(home.read("imv/config"), late);
}

#[test]
fn quiet_says_nothing() {
    let home = Home::new();
    home.put(".config/gtk-4.0/gtk.css", "mine\n");
    home.put(".config/foot/foot.ini", "mine\n");
    for flag in ["-q", "--quiet"] {
        assert_eq!(home.ok(&[flag]), "");
    }
    assert!(home.config_dir().join("imv/config").is_file());
}

#[test]
fn config_and_out_can_be_chosen_and_leave_the_desktop_alone() {
    let home = Home::new();
    home.fake("gsettings", "");
    let config = home.put(
        "mine.toml",
        "[theme]\nmode = \"light\"\naccent = \"#123456\"",
    );
    let out_dir = home.0.join("out");
    home.ok(&[
        "--config",
        config.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    let css = std::fs::read_to_string(out_dir.join("gtk-4.0/gtk.css")).unwrap();
    assert!(css.contains("#123456"), "{css}");
    let qt = std::fs::read_to_string(out_dir.join("qt6ct/qt6ct.conf")).unwrap();
    assert!(qt.contains(&format!(
        "color_scheme_path={}/qt6ct/colors/nordlys.conf",
        out_dir.display()
    )));
    assert!(!home.config_dir().exists());
    assert_eq!(
        home.calls(),
        Vec::<String>::new(),
        "--out never touches gsettings"
    );
}

#[test]
fn a_dry_run_prints_everything_and_changes_nothing() {
    let home = Home::new();
    home.fake("gsettings", "");
    let said = home.ok(&["--dry-run"]);
    for f in FILES {
        let path = home.config_dir().join(f);
        assert!(said.contains(&format!("── {} ──", path.display())), "{f}");
    }
    for line in [
        "would run: gsettings set org.gnome.desktop.interface color-scheme 'prefer-dark'",
        "would run: gsettings set org.gnome.desktop.interface gtk-theme 'Nordlys-dark'",
    ] {
        assert!(said.contains(line), "{said}");
    }
    assert!(!home.config_dir().exists());
    assert_eq!(home.calls(), Vec::<String>::new());
}

#[test]
fn the_desktop_mode_is_set_and_only_unset_fonts_are_filled_in() {
    let home = Home::new();
    home.fake("gsettings", "");
    // The monospace font was chosen already; the rest are at their default.
    home.fake(
        "dconf",
        "case $2 in */monospace-font-name) echo \"'Mine 12'\" ;; esac",
    );
    home.ok(&["-q"]);
    let sets: Vec<String> = home
        .calls()
        .into_iter()
        .filter_map(|c| {
            c.strip_prefix("gsettings set org.gnome.desktop.interface ")
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(
        sets,
        [
            "color-scheme prefer-dark",
            "gtk-theme Nordlys-dark",
            "font-name Inter 11",
            "document-font-name Inter 11",
            "cursor-theme Adwaita",
        ]
    );
    assert!(
        home.calls()
            .contains(&"dconf read /org/gnome/desktop/interface/font-name".to_owned())
    );
}

#[test]
fn a_dconf_that_fails_changes_no_fonts() {
    let home = Home::new();
    home.fake("gsettings", "");
    home.fake("dconf", "exit 1");
    home.ok(&["-q"]);
    let sets = home
        .calls()
        .iter()
        .filter(|c| c.starts_with("gsettings set"))
        .count();
    assert_eq!(sets, 2);
}

#[test]
fn old_gnome_gets_default_for_light_mode() {
    let home = Home::new();
    home.fake("gsettings", "[ \"$4\" != prefer-light ]");
    let config = home.put("light.toml", "[theme]\nmode = \"light\"");
    let out = home.theme(&["--config", config.to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    let calls = home.calls();
    assert_eq!(
        calls[..4],
        [
            "gsettings get org.gnome.desktop.interface gtk-theme",
            "gsettings set org.gnome.desktop.interface color-scheme prefer-light",
            "gsettings set org.gnome.desktop.interface color-scheme default",
            "gsettings set org.gnome.desktop.interface gtk-theme Nordlys",
        ]
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

#[test]
fn a_setting_that_cannot_be_changed_is_reported_but_not_fatal() {
    let home = Home::new();
    home.fake("gsettings", "exit 1");
    let out = home.theme(&[]);
    assert!(out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("couldn't set org.gnome.desktop.interface color-scheme"),
        "{err}"
    );
    assert!(
        err.contains("couldn't set org.gnome.desktop.interface gtk-theme"),
        "{err}"
    );
    assert_eq!(String::from_utf8_lossy(&home.theme(&["-q"]).stderr), "");
}

#[test]
fn adw_gtk3_is_used_wherever_it_is_installed() {
    for place in ["data/themes", ".themes", ".local/share/themes"] {
        for (mode, theme, base) in [
            ("dark", "Nordlys-dark", "adw-gtk3-dark"),
            ("light", "Nordlys", "adw-gtk3"),
        ] {
            let home = Home::new();
            home.put(&format!("{place}/adw-gtk3/gtk-3.0/gtk.css"), "");
            let config = home.put("c.toml", &format!("[theme]\nmode = \"{mode}\""));
            home.ok(&["-q", "--config", config.to_str().unwrap()]);
            assert!(
                home.read("gtk-3.0/settings.ini")
                    .contains(&format!("gtk-theme-name={theme}\n")),
                "{place} {mode}"
            );
            let css = std::fs::read_to_string(
                home.0
                    .join(".local/share/themes")
                    .join(theme)
                    .join("gtk-3.0/gtk.css"),
            )
            .unwrap();
            let import = format!(
                "@import url(\"file://{}/{base}/gtk-3.0/gtk.css\");",
                home.0.join(place).display()
            );
            assert!(css.contains(&import), "{place} {mode}: {css}");
        }
    }
}

#[test]
fn both_gtk3_themes_go_to_the_data_home() {
    let home = Home::new();
    let data = home.0.join("mydata");
    let out = home
        .command(&["-q"], &[("XDG_DATA_HOME", data.clone())])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    for theme in ["Nordlys", "Nordlys-dark"] {
        let css = std::fs::read_to_string(data.join("themes").join(theme).join("gtk-3.0/gtk.css"))
            .unwrap();
        assert!(css.contains("Adwaita/gtk-contained"), "{css}");
        assert!(css.contains("@define-color theme_bg_color"), "{css}");
    }
}

#[test]
fn a_new_accent_in_the_same_mode_reloads_open_gtk3_apps() {
    let home = Home::new();
    // GTK 3 already uses Nordlys-dark, so its name has to change and back.
    home.fake(
        "gsettings",
        "[ \"$1\" = get ] && echo \"'Nordlys-dark'\"; true",
    );
    home.ok(&["-q"]);
    let themes: Vec<String> = home
        .calls()
        .into_iter()
        .filter_map(|c| {
            c.strip_prefix("gsettings set org.gnome.desktop.interface gtk-theme ")
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(themes, ["Adwaita-dark", "Nordlys-dark"]);
}

#[test]
fn watch_applies_a_changed_mode_and_ends_with_the_session() {
    let home = Home::new();
    home.fake("gsettings", "");
    let config = home.put("c.toml", "[theme]\nmode = \"dark\"");
    let socket = home.put("run/wayland-1", "");
    let child = home
        .command(
            &["-q", "--watch", "--config", config.to_str().unwrap()],
            &[("WAYLAND_DISPLAY", socket.clone())],
        )
        .spawn()
        .unwrap();
    let settings = home.config_dir().join("gtk-3.0/settings.ini");
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        for _ in 0..100 {
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("timed out waiting for {what}");
    };
    let has = |s: &str| std::fs::read_to_string(&settings).is_ok_and(|b| b.contains(s));
    wait_for("the first run", &|| has("gtk-theme-name=Nordlys-dark\n"));
    // Modification times can be coarse: make sure this one is newer.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&config, "[theme]\nmode = \"light\"").unwrap();
    wait_for("the light theme", &|| has("gtk-theme-name=Nordlys\n"));
    std::fs::remove_file(&socket).unwrap();
    let child = std::cell::RefCell::new(child);
    wait_for("the watcher to end", &|| {
        child.borrow_mut().try_wait().unwrap().is_some()
    });
}

#[test]
fn help_writes_nothing() {
    for flag in ["-h", "--help"] {
        let home = Home::new();
        assert!(home.ok(&[flag]).starts_with("usage: nordlys-theme"));
        assert!(!home.config_dir().exists());
    }
}

#[test]
fn bad_arguments_exit_2() {
    for (args, why) in [
        (&["--config"][..], "--config needs a path"),
        (&["--out"], "--out needs a directory"),
        (&["--fast"], "unknown argument \"--fast\""),
    ] {
        let out = Home::new().theme(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let err = String::from_utf8(out.stderr).unwrap();
        assert_eq!(err, format!("nordlys-theme: {why}\n"));
    }
}

#[test]
fn a_broken_config_writes_nothing() {
    let home = Home::new();
    let config = home.put("bad.toml", "[theme]\naccent = \"red\"");
    let out = home.theme(&["--config", config.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not #RRGGBB"));
    assert!(!home.config_dir().exists());
}

#[test]
fn an_unwritable_target_is_reported_and_the_rest_still_written() {
    let home = Home::new();
    // A file where a folder has to be.
    home.put(".config/foot", "");
    let out = home.theme(&[]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("cannot write"), "{err}");
    assert!(home.config_dir().join("imv/config").is_file());
}
