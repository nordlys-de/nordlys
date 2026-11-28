//! nordlys-shell: the Nordlys top bar (with now playing and notifications
//! on its right), launcher, shelf, notifications and lock screen.
//!
//! Usage:
//!   nordlys-shell [bar] [--window] [--output NAME]
//!   nordlys-shell launcher [--window] [--query TEXT]
//!   nordlys-shell shelf [--window] [--output NAME]
//!   nordlys-shell notify [--window]
//!   nordlys-shell lock [--window | --check]
//!   nordlys-shell osd volume|brightness up|down|show
//!   nordlys-shell tray
//!   nordlys-shell welcome [--first-run] [--window]
//!
//! On Wayland each piece is a layer-shell surface, so it runs on Hyprland
//! today and on nordlys-comp later. Elsewhere, or with `--window`, it opens
//! as an ordinary preview window.

mod apps;
mod bar;
mod calendar;
mod closer;
mod controls;
mod dbus;
mod events;
mod files;
mod gpu;
mod icons;
mod launcher;
mod lock;
mod notify;
mod now;
mod osd;
mod screens;
mod search;
mod shelf;
mod sources;
mod system;
mod tray;
mod ui;
mod welcome;

use bar::Bar;
use launcher::Launcher;
use lock::Lock;
use nordlys_config::Config;
use notify::Toasts;
use shelf::{Shelf, ZONE};
use std::process::ExitCode;

const USAGE: &str = "\
usage: nordlys-shell [bar] [--window] [--output NAME]
       nordlys-shell launcher [--window] [--query TEXT]
       nordlys-shell shelf [--window] [--output NAME]
       nordlys-shell notify [--window]
       nordlys-shell close-button [--output NAME]
       nordlys-shell lock [--window | --check]
       nordlys-shell osd volume|brightness up|down|show · osd volume|mic mute
       nordlys-shell tray                 (print the tray's icons and menus)
       nordlys-shell welcome [--first-run] [--window]

On Hyprland, bar, shelf and close-button without --output run
one copy per screen, following screens as they come and go.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    // The first bare word that is not the value of an option.
    let piece = args
        .iter()
        .enumerate()
        .find(|(i, a)| {
            !a.starts_with("--")
                && (*i == 0 || !matches!(args[i - 1].as_str(), "--output" | "--query"))
        })
        .map(|(_, a)| a.as_str());
    let query = args
        .iter()
        .position(|a| a == "--query")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_default();
    let output = args
        .iter()
        .position(|a| a == "--output")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let window = args.iter().any(|a| a == "--window") || !wayland;

    let config = match Config::load(&nordlys_config::default_path()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nordlys-shell: {e}");
            return ExitCode::FAILURE;
        }
    };
    // A hidden shelf starts nothing. It exits cleanly, so systemd doesn't restart it.
    if piece == Some("shelf") && !config.stage.shelf {
        return ExitCode::SUCCESS;
    }
    if window && !wayland {
        eprintln!("nordlys-shell: not on Wayland, opening a preview window");
    }

    // One per screen: this process only watches the screens.
    let per_screen = matches!(piece, None | Some("bar" | "shelf" | "close-button"));
    if per_screen && !window && output.is_none() {
        let name = piece.unwrap_or("bar");
        match screens::supervise(&sources::Compositor::detect(), name) {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) if e == screens::NO_SCREENS => {}
            Err(e) => eprintln!("nordlys-shell {name}: {e}; running on the active screen only"),
        }
    }

    // The pieces that stay running. The lock screen is left as it was: it
    // must not depend on one driver.
    if !window && matches!(piece, None | Some("bar" | "shelf" | "close-button" | "notify")) {
        // SAFETY: nothing has started a thread yet.
        unsafe { gpu::prefer_intel() };
    }

    let result = match (piece, window) {
        (None | Some("bar"), false) => bar_layer(config, output),
        (None | Some("bar"), true) => bar_window(config),
        (Some("launcher"), false) => launcher_layer(config, query),
        (Some("launcher"), true) => launcher_window(config, query),
        (Some("shelf"), false) => shelf_layer(config, output),
        (Some("shelf"), true) => shelf_window(config),
        (Some("close-button"), false) => close_button_layer(output),
        (Some("close-button"), true) => Err("the close button needs Hyprland".into()),
        (Some("notify"), false) => notify_layer(config),
        (Some("notify"), true) => notify_window(config),
        (Some("osd"), _) => osd_show(config, &args, window),
        (Some("tray"), _) => tray::print(),
        // Only once, at the first login.
        (Some("welcome"), _) if args.iter().any(|a| a == "--first-run") && welcome::seen() => {
            Ok(())
        }
        (Some("welcome"), false) => welcome_layer(config),
        (Some("welcome"), true) => welcome_window(config),
        (Some("lock"), _) if args.iter().any(|a| a == "--check") => lock_check(),
        (Some("lock"), false) => lock_session(config),
        (Some("lock"), true) => lock_window(config),
        (Some(other), _) => {
            eprintln!("unknown piece {other:?}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nordlys-shell: {e}");
            ExitCode::FAILURE
        }
    }
}

