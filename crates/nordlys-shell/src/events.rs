//! "Something changed" from the compositor, so the bar, shelf and close
//! button redraw when the desk changes instead of asking every second.
//!
//! * Hyprland writes one line per event to `.socket2.sock`, next to its
//!   request socket.
//! * nordlys-comp streams its state after a `subscribe` request.
//!
//! Both are read on a thread of their own. When the socket goes away (the
//! compositor restarted), the thread waits a moment and connects again.

use crate::sources::Compositor;
use futures::channel::mpsc;
use iced::Subscription;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The desk changed: workspaces, windows, focus or a title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Changed;

/// Where to listen, if this compositor can tell us about changes.
fn source(compositor: &Compositor) -> Option<Source> {
    match compositor {
        Compositor::Hyprland(requests) => {
            Some(Source::Hyprland(requests.with_file_name(".socket2.sock")))
        }
        Compositor::Nordlys(sock) => Some(Source::Nordlys(sock.clone())),
        Compositor::Demo => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Source {
    Hyprland(PathBuf),
    Nordlys(PathBuf),
}

/// Fires on every change the compositor reports. Nothing in the demo.
pub fn changes(compositor: &Compositor) -> Subscription<Changed> {
    match source(compositor) {
        Some(src) => Subscription::run_with(src, |src| listen(src.clone())),
        None => Subscription::none(),
    }
}

fn listen(src: Source) -> mpsc::UnboundedReceiver<Changed> {
    let (tx, rx) = mpsc::unbounded();
    std::thread::spawn(move || {
        loop {
            // Each reconnect may have missed changes, so say so first.
            if tx.unbounded_send(Changed).is_err() {
                return;
            }
            let alive = match &src {
                Source::Hyprland(p) => follow_hyprland(p, &tx),
                Source::Nordlys(p) => follow_nordlys(p, &tx),
            };
            if !alive {
                return; // The subscription is gone.
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    rx
}

/// Hyprland events that change what the shell shows.
fn matters(line: &str) -> bool {
    let name = line.split_once(">>").map_or(line, |(n, _)| n);
    matches!(
        name,
        "workspace"
            | "workspacev2"
            | "focusedmon"
            | "focusedmonv2"
            | "activewindow"
            | "activewindowv2"
            | "openwindow"
            | "closewindow"
            | "movewindow"
            | "movewindowv2"
            | "createworkspace"
            | "createworkspacev2"
            | "destroyworkspace"
            | "destroyworkspacev2"
            | "renameworkspace"
            | "moveworkspace"
            | "moveworkspacev2"
            | "activespecial"
            | "activespecialv2"
            | "fullscreen"
            | "changefloatingmode"
            | "windowtitle"
            | "windowtitlev2"
            | "monitoradded"
            | "monitoraddedv2"
            | "monitorremoved"
            | "monitorremovedv2"
            | "pin"
    )
}

/// Read events until the socket closes. False once nobody is listening.
fn follow_hyprland(path: &Path, tx: &mpsc::UnboundedSender<Changed>) -> bool {
    let Ok(sock) = UnixStream::connect(path) else {
        return !tx.is_closed();
    };
    for line in BufReader::new(sock).lines() {
        let Ok(line) = line else { break };
        if matters(&line) && tx.unbounded_send(Changed).is_err() {
            return false;
        }
    }
    !tx.is_closed()
}

fn follow_nordlys(path: &Path, tx: &mpsc::UnboundedSender<Changed>) -> bool {
    let Ok(mut sock) = UnixStream::connect(path) else {
        return !tx.is_closed();
    };
    let hello = nordlys_ipc::to_line(&nordlys_ipc::Request::Subscribe);
    if sock.write_all(hello.as_bytes()).is_err() {
        return !tx.is_closed();
    }
    // The first line answers the request; every later one is a change.
    for line in BufReader::new(sock).lines() {
        let Ok(_) = line else { break };
        if tx.unbounded_send(Changed).is_err() {
            return false;
        }
    }
    !tx.is_closed()
}

/// The new `nordlys.toml` each time it's saved, so colours change live.
/// A file that doesn't parse is reported once and skipped; the last good
/// one stays in use.
pub fn config_changes() -> Subscription<nordlys_config::Config> {
    Subscription::run(|| watch_config(nordlys_config::default_path(), Duration::from_secs(2)))
}

fn watch_config(path: PathBuf, every: Duration) -> mpsc::UnboundedReceiver<nordlys_config::Config> {
    let (tx, rx) = mpsc::unbounded();
    std::thread::spawn(move || {
        let stamp = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut last = stamp(&path);
        while !tx.is_closed() {
            std::thread::sleep(every);
            let now = stamp(&path);
            if now == last {
                continue;
            }
            last = now;
            match nordlys_config::Config::load(&path) {
                Ok(c) => {
                    if tx.unbounded_send(c).is_err() {
                        return;
                    }
                }
                Err(e) => eprintln!("nordlys-shell: keeping the old settings: {e}"),
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_desk_events_matter() {
        assert!(matters("workspacev2>>2,2"));
        assert!(matters("activewindow>>foot,~"));
        assert!(matters("closewindow>>5612ab"));
        assert!(!matters("activelayout>>kbd,us"));
        assert!(!matters("submap>>resize"));
        assert!(!matters("bell>>"));
    }

    #[test]
    fn hyprland_events_sit_beside_its_request_socket() {
        let c = Compositor::Hyprland("/run/user/1000/hypr/abc/.socket.sock".into());
        assert_eq!(
            source(&c),
            Some(Source::Hyprland(
                "/run/user/1000/hypr/abc/.socket2.sock".into()
            ))
        );
        assert_eq!(source(&Compositor::Demo), None);
    }

    #[test]
    fn a_saved_config_arrives_and_a_broken_one_is_skipped() {
        let dir = std::env::temp_dir().join(format!("nordlys-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nordlys.toml");
        std::fs::write(&path, "").unwrap();
        let mut rx = watch_config(path.clone(), Duration::from_millis(20));
        let next = |rx: &mut mpsc::UnboundedReceiver<nordlys_config::Config>| {
            futures::executor::block_on(futures::StreamExt::next(rx))
        };
        // Mtimes can be coarse; make sure each write lands on a new one.
        let bump = || std::thread::sleep(Duration::from_millis(1100));
        bump();
        std::fs::write(&path, "[theme]\nmode = \"bogus\"\n").unwrap();
        bump();
        std::fs::write(&path, "[theme]\nmode = \"light\"\n").unwrap();
        let c = next(&mut rx).unwrap();
        assert_eq!(c.theme.mode, nordlys_config::Mode::Light);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changes_arrive_and_the_listener_reconnects() {
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("nordlys-events-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".socket2.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let mut rx = listen(Source::Hyprland(path.clone()));
        let next = |rx: &mut mpsc::UnboundedReceiver<Changed>| {
            futures::executor::block_on(futures::StreamExt::next(rx))
        };
        // One on connect, then one per event that matters.
        assert_eq!(next(&mut rx), Some(Changed));
        let (mut conn, _) = listener.accept().unwrap();
        conn.write_all(b"activelayout>>kbd,us\nopenwindow>>1,2,foot,foot\n")
            .unwrap();
        assert_eq!(next(&mut rx), Some(Changed));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
