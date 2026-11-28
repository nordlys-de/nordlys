use super::*;

/// Run the calling test again in a child whose environment holds only
/// `vars`, and run `body` there. Environment lookups can then be tested
/// without touching this process's environment.
fn in_env(vars: &[(&str, &str)], body: impl FnOnce()) {
    if std::env::var_os("NORDLYS_TEST_CHILD").is_some() {
        return body();
    }
    let name = std::thread::current().name().unwrap().to_owned();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args([&name, "--exact", "--test-threads=1"])
        .env_clear()
        .env("NORDLYS_TEST_CHILD", "1")
        .envs(vars.iter().copied());
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        child.env("LLVM_PROFILE_FILE", profile);
    }
    let out = child.output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "child failed:\n{text}");
    assert!(text.contains("1 passed"), "child ran no test:\n{text}");
}

fn group(name: &str, key: Option<char>) -> Group {
    Group {
        name: name.into(),
        key,
        apps: vec!["foot".into()],
        open_in: OpenIn::default(),
        dir: None,
        reuse_running: true,
    }
}

#[test]
fn config_dir_prefers_xdg_config_home() {
    in_env(&[("XDG_CONFIG_HOME", "/cfg"), ("HOME", "/home/u")], || {
        assert_eq!(config_dir(), PathBuf::from("/cfg/nordlys"));
        assert_eq!(default_path(), PathBuf::from("/cfg/nordlys/nordlys.toml"));
    });
}

#[test]
fn config_dir_falls_back_to_the_home_folder() {
    in_env(&[("HOME", "/home/u")], || {
        assert_eq!(config_dir(), PathBuf::from("/home/u/.config/nordlys"));
    });
}

#[test]
fn config_dir_without_any_variable_is_relative() {
    in_env(&[], || {
        assert_eq!(config_dir(), PathBuf::from(".config/nordlys"));
    });
}

#[test]
fn a_tilde_folder_needs_a_home() {
    in_env(&[], || {
        let mut g = group("x", None);
        for dir in ["~", "~/code"] {
            g.dir = Some(dir.into());
            assert_eq!(g.dir_path(), None, "{dir}");
        }
        g.dir = Some("relative/dir".into());
        assert_eq!(g.dir_path(), Some(PathBuf::from("relative/dir")));
    });
}

#[test]
fn a_group_without_a_folder_has_no_path() {
    assert_eq!(group("x", None).dir_path(), None);
}

#[test]
fn layout_boundaries() {
    let auto = |w| Layout::Auto.for_width(w);
    assert_eq!(auto(2401), Layout::Ultrawide);
    assert_eq!(auto(2400), Layout::Rails);
    assert_eq!(auto(1600), Layout::Rails);
    assert_eq!(auto(1599), Layout::Peek);
    assert_eq!(auto(0), Layout::Peek);
    for fixed in [Layout::Ultrawide, Layout::Rails, Layout::Peek] {
        for width in [800, 1920, 5120] {
            assert_eq!(fixed.for_width(width), fixed);
        }
    }
}

#[test]
fn groups_are_found_by_name_or_single_letter_key() {
    let c = Config {
        groups: vec![
            group("Mail", None),
            group("Nordlys dev", Some('D')),
            group("d", Some('x')),
        ],
        ..Config::default()
    };
    let name = |q: &str| c.group(q).map(|g| g.name.as_str());
    assert_eq!(name("MAIL"), Some("Mail"));
    assert_eq!(name("nordlys DEV"), Some("Nordlys dev"));
    // The first match wins: the key D before the group named "d".
    assert_eq!(name("d"), Some("Nordlys dev"));
    assert_eq!(name("X"), Some("d"));
    assert_eq!(name("m"), None);
    assert_eq!(name("dd"), None);
    assert_eq!(name(""), None);
}

#[test]
fn light_mode_uses_the_light_palette() {
    let c = Config::from_toml("[theme]\nmode = \"light\"").unwrap();
    assert_eq!(c.theme.mode, Mode::Light);
    assert_eq!(c.palette(), Palette::light());
    assert_eq!(Config::default().palette(), Palette::dark());
}

#[test]
fn an_unreadable_accent_set_in_code_is_ignored() {
    let mut c = Config::default();
    c.theme.accent = Some("not a colour".into());
    assert_eq!(c.palette(), Palette::dark());
    assert!(matches!(c.validate(), Err(Error::Invalid(_))));
}

#[test]
fn split_must_stay_in_range() {
    let split = |v: &str| Config::from_toml(&format!("[stage]\nsplit = {v}"));
    for good in ["0.3", "0.55", "0.8"] {
        assert!(split(good).is_ok(), "{good}");
    }
    for bad in ["0.29", "0.81", "0.0", "1.0", "-0.5", "nan"] {
        let e = split(bad).unwrap_err();
        assert_eq!(
            e.to_string(),
            "nordlys.toml: stage.split must be between 0.3 and 0.8",
            "{bad}"
        );
    }
}

