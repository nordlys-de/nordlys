//! The real `nordlysctl` process against a stand-in compositor.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;

fn socket() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "nctl-{}-{}.sock",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Accept one client, note its request, answer with `replies`, hang up.
fn serve_once(replies: &'static [&'static str]) -> (PathBuf, JoinHandle<String>) {
    let path = socket();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(&stream).read_line(&mut request).unwrap();
        for r in replies {
            writeln!(stream, "{r}").unwrap();
        }
        request.trim_end().to_owned()
    });
    (path, worker)
}

fn ctl(socket: &PathBuf, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nordlysctl"));
    c.args(args)
        .env_clear()
        .env("NORDLYS_SOCKET", socket)
        .stdin(Stdio::null());
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        c.env("LLVM_PROFILE_FILE", profile);
    }
    c.output().unwrap()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

const TWO_UP: &str = r#"{"status":"ok","state":{"main":"helix","side":"foot","shelf":["docs","files"],"workspace":2}}"#;
const EMPTY: &str = r#"{"status":"ok","state":{"main":null,"side":null,"shelf":[]}}"#;

#[test]
fn the_stage_is_printed_for_people() {
    let (path, server) = serve_once(&[TWO_UP]);
    let out = ctl(&path, &["state"]);
    assert_eq!(server.join().unwrap(), r#"{"cmd":"state"}"#);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        text(&out.stdout),
        "workspace  2\nstage  helix  |  foot\nshelf  1 docs   2 files\n"
    );
}

#[test]
fn an_empty_stage_says_so() {
    let (path, server) = serve_once(&[EMPTY]);
    let out = ctl(&path, &["state"]);
    server.join().unwrap();
    assert_eq!(
        text(&out.stdout),
        "workspace  1\nstage  (empty)\nshelf  (empty)\n"
    );
}

#[test]
fn json_passes_the_reply_through() {
    let (path, server) = serve_once(&[TWO_UP]);
    let out = ctl(&path, &["--json", "slot", "2"]);
    assert_eq!(server.join().unwrap(), r#"{"cmd":"slot","n":2}"#);
    assert_eq!(text(&out.stdout), format!("{TWO_UP}\n"));
}

#[test]
fn errors_go_to_stderr_with_exit_1() {
    let (path, server) = serve_once(&[r#"{"status":"error","message":"shelf is empty"}"#]);
    let out = ctl(&path, &["previous"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(text(&out.stderr), "nordlysctl: shelf is empty\n");
    assert_eq!(text(&out.stdout), "");
}

#[test]
fn watch_prints_every_change_until_the_server_leaves() {
    let (path, server) = serve_once(&[EMPTY, TWO_UP]);
    let out = ctl(&path, &["watch"]);
    assert_eq!(server.join().unwrap(), r#"{"cmd":"subscribe"}"#);
    assert_eq!(out.status.code(), Some(1));
    let printed = text(&out.stdout);
    assert_eq!(printed.matches("stage  ").count(), 2, "{printed}");
    assert!(
        printed.contains("shelf  (empty)\n\nworkspace  2"),
        "{printed}"
    );
    assert_eq!(text(&out.stderr), "nordlysctl: nordlys-comp hung up\n");
}

#[test]
fn watch_in_json_adds_no_blank_lines() {
    let (path, server) = serve_once(&[EMPTY, TWO_UP]);
    let out = ctl(&path, &["--json", "watch"]);
    server.join().unwrap();
    assert_eq!(text(&out.stdout), format!("{EMPTY}\n{TWO_UP}\n"));
}

#[test]
fn watch_stops_at_an_error_reply() {
    let (path, server) = serve_once(&[r#"{"status":"error","message":"no"}"#, EMPTY]);
    let out = ctl(&path, &["watch"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(text(&out.stdout), "");
}

#[test]
fn no_compositor_is_explained() {
    let path = socket();
    let out = ctl(&path, &["state"]);
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(err.contains("Is nordlys-comp running?"), "{err}");
    assert!(err.contains(path.to_str().unwrap()), "{err}");
}

#[test]
fn usage_mistakes_exit_2_without_connecting() {
    for args in [&[][..], &["--help"], &["--json"], &["slot", "x"]] {
        let out = ctl(&socket(), args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(!out.stderr.is_empty(), "{args:?}");
    }
}

const OK_REPLY: &str = r#"{"status":"ok","state":{"main":null,"side":null,"shelf":[]}}"#;

/// A compositor that answers `screenshot` and then, like the real one,
/// writes the file a little later (or never).
fn serve_screenshot(write_after: Option<u64>) -> (PathBuf, JoinHandle<String>) {
    let path = socket();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(&stream).read_line(&mut request).unwrap();
        writeln!(stream, "{OK_REPLY}").unwrap();
        if let Some(ms) = write_after {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let Ok(nordlys_ipc::Request::Screenshot { path }) = nordlys_ipc::from_line(&request)
            else {
                panic!("not a screenshot: {request}");
            };
            std::fs::write(path, "picture").unwrap();
        }
        request.trim_end().to_owned()
    });
    (path, worker)
}

#[test]
fn screenshot_waits_for_the_file_to_be_written() {
    let (path, server) = serve_screenshot(Some(200));
    let shot = path.with_extension("png");
    let _ = std::fs::remove_file(&shot);
    let out = ctl(&path, &["screenshot", shot.to_str().unwrap()]);
    assert!(server.join().unwrap().contains(r#""cmd":"screenshot""#));
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(text(&out.stderr), format!("saved {}\n", shot.display()));
    let _ = std::fs::remove_file(&shot);
}

#[test]
fn a_screenshot_that_never_arrives_fails() {
    let (path, server) = serve_screenshot(None);
    let shot = path.with_extension("png");
    // An old file with that name doesn't count.
    std::fs::write(&shot, "yesterday").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let out = ctl(&path, &["screenshot", shot.to_str().unwrap()]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("no screenshot after 3 s"),
        "{out:?}"
    );
    let _ = std::fs::remove_file(&shot);
}

#[test]
fn a_refused_screenshot_does_not_wait() {
    let (path, server) = serve_once(&[r#"{"status":"error","message":"no display"}"#]);
    let started = std::time::Instant::now();
    let out = ctl(&path, &["screenshot", "x.png"]);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(text(&out.stderr), "nordlysctl: no display\n");
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn screen_commands_reach_the_wire() {
    let (path, server) = serve_once(&[OK_REPLY]);
    assert!(ctl(&path, &["focus-screen", "DP-2"]).status.success());
    assert_eq!(
        server.join().unwrap(),
        r#"{"cmd":"focus-screen","name":"DP-2"}"#
    );
}
