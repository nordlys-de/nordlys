use super::*;
use std::collections::HashSet;

const DARK_THEME: &str = "/data/themes/Nordlys-dark/gtk-3.0/gtk.css";
const LIGHT_THEME: &str = "/data/themes/Nordlys/gtk-3.0/gtk.css";

fn ctx_with(other: &'static Palette) -> Ctx<'static> {
    Ctx {
        out: Path::new("/out"),
        other,
        themes: Path::new("/data/themes"),
        adw_gtk3: None,
    }
}

fn ctx() -> Ctx<'static> {
    ctx_with(Box::leak(Box::new(Palette::light())))
}

fn body<'a>(targets: &'a [Target], path: &str) -> &'a str {
    &targets.iter().find(|t| t.path == path).unwrap().body
}

#[test]
fn every_app_gets_its_files() {
    let files = all(&Palette::dark(), &ctx());
    let of = |kind: Kind| -> Vec<&str> {
        files
            .iter()
            .filter(|t| t.kind == kind)
            .map(|t| t.path.as_str())
            .collect()
    };
    assert_eq!(
        of(Kind::Marked),
        [
            "gtk-4.0/gtk.css",
            "gtk-3.0/gtk.css",
            "gtk-3.0/settings.ini",
            "qt6ct/qt6ct.conf",
            "imv/config",
        ]
    );
    assert_eq!(
        of(Kind::Owned),
        [
            "qt6ct/colors/nordlys.conf",
            "foot/nordlys.ini",
            "helix/themes/nordlys.toml",
            "btop/themes/nordlys.theme",
            "zathura/nordlys",
            "mpv/nordlys.conf",
            DARK_THEME,
            LIGHT_THEME,
        ]
    );
    assert_eq!(of(Kind::Seed).len(), SEEDS.len());
    let unique: HashSet<&str> = files.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(unique.len(), files.len(), "no path is written twice");
    for t in &files {
        assert!(t.body.ends_with('\n'), "{}", t.path);
        let theme = t.path.starts_with("/data/themes/");
        assert!(theme || !Path::new(&t.path).is_absolute(), "{}", t.path);
        assert!(t.hook.is_none() || t.kind == Kind::Seed, "{}", t.path);
    }
}

#[test]
fn gtk_defines_each_named_colour_once() {
    let p = Palette::dark();
    for (css, count) in [
        (gtk4_css(&p, &p, &Palette::light()), adw_colors(&p).len()),
        (gtk3_css(&p, None), adw_colors(&p).len() + 14),
    ] {
        let names: Vec<&str> = css
            .lines()
            .filter_map(|l| l.strip_prefix("@define-color "))
            .map(|l| l.split(' ').next().unwrap())
            .collect();
        assert_eq!(names.len(), count);
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), count);
        assert!(css.contains(&format!(
            "@define-color window_bg_color {};",
            p.ground.hex()
        )));
        assert!(css.contains(&format!(
            "@define-color accent_fg_color {};",
            p.on_accent.hex()
        )));
    }
    let variables = gtk4_css(&p, &p, &Palette::light())
        .lines()
        .filter(|l| l.trim_start().starts_with("--"))
        .count();
    assert_eq!(variables, 2 * adw_colors(&p).len(), "a block per mode");
}

#[test]
fn gtk4_switches_mode_without_a_restart() {
    let (dark, light) = (Palette::dark(), Palette::light());
    let css = gtk4_css(&light, &dark, &light);
    let block = |scheme: &str| {
        let start = css
            .find(&format!("@media (prefers-color-scheme: {scheme})"))
            .unwrap();
        let end = css[start..].find("\n}\n").unwrap();
        css[start..start + end].to_owned()
    };
    let bg = |p: &Palette| format!("--window-bg-color: {};", p.ground.hex());
    assert!(block("dark").contains(&bg(&dark)));
    assert!(block("light").contains(&bg(&light)));
    // The named colours, for older libadwaita, are the current mode's.
    assert!(css.contains(&format!(
        "@define-color window_bg_color {};",
        light.ground.hex()
    )));
}

