//! X11 apps, through xwayland-satellite: it runs Xwayland and shows its
//! windows to us as ordinary Wayland ones, so the compositor needs no X11
//! code of its own. Started once the compositor's socket is up, restarted
//! if it dies, and given up on if it keeps dying straight away.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Start xwayland-satellite on a free display, if it's installed. Returns
/// that display (":1"), for apps' `DISPLAY`.
pub fn start(wayland_display: &str) -> Option<String> {
    if !crate::comp::on_path("xwayland-satellite") {
        eprintln!("nordlys-comp: xwayland-satellite isn't installed, so X11 apps won't open");
        return None;
    }
    let n = (0..64).find(|n| free(*n))?;
    let display = format!(":{n}");
    let (wl, d) = (wayland_display.to_owned(), display.clone());
    std::thread::spawn(move || supervise(&wl, &d));
    Some(display)
}

/// Nobody has the display: no socket and no lock file.
fn free(n: u32) -> bool {
    !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
        && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
}

fn supervise(wayland_display: &str, display: &str) {
    let mut quick_deaths = 0;
    loop {
        let started = Instant::now();
        let status = Command::new("xwayland-satellite")
            .arg(display)
            .env("WAYLAND_DISPLAY", wayland_display)
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .status();
        match status {
            Err(e) => {
                eprintln!("nordlys-comp: cannot start xwayland-satellite: {e}");
                return;
            }
            Ok(s) => eprintln!("nordlys-comp: xwayland-satellite exited ({s})"),
        }
        if started.elapsed() < Duration::from_secs(5) {
            quick_deaths += 1;
            if quick_deaths >= 5 {
                eprintln!("nordlys-comp: xwayland-satellite keeps failing; X11 apps are off");
                return;
            }
        } else {
            quick_deaths = 0;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
