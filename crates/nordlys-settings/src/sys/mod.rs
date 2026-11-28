//! Talking to the system. Everything here blocks, so pages run it through
//! [`job`], on a thread of its own, and the window never waits on a slow
//! service. Each tool gets a time limit: some (bluetoothctl without
//! bluetoothd) never return.

pub mod about;
pub mod apps;
pub mod audio;
pub mod bluetooth;
pub mod datetime;
pub mod display;
pub mod logind;
pub mod network;
pub mod notify;
pub mod power;
pub mod services;

use iced::Task;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Reads: answers come back quickly or not at all.
pub const QUICK: Duration = Duration::from_millis(1500);
/// Changes that talk to hardware or the network (connecting to Wi-Fi,
/// pairing) take longer.
pub const SLOW: Duration = Duration::from_secs(45);

/// Run `work` on its own thread and hand the answer to `done`. A panic in
/// `work` arrives as an error rather than taking the window down.
pub fn job<T, M>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl FnOnce(Result<T, String>) -> M + Send + 'static,
) -> Task<M>
where
    T: Send + 'static,
    M: Send + 'static,
{
    Task::perform(
        async move {
            let (tx, rx) = futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(work());
            });
            rx.await
                .unwrap_or_else(|_| Err("that went wrong unexpectedly".into()))
        },
        done,
    )
}

fn wait_within(mut child: Child, limit: Duration) -> Option<(ExitStatus, Child)> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some((status, child)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// A tool's output, or why it failed: its own message when it gave one.
pub fn run_for(limit: Duration, cmd: &str, args: &[&str]) -> Result<String, String> {
    let child = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("{cmd} is not installed"),
            _ => format!("{cmd}: {e}"),
        })?;
    // Read both pipes on threads so a chatty tool can't fill one and stall.
    let mut child = child;
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut v);
        }
        v
    });
    let err_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut v);
        }
        v
    });
    let Some((status, _)) = wait_within(child, limit) else {
        return Err(format!("{cmd} took too long"));
    };
    let out = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
    if status.success() {
        Ok(out.trim_end().to_owned())
    } else {
        let msg = [err.trim(), out.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("failed")
            .lines()
            .last()
            .unwrap_or("failed")
            .to_owned();
        Err(format!("{cmd}: {msg}"))
    }
}

pub fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    run_for(QUICK, cmd, args)
}

/// Start a program and leave it running on its own.
pub fn spawn(cmd: &str, args: &[&str]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{cmd}: {e}"))
}

/// A Nordlys program: the one beside this binary (a build tree), else
/// whatever PATH finds.
pub fn sibling(name: &str) -> String {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_owned())
}

/// Is `cmd` on PATH?
pub fn has(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|d| {
            let p = d.join(cmd);
            p.is_file()
                && std::fs::metadata(&p)
                    .map(|m| {
                        use std::os::unix::fs::PermissionsExt;
                        m.permissions().mode() & 0o111 != 0
                    })
                    .unwrap_or(false)
        })
    })
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

/// The data folder Nordlys was installed with (hyprland.lua, wallpaper…).
pub fn data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let beside = exe
        .as_deref()
        .and_then(Path::parent)
        .map(|b| b.join("../share/nordlys"));
    std::env::var_os("NORDLYS_DATA")
        .map(PathBuf::from)
        .into_iter()
        .chain(beside)
        .chain([
            "/usr/local/share/nordlys".into(),
            "/usr/share/nordlys".into(),
        ])
        .find(|d| d.join("hyprland.lua").is_file())
}

pub fn on_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// Run Lua in Hyprland (0.55+). Hyprland answers "ok" or the error.
pub fn hypr_eval(lua: &str) -> Result<(), String> {
    let out = run("hyprctl", &["eval", lua])?;
    let reply = out.trim();
    if reply.is_empty() || reply == "ok" {
        Ok(())
    } else {
        // "error: return hl.config(…);:1: unknown config key 'x'" → the reason.
        let reason = reply.rsplit(": ").next().unwrap_or(reply);
        Err(format!("Hyprland: {reason}"))
    }
}

/// Write a file whole or not at all: to a neighbour first, then renamed.
pub fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, contents).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// Split nmcli's terse output: `:` separates fields, `\:` is a literal colon.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    fields.last_mut().unwrap().push(n);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// `key=value` lines into pairs (systemd's `show` output, os-release).
pub fn key_values(s: &str) -> Vec<(String, String)> {
    s.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().trim_matches('"').to_owned()))
        .collect()
}

pub fn lookup<'a>(kv: &'a [(String, String)], key: &str) -> Option<&'a str> {
    kv.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_fields_keep_escaped_colons() {
        assert_eq!(
            split_terse(r"yes:Home 5G:67:WPA2:*:8C\:A3\:99"),
            ["yes", "Home 5G", "67", "WPA2", "*", "8C:A3:99"]
        );
        assert_eq!(split_terse(""), [""]);
    }

    #[test]
    fn tools_report_errors_and_time_limits() {
        assert_eq!(run("echo", &["hi"]).as_deref(), Ok("hi"));
        assert!(run("false", &[]).is_err());
        assert!(
            run("nordlys-no-such-tool", &[])
                .unwrap_err()
                .contains("not installed")
        );
        let t = Instant::now();
        assert!(run_for(Duration::from_millis(200), "sleep", &["5"]).is_err());
        assert!(t.elapsed() < Duration::from_secs(2));
        let e = run("sh", &["-c", "echo nope >&2; exit 3"]).unwrap_err();
        assert_eq!(e, "sh: nope");
    }

    #[test]
    fn key_value_lines() {
        let kv = key_values("NAME=\"Arch Linux\"\nID=arch\nbad line\n");
        assert_eq!(lookup(&kv, "NAME"), Some("Arch Linux"));
        assert_eq!(lookup(&kv, "ID"), Some("arch"));
        assert_eq!(lookup(&kv, "X"), None);
    }
}