fn bar_layer(config: Config, output: Option<String>) -> Result<(), String> {
    use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    let (top, below) = (config.gaps.topbar as u32, config.bar.gap as u32);
    let height = bar::surface(top, below);
    let screen = output.clone();
    iced_layershell::application(
        move || Bar::new(&config, false, screen.clone()),
        || "nordlys-bar".to_owned(),
        Bar::update,
        Bar::view,
    )
    .style(Bar::style)
    .subscription(Bar::subscription)
    .settings(Settings {
        default_font: bar::INTER,
        fonts: vec![icons::FONT_BYTES.into()],
        layer_settings: LayerShellSettings {
            anchor: Anchor::Top | Anchor::Left | Anchor::Right,
            layer: Layer::Top,
            size: Some((0, height)),
            // Reserve the top gap and the bar; the gap below belongs to
            // whatever sits under the bar (windows, the shelf).
            exclusive_zone: bar::reserved(top) as i32,
            keyboard_interactivity: KeyboardInteractivity::None,
            start_mode: output
                .map(StartMode::TargetScreen)
                .unwrap_or(StartMode::Active),
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
    .map_err(|e| e.to_string())
}

/// Change a level, then show it: in the pill that's already up, else in
/// a new one (or, without Wayland, on stdout).
fn osd_show(config: Config, args: &[String], window: bool) -> Result<(), String> {
    use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    let rest: Vec<String> = args
        .iter()
        .skip_while(|a| *a != "osd")
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let (kind, change) = osd::parse(&rest)?;
    let level = osd::apply(kind, change).ok_or("couldn't read the level back")?;
    if window {
        println!(
            "{kind:?} {}%{}",
            level.percent,
            if level.muted { " (muted)" } else { "" }
        );
        return Ok(());
    }
    if osd::tell_running(level) {
        return Ok(());
    }
    osd::claim()?;
    iced_layershell::application(
        move || osd::Osd::new(&config, level),
        || "nordlys-osd".to_owned(),
        osd::Osd::update,
        osd::Osd::view,
    )
    .style(osd::Osd::style)
    .subscription(osd::Osd::subscription)
    .settings(Settings {
        default_font: bar::INTER,
        fonts: vec![icons::FONT_BYTES.into()],
        layer_settings: LayerShellSettings {
            anchor: Anchor::Bottom,
            layer: Layer::Overlay,
            size: Some((osd::WIDTH, osd::HEIGHT)),
            margin: (0, 0, osd::LIFT, 0),
            exclusive_zone: -1,
            keyboard_interactivity: KeyboardInteractivity::None,
            start_mode: StartMode::Active,
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
    .map_err(|e| e.to_string())
}

fn close_button_layer(output: Option<String>) -> Result<(), String> {
    use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
    use iced_layershell::settings::{LayerShellSettings, Settings};

    let compositor = sources::Compositor::detect();
    if !matches!(compositor, sources::Compositor::Hyprland(_)) {
        return Err("the close button needs Hyprland".into());
    }
    let screen = output.clone();
    iced_layershell::application(
        move || closer::Closer::new(compositor.clone(), screen.clone()),
        || "nordlys-close".to_owned(),
        closer::Closer::update,
        closer::Closer::view,
    )
    .style(closer::Closer::style)
    .subscription(closer::Closer::subscription)
    .settings(Settings {
        default_font: bar::INTER,
        fonts: vec![icons::FONT_BYTES.into()],
        layer_settings: LayerShellSettings {
            // Placed by margins from the screen's top left, ignoring the
            // bar's and rails' zones, so window coordinates work as is.
            anchor: Anchor::Top | Anchor::Left,
            layer: Layer::Top,
            size: Some((1, 1)),
            exclusive_zone: -1,
            keyboard_interactivity: KeyboardInteractivity::None,
            start_mode: screen_mode(output.clone()),
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
    .map_err(|e| e.to_string())
}

fn screen_mode(output: Option<String>) -> iced_layershell::settings::StartMode {
    use iced_layershell::settings::StartMode;
    output
        .map(StartMode::TargetScreen)
        .unwrap_or(StartMode::Active)
}

fn bar_window(config: Config) -> Result<(), String> {
    iced::application(
        move || Bar::new(&config, true, None),
        Bar::update,
        Bar::view,
    )
    .title("Nordlys bar · preview")
    .style(Bar::style)
    .subscription(Bar::subscription)
    .default_font(bar::INTER)
    .font(icons::FONT_BYTES)
    .window_size((1440.0, 700.0))
    .resizable(false)
    .run()
    .map_err(|e| e.to_string())
}

fn launcher_layer(config: Config, query: String) -> Result<(), String> {
    use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    iced_layershell::application(
        move || Launcher::new(&config, false, query.clone()),
        || "nordlys-launcher".to_owned(),
        Launcher::update,
        Launcher::view,
    )
    .style(Launcher::style)
    .subscription(Launcher::subscription)
    .settings(Settings {
        default_font: bar::INTER,
        fonts: vec![icons::FONT_BYTES.into()],
        layer_settings: LayerShellSettings {
            // Cover the whole output: the scrim dims everything, the panel
            // sits near the top, and clicking the scrim closes it.
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            layer: Layer::Overlay,
            size: Some((0, 0)),
            exclusive_zone: -1,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            start_mode: StartMode::Active,
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
    .map_err(|e| e.to_string())
}

fn welcome_layer(config: Config) -> Result<(), String> {
    use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    iced_layershell::application(
        move || welcome::Welcome::new(&config, false),
        || "nordlys-welcome".to_owned(),
        welcome::Welcome::update,
        welcome::Welcome::view,
    )
    .style(welcome::Welcome::style)
    .subscription(welcome::Welcome::subscription)
    .settings(Settings {
        default_font: bar::INTER,
        fonts: vec![icons::FONT_BYTES.into()],
        layer_settings: LayerShellSettings {
            anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
            layer: Layer::Overlay,
            size: Some((0, 0)),
            exclusive_zone: -1,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            start_mode: StartMode::Active,
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
    .map_err(|e| e.to_string())
}

fn welcome_window(config: Config) -> Result<(), String> {
    iced::application(
        move || welcome::Welcome::new(&config, true),
        welcome::Welcome::update,
        welcome::Welcome::view,
    )
    .title("Welcome to Nordlys · preview")
    .style(welcome::Welcome::style)
    .subscription(welcome::Welcome::subscription)
    .default_font(bar::INTER)
    .font(icons::FONT_BYTES)
    .window_size((1000.0, 760.0))
    .run()
    .map_err(|e| e.to_string())
}

fn launcher_window(config: Config, query: String) -> Result<(), String> {
    iced::application(
        move || Launcher::new(&config, true, query.clone()),
        Launcher::update,
        Launcher::view,
    )
    .title("Nordlys launcher · preview")
    .style(Launcher::style)
    .subscription(Launcher::subscription)
    .default_font(bar::INTER)
    .font(icons::FONT_BYTES)
    .window_size((1440.0, 840.0))
    .resizable(false)
    .run()
    .map_err(|e| e.to_string())
}

/// Layer settings shared by the two rails: a full-height strip on one edge
/// that reserves its width, so the stage sits between them.
fn rail_settings(
    edge: iced_layershell::reexport::Anchor,
    output: Option<String>,
    margin: (i32, i32, i32, i32),
    exclusive: i32,
) -> iced_layershell::settings::Settings {
    use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    Settings {
        default_font: bar::INTER,
        fonts: vec![icons::FONT_BYTES.into()],
        layer_settings: LayerShellSettings {
            anchor: edge | Anchor::Top | Anchor::Bottom,
            layer: Layer::Top,
            size: Some((ZONE, 0)),
            margin,
            exclusive_zone: exclusive,
            keyboard_interactivity: KeyboardInteractivity::None,
            start_mode: output
                .map(StartMode::TargetScreen)
                .unwrap_or(StartMode::Active),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn shelf_layer(config: Config, output: Option<String>) -> Result<(), String> {
    use iced_layershell::reexport::Anchor;
    let (margin, exclusive) = shelf::placement(&config.bar, &config.gaps);
    let screen = output.clone();
    iced_layershell::application(
        move || Shelf::new(&config, false, screen.clone()),
        || "nordlys-shelf".to_owned(),
        Shelf::update,
        Shelf::view,
    )
    .style(Shelf::style)
    .subscription(Shelf::subscription)
    .settings(rail_settings(Anchor::Left, output, margin, exclusive))
    .run()
    .map_err(|e| e.to_string())
}

fn shelf_window(config: Config) -> Result<(), String> {
    iced::application(
        move || Shelf::new(&config, true, None),
        Shelf::update,
        Shelf::view,
    )
    .title("Nordlys shelf · preview")
    .style(Shelf::style)
    .subscription(Shelf::subscription)
    .default_font(bar::INTER)
    .font(icons::FONT_BYTES)
    .window_size((520.0, 720.0))
    .resizable(false)
    .run()
    .map_err(|e| e.to_string())
}

fn notify_layer(config: Config) -> Result<(), String> {
    use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
    use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};

    iced_layershell::application(
        move || Toasts::new(&config, false),
        || "nordlys-notify".to_owned(),
        Toasts::update,
        Toasts::view,
    )
    .style(Toasts::style)
    .subscription(Toasts::subscription)
    .settings(Settings {
        default_font: bar::INTER,
        fonts: vec![icons::FONT_BYTES.into()],
        layer_settings: LayerShellSettings {
            // Top right, under the bar. The
            // surface is resized to fit the toasts and shrinks to a pixel
            // when there are none.
            anchor: Anchor::Top | Anchor::Right,
            layer: Layer::Overlay,
            size: Some((1, 1)),
            exclusive_zone: 0,
            keyboard_interactivity: KeyboardInteractivity::None,
            start_mode: StartMode::Active,
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
    .map_err(|e| e.to_string())
}

fn notify_window(config: Config) -> Result<(), String> {
    iced::application(
        move || Toasts::new(&config, true),
        Toasts::update,
        Toasts::view,
    )
    .title("Nordlys notifications · preview")
    .style(Toasts::style)
    .subscription(Toasts::subscription)
    .default_font(bar::INTER)
    .font(icons::FONT_BYTES)
    .window_size((520.0, 560.0))
    .resizable(false)
    .run()
    .map_err(|e| e.to_string())
}

fn lock_session(config: Config) -> Result<(), String> {
    use iced_sessionlock::settings::Settings;

    iced_sessionlock::application(
        move || (Lock::new(&config, false), iced::Task::none()),
        Lock::update,
        Lock::view_output,
    )
    .style(Lock::style)
    .subscription(Lock::subscription)
    .settings(Settings {
        default_font: bar::INTER,
        fonts: vec![icons::FONT_BYTES.into()],
        ..Default::default()
    })
    .run()
    .map_err(|e| e.to_string())
}

fn lock_window(config: Config) -> Result<(), String> {
    eprintln!(
        "nordlys-shell lock: preview, nothing is locked; the password is {:?}",
        lock::PREVIEW_PASSWORD
    );
    iced::application(move || Lock::new(&config, true), Lock::update, Lock::view)
        .title("Nordlys lock · preview")
        .style(Lock::style)
        .subscription(Lock::subscription)
        .default_font(bar::INTER)
        .font(icons::FONT_BYTES)
        .window_size((1280.0, 800.0))
        .resizable(false)
        .run()
        .map_err(|e| e.to_string())
}

/// Try a password against PAM from the terminal, to know the lock screen
/// will accept it before relying on it.
fn lock_check() -> Result<(), String> {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};

    let (login, _) = lock::pam::current_user();
    println!(
        "Checking {login}'s password with PAM service {:?}.",
        lock::pam::service()
    );
    print!("Password: ");
    let _ = std::io::stdout().flush();
    let stty = |arg: &str| {
        Command::new("stty")
            .arg(arg)
            .stdin(Stdio::inherit())
            .status()
    };
    let _ = stty("-echo");
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    let _ = stty("echo");
    println!();
    read.map_err(|e| e.to_string())?;
    let password = lock::pam::Secret::new(line.trim_end_matches(['\n', '\r']).to_owned());
    // SAFETY: zero bytes keep the String valid.
    unsafe { line.as_mut_vec().fill(0) };
    match lock::pam::authenticate(&login, password) {
        Ok(()) => {
            println!("Accepted. The lock screen will unlock with this password.");
            Ok(())
        }
        Err(e) if e == "wrong" => Err("PAM did not accept that password".into()),
        Err(e) => Err(e),
    }
}
