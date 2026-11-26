//! nordlys-settings: one window for the Nordlys desktop (appearance, the
//! stage, app groups, notifications, idle) and the system under it
//! (network, Bluetooth, sound, screens, power, time, apps, services).
//!
//! Usage:
//!   nordlys-settings [PAGE]      open on a page, e.g. `network`
//!   nordlys-settings --pages     list the pages

mod app;
mod icons;
mod pages;
mod store;
mod sys;
mod theme;
mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: nordlys-settings [PAGE]\n       nordlys-settings --pages");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--pages") {
        for p in app::Page::ALL {
            println!("{:<14} {}", p.slug(), p.title());
        }
        return ExitCode::SUCCESS;
    }
    let start = match args.iter().find(|a| !a.starts_with('-')) {
        None => app::Page::Appearance,
        Some(name) => match app::Page::from_slug(name) {
            Some(p) => p,
            None => {
                eprintln!("nordlys-settings: no page called {name:?}; try --pages");
                return ExitCode::from(2);
            }
        },
    };
    match app::run(start) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nordlys-settings: {e}");
            ExitCode::FAILURE
        }
    }
}