#[test]
fn gtk3_themes_build_on_adw_gtk3_or_adwaita() {
    let (dark, light) = (Palette::dark(), Palette::light());
    let adw = Path::new("/usr/share/themes");
    assert!(
        gtk3_css(&dark, Some(adw))
            .contains("@import url(\"file:///usr/share/themes/adw-gtk3-dark/gtk-3.0/gtk.css\");")
    );
    assert!(
        gtk3_css(&light, Some(adw))
            .contains("@import url(\"file:///usr/share/themes/adw-gtk3/gtk-3.0/gtk.css\");")
    );
    assert!(
        gtk3_css(&light, None).contains(
            "@import url(\"resource:///org/gtk/libgtk/theme/Adwaita/gtk-contained.css\");"
        )
    );
    // Each mode's theme has its own colours; the user file has none.
    let files = all(&dark, &ctx());
    assert!(body(&files, DARK_THEME).contains(&dark.ground.hex()));
    assert!(body(&files, LIGHT_THEME).contains(&light.ground.hex()));
    assert!(!body(&files, "gtk-3.0/gtk.css").contains("@define-color"));
    assert!(body(&files, "gtk-3.0/settings.ini").contains("\ngtk-theme-name=Nordlys-dark\n"));
    assert_eq!(gtk3_base(true, false), "Adwaita-dark");
}

#[test]
fn gtk3_settings_follow_the_mode_and_theme() {
    let dark = gtk3_settings(&Palette::dark(), "adw-gtk3-dark");
    assert!(dark.contains("\ngtk-theme-name=adw-gtk3-dark\n"));
    assert!(dark.contains("\ngtk-application-prefer-dark-theme=1\n"));
    let light = gtk3_settings(&Palette::light(), "Adwaita");
    assert!(light.contains("\ngtk-theme-name=Adwaita\n"));
    assert!(light.contains("\ngtk-application-prefer-dark-theme=0\n"));
}

#[test]
fn qt_dims_only_the_disabled_row() {
    let s = qt_scheme(&Palette::dark());
    let rows: Vec<(&str, &str)> = s
        .lines()
        .skip(1)
        .map(|l| l.split_once('=').unwrap())
        .collect();
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        ["active_colors", "disabled_colors", "inactive_colors"]
    );
    assert_eq!(rows[0].1, rows[2].1);
    assert_ne!(rows[0].1, rows[1].1);
    assert!(
        rows[0].1.starts_with("#ffe6eaf0, #ff1b2029"),
        "{}",
        rows[0].1
    );
}

#[test]
fn foot_writes_bare_colours() {
    let f = foot(&Palette::dark());
    assert!(f.contains("\n[colors]\nbackground=0A0D13\nforeground=E6EAF0\n"));
    assert!(f.contains("\nalpha=0.86\n"));
    assert!(!f.lines().skip(1).any(|l| l.contains('#')));
    let keys = f
        .lines()
        .filter(|l| l.starts_with("regular") || l.starts_with("bright"))
        .count();
    assert_eq!(keys, 16);
}

#[test]
fn templates_take_all_three_spellings_of_a_colour() {
    let p = Palette::dark();
    let (r, g, b) = (p.accent.0, p.accent.1, p.accent.2);
    assert_eq!(
        fill("%accent% %accent.bare% %accent_a% %dark%", &p),
        format!(
            "{} {} rgba({r},{g},{b},0.45) true",
            p.accent.hex(),
            p.accent.bare()
        )
    );
    assert_eq!(
        fill("%dark% 100% %nope%", &Palette::light()),
        "false 100% %nope%"
    );
}

#[test]
fn the_selection_colour_sits_between_accent_and_ground() {
    for p in [Palette::dark(), Palette::light()] {
        let s = selection(&p);
        for (mid, a, b) in [
            (s.0, p.accent.0, p.ground.0),
            (s.1, p.accent.1, p.ground.1),
            (s.2, p.accent.2, p.ground.2),
        ] {
            assert!((a.min(b)..=a.max(b)).contains(&mid), "{mid} {a} {b}");
        }
        assert_ne!(s, p.accent);
        assert_ne!(s, p.ground);
    }
}

#[test]
fn mixing_runs_from_one_colour_to_the_other() {
    let (a, b) = (Rgb(0, 100, 200), Rgb(200, 100, 0));
    assert_eq!(mix(a, b, 0.0), a);
    assert_eq!(mix(a, b, 1.0), b);
    assert_eq!(mix(a, b, 0.5), Rgb(100, 100, 100));
}

#[test]
fn a_custom_accent_reaches_every_colour_file() {
    let mut p = Palette::light();
    p.set_accent(Rgb(0x12, 0x34, 0x56));
    let mut other = Palette::dark();
    other.set_accent(Rgb(0x12, 0x34, 0x56));
    let files = all(&p, &ctx_with(Box::leak(Box::new(other))));
    for path in [
        "gtk-4.0/gtk.css",
        DARK_THEME,
        LIGHT_THEME,
        "helix/themes/nordlys.toml",
        "btop/themes/nordlys.theme",
        "zathura/nordlys",
    ] {
        assert!(body(&files, path).contains("#123456"), "{path}");
    }
    assert!(body(&files, "qt6ct/colors/nordlys.conf").contains("#ff123456"));
    assert!(body(&files, "foot/nordlys.ini").contains("urls=123456"));
}
