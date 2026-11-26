//! Where settings are kept: nordlys.toml for the shell, settings.lua for
//! Hyprland and hypridle.conf for idle, all in ~/.config/nordlys.

pub mod hypr;
pub mod idle;
pub mod nordlys;

pub use hypr::{Hypr, V};
pub use nordlys::NordlysToml;

/// Where the wallpaper and its fit live: the session's wallpaper service
/// reads both.
pub fn wallpaper_link() -> std::path::PathBuf {
    nordlys_config::config_dir().join("wallpaper")
}

pub fn wallpaper_mode_file() -> std::path::PathBuf {
    nordlys_config::config_dir().join("wallpaper-mode")
}

pub const WALLPAPER_MODES: [&str; 4] = ["fill", "fit", "center", "tile"];

pub fn wallpaper() -> (Option<std::path::PathBuf>, String) {
    let link = wallpaper_link();
    let path = std::fs::read_link(&link)
        .ok()
        .or_else(|| link.exists().then_some(link.clone()))
        .or_else(|| crate::sys::data_dir().map(|d| d.join("wallpaper.jpg")));
    let mode = std::fs::read_to_string(wallpaper_mode_file())
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|m| WALLPAPER_MODES.contains(&m.as_str()))
        .unwrap_or_else(|| "fill".into());
    (path, mode)
}

/// Point the wallpaper at `image` with `mode` and redraw it.
pub fn set_wallpaper(image: Option<&std::path::Path>, mode: &str) -> Result<(), String> {
    use crate::sys;
    let link = wallpaper_link();
    if let Some(img) = image {
        if !img.is_file() {
            return Err(format!("{} isn't a file", img.display()));
        }
        if let Some(d) = link.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(img, &link).map_err(|e| e.to_string())?;
    }
    sys::write_atomic(&wallpaper_mode_file(), mode)?;
    let target = std::fs::read_link(&link)
        .ok()
        .or_else(|| sys::data_dir().map(|d| d.join("wallpaper.jpg")))
        .ok_or("no wallpaper to show")?;
    if sys::run(
        "systemctl",
        &["--user", "is-active", "nordlys-wallpaper.service"],
    )
    .as_deref()
        == Ok("active")
    {
        sys::run(
            "systemctl",
            &["--user", "restart", "nordlys-wallpaper.service"],
        )
        .map(|_| ())
    } else {
        let _ = sys::run("pkill", &["-x", "swaybg"]);
        sys::spawn("swaybg", &["-m", mode, "-i", &target.to_string_lossy()])
    }
}
