//! One bar, shelf, ambient rail and close button on every screen.
//!
//! A layer surface belongs to one output, so `nordlys-shell bar` (without
//! `--output`) on Hyprland or nordlys-comp runs as a small supervisor: it starts
//! `nordlys-shell bar --output NAME` for each screen, starts one for a
//! screen that's plugged in, stops the one whose screen goes away, and
//! starts a crashed one again.

use crate::sources::Compositor;
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// What `supervise` says when there are no screens to watch: the caller
/// runs a single copy, quietly.
pub const NO_SCREENS: &str = "no screens to watch";

/// How often the screens and children are looked at.
const EVERY: Duration = Duration::from_secs(2);
/// A child that dies sooner than this after starting counts as a crash loop.
const QUICK: Duration = Duration::from_secs(5);
/// Give up on a screen's child after this many quick deaths in a row.
const PATIENCE: u32 = 5;

struct Kid {
    child: Child,
    started: Instant,
    quick_deaths: u32,
}

/// Run `piece` once per screen. Returns only if the screens can't be
/// listed (then the caller runs a single copy instead).
pub fn supervise(compositor: &Compositor, piece: &str) -> Result<(), String> {
    if let Compositor::Demo = compositor {
        return Err(NO_SCREENS.into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut kids: BTreeMap<String, Kid> = BTreeMap::new();
    let mut gave_up: BTreeSet<String> = BTreeSet::new();
    let mut first = true;
    loop {
        let Some(screens) = compositor.screens() else {
            if first {
                // An older nordlys-comp, say: one copy on the active screen.
                return Err(NO_SCREENS.into());
            }
            // The compositor is going away; the session will stop us.
            std::thread::sleep(EVERY);
            continue;
        };
        first = false;

        // Screens that went away.
        kids.retain(|name, kid| {
            let keep = screens.contains(name);
            if !keep {
                let _ = kid.child.kill();
                let _ = kid.child.wait();
            }
            keep
        });
        gave_up.retain(|name| screens.contains(name));

        // Children that exited.
        for (name, kid) in kids.iter_mut() {
            if let Ok(Some(status)) = kid.child.try_wait() {
                let quick = kid.started.elapsed() < QUICK;
                kid.quick_deaths = if quick { kid.quick_deaths + 1 } else { 0 };
                eprintln!("nordlys-shell {piece}: the one on {name} exited ({status})");
                if kid.quick_deaths >= PATIENCE {
                    eprintln!("nordlys-shell {piece}: giving up on {name}");
                    gave_up.insert(name.clone());
                    continue;
                }
                match spawn(&exe, piece, name) {
                    Ok(child) => {
                        kid.child = child;
                        kid.started = Instant::now();
                    }
                    Err(e) => eprintln!("nordlys-shell {piece}: {e}"),
                }
            }
        }
        kids.retain(|name, _| !gave_up.contains(name));

        // New screens.
        for name in &screens {
            if kids.contains_key(name) || gave_up.contains(name) {
                continue;
            }
            match spawn(&exe, piece, name) {
                Ok(child) => {
                    kids.insert(
                        name.clone(),
                        Kid {
                            child,
                            started: Instant::now(),
                            quick_deaths: 0,
                        },
                    );
                }
                Err(e) => eprintln!("nordlys-shell {piece}: {e}"),
            }
        }
        std::thread::sleep(EVERY);
    }
}

fn spawn(exe: &std::path::Path, piece: &str, screen: &str) -> Result<Child, String> {
    let mut cmd = Command::new(exe);
    cmd.args([piece, "--output", screen])
        .stdin(Stdio::null())
        // Children go when the supervisor does, however it ends.
        .process_group(0);
    // SAFETY: prctl is async-signal-safe and touches only this process.
    unsafe {
        cmd.pre_exec(|| {
            parent_death_signal();
            Ok(())
        });
    }
    cmd.spawn()
        .map_err(|e| format!("can't start the one on {screen}: {e}"))
}

/// Ask the kernel to send SIGTERM when the parent dies.
fn parent_death_signal() {
    use std::ffi::{c_int, c_ulong};
    unsafe extern "C" {
        fn prctl(option: c_int, arg2: c_ulong, ...) -> c_int;
    }
    const PR_SET_PDEATHSIG: c_int = 1;
    const SIGTERM: c_ulong = 15;
    // SAFETY: a plain syscall wrapper with integer arguments.
    unsafe {
        prctl(PR_SET_PDEATHSIG, SIGTERM);
    }
}