#[test]
fn a_group_needs_apps() {
    let e = Config::from_toml("[[group]]\nname = \"Empty\"\napps = []").unwrap_err();
    assert_eq!(e.to_string(), "nordlys.toml: group \"Empty\" has no apps");
}

#[test]
fn group_options_have_defaults() {
    let c = Config::from_toml("[[group]]\nname = \"A\"\napps = [\"foot\"]").unwrap();
    let g = &c.groups[0];
    assert_eq!(g.open_in, OpenIn::NewWorkspace);
    assert!(g.reuse_running);
    assert_eq!((g.key, g.dir.as_deref()), (None, None));
}

#[test]
fn unknown_keys_and_bad_values_are_parse_errors() {
    for bad in [
        "colour = 1",
        "[stage]\nfps = 3",
        "[stage]\nlayout = \"huge\"",
        "[theme]\nmode = \"blue\"",
        "[theme]\nblur = 900",
        "[[group]]\napps = [\"foot\"]",
        "[[group]]\nname = \"a\"\napps = [\"foot\"]\ncolour = 1",
        "[theme",
    ] {
        let e = Config::from_toml(bad).unwrap_err();
        assert!(matches!(e, Error::Parse(_)), "{bad}: {e:?}");
        assert!(e.to_string().starts_with("nordlys.toml: "), "{e}");
    }
}

#[test]
fn a_config_survives_a_round_trip() {
    let c = Config::from_toml(
        "[theme]\nmode = \"light\"\naccent = \"#112233\"\nblur = 0\n\
         reduce_transparency = true\nreduce_motion = true\n\
         [stage]\nlayout = \"peek\"\nsplit = 0.5\nshelf_fps = 3\n\
         [[group]]\nname = \"A\"\nkey = \"a\"\napps = [\"foot\"]\n\
         open_in = \"current\"\ndir = \"/srv\"\nreuse_running = false\n",
    )
    .unwrap();
    let written = toml::to_string(&c).unwrap();
    assert_eq!(Config::from_toml(&written).unwrap(), c);
}

#[test]
fn load_reads_a_file_and_defaults_when_it_is_missing() {
    let dir = std::env::temp_dir().join(format!("nordlys-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("nordlys.toml");
    assert_eq!(Config::load(&file).unwrap(), Config::default());

    std::fs::write(&file, "[theme]\nblur = 7").unwrap();
    assert_eq!(Config::load(&file).unwrap().theme.blur, 7);

    // A folder where the file should be: readable error, not defaults.
    let e = Config::load(&dir).unwrap_err();
    assert!(matches!(&e, Error::Io(path, _) if path == &dir), "{e:?}");
    assert!(
        e.to_string()
            .starts_with(&format!("cannot read {}: ", dir.display())),
        "{e}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn colours_parse_print_and_weigh() {
    assert_eq!(Rgb::hex_u32(0x01_02_03), Rgb(1, 2, 3));
    assert_eq!(Rgb(0, 10, 255).hex(), "#000AFF");
    assert_eq!(Rgb(0, 10, 255).bare(), "000AFF");
    assert_eq!(Rgb(0, 0, 0).luma(), 0.0);
    assert!((Rgb(255, 255, 255).luma() - 1.0).abs() < 1e-6);
    // Green reads brighter than red, red brighter than blue.
    assert!(Rgb(0, 255, 0).luma() > Rgb(255, 0, 0).luma());
    assert!(Rgb(255, 0, 0).luma() > Rgb(0, 0, 255).luma());
    for bad in [
        "", "#", "##112233", "1122334", "11223", "+12233", "11223é", "ééé",
    ] {
        assert_eq!(Rgb::parse(bad), None, "{bad:?}");
    }
}

/// Body text has to stay readable on every background it is drawn on.
#[test]
fn both_palettes_keep_text_legible() {
    fn relative_luminance(c: Rgb) -> f32 {
        let channel = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
    }
    fn contrast(a: Rgb, b: Rgb) -> f32 {
        let (a, b) = (relative_luminance(a), relative_luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    for p in [Palette::dark(), Palette::light()] {
        for back in [p.ground, p.surface, p.raised] {
            assert!(contrast(p.text, back) >= 7.0, "text on {}", back.hex());
            assert!(contrast(p.muted, back) >= 4.5, "muted on {}", back.hex());
        }
        assert!(contrast(p.on_accent, p.accent) >= 4.5, "on accent");
        assert_eq!(p.dark, p.ground.luma() < p.text.luma());
    }
}
