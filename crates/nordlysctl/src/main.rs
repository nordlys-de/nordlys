//! nordlysctl: talk to nordlys-comp over its socket.

use nordlys_ipc::{Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

const USAGE: &str = "\
usage: nordlysctl [--json] <command>

  state                  show the stage and shelf
  bring <id>             put a window in the centre
  previous               Super K  - the window you just left
  cycle                  Super J  - walk the shelf
  two-up                 Super T  - toggle two-up
  swap                   Super Tab
  send-to-shelf          Super S
  slot <1-4>             Super Alt 1-4
  workspace <n>          Super 1-9
  move-to <n> [id]       Super Shift 1-9 - the main window, or id
  focus-screen <name>    Super Ctrl Left/Right - eDP-1, DP-2…
  group <name|key>       Super G  - launch an app group
  open <id> [--ambient]  register a window (testing)
  close <id>
  reload                 re-read nordlys.toml
  screenshot <file.png>  save what nordlys-comp shows
  watch                  print the stage after every change
  version                the protocol version nordlys-comp speaks";

fn parse(args: &[String]) -> Result<Request, String> {
    let arg = |i: usize, what: &str| {
        args.get(i)
            .cloned()
            .ok_or_else(|| format!("{} needs {what}", args[0]))
    };
    let cmd = args.first().ok_or_else(|| USAGE.to_owned())?;
    Ok(match cmd.as_str() {
        "state" => Request::State,
        "bring" => Request::Bring {
            id: arg(1, "a window id")?,
        },
        "previous" => Request::Previous,
        "cycle" => Request::Cycle,
        "two-up" => Request::TwoUp,
        "swap" => Request::Swap,
        "send-to-shelf" => Request::SendToShelf,
        "slot" => Request::Slot {
            n: arg(1, "a slot number")?
                .parse()
                .map_err(|_| "slot must be a number")?,
        },
        "workspace" => Request::Workspace {
            n: arg(1, "a workspace number")?
                .parse()
                .map_err(|_| "workspace must be a number")?,
        },
        "move-to" => Request::MoveTo {
            n: arg(1, "a workspace number")?
                .parse()
                .map_err(|_| "workspace must be a number")?,
            id: args.get(2).cloned(),
        },
        "focus-screen" => Request::FocusScreen {
            name: arg(1, "a screen name")?,
        },
        "group" => {
            let name = args[1..].join(" ");
            if name.trim().is_empty() {
                return Err("group needs a name or key".into());
            }
            Request::Group { name }
        }
        "open" => Request::Open {
            id: arg(1, "a window id")?,
            ambient: args.iter().any(|a| a == "--ambient"),
        },
        "close" => Request::Close {
            id: arg(1, "a window id")?,
        },
        "reload" => Request::Reload,
        "screenshot" => {
            let path = std::path::absolute(arg(1, "a file name")?).map_err(|e| e.to_string())?;
            Request::Screenshot {
                path: path.to_string_lossy().into_owned(),
            }
        }
        "watch" => Request::Subscribe,
        "version" => Request::Version,
        "-h" | "--help" | "help" => return Err(USAGE.to_owned()),
        other => return Err(format!("unknown command {other:?}\n\n{USAGE}")),
    })
}

fn connect(req: &Request) -> Result<BufReader<UnixStream>, String> {
    let path = nordlys_ipc::socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        format!(
            "cannot reach {} ({e}). Is nordlys-comp running?",
            path.display()
        )
    })?;
    stream
        .write_all(nordlys_ipc::to_line(req).as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(BufReader::new(stream))
}

fn read(reader: &mut BufReader<UnixStream>) -> Result<String, String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Err("nordlys-comp hung up".into()),
        Ok(_) => Ok(line),
        Err(e) => Err(e.to_string()),
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.first().is_some_and(|a| a == "--json");
    if json {
        args.remove(0);
    }
    let req = match parse(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let started = std::time::SystemTime::now();
    let mut reader = match connect(&req) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("nordlysctl: {e}");
            return ExitCode::FAILURE;
        }
    };
    loop {
        let line = match read(&mut reader) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("nordlysctl: {e}");
                return ExitCode::FAILURE;
            }
        };
        let code = show(&line, json);
        if let (Request::Screenshot { path }, true) = (&req, code == ExitCode::SUCCESS) {
            return wait_for_file(path, started);
        }
        // `watch` keeps printing until nordlys-comp goes away.
        if req != Request::Subscribe || code != ExitCode::SUCCESS {
            return code;
        }
        if !json {
            println!();
        }
    }
}

/// The compositor writes the file with its next frame; wait for it.
fn wait_for_file(path: &str, since: std::time::SystemTime) -> ExitCode {
    for _ in 0..60 {
        let fresh = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= since);
        if fresh {
            eprintln!("saved {path}");
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    eprintln!("nordlysctl: no screenshot after 3 s (is a frame being drawn?)");
    ExitCode::FAILURE
}

fn show(line: &str, json: bool) -> ExitCode {
    if json {
        print!("{line}");
        return ExitCode::SUCCESS;
    }
    match nordlys_ipc::from_line::<Response>(line) {
        Ok(Response::Ok { state }) => {
            let stage = match (&state.main, &state.side) {
                (Some(m), Some(s)) => format!("{m}  |  {s}"),
                (Some(m), None) => m.clone(),
                _ => "(empty)".into(),
            };
            println!("workspace  {}", state.workspace);
            println!("stage  {stage}");
            let shelf: Vec<String> = state
                .shelf
                .iter()
                .enumerate()
                .map(|(i, w)| format!("{} {w}", i + 1))
                .collect();
            println!(
                "shelf  {}",
                if shelf.is_empty() {
                    "(empty)".into()
                } else {
                    shelf.join("   ")
                }
            );
            ExitCode::SUCCESS
        }
        Ok(Response::Version { version }) => {
            println!(
                "protocol {version} (nordlysctl speaks {})",
                nordlys_ipc::VERSION
            );
            ExitCode::SUCCESS
        }
        Ok(Response::Error { message }) => {
            eprintln!("nordlysctl: {message}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("nordlysctl: bad reply: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Request, String> {
        parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn parses_commands() {
        assert_eq!(p("two-up").unwrap(), Request::TwoUp);
        assert_eq!(p("slot 2").unwrap(), Request::Slot { n: 2 });
        assert_eq!(
            p("group Nordlys dev").unwrap(),
            Request::Group {
                name: "Nordlys dev".into()
            }
        );
        assert_eq!(
            p("open chat --ambient").unwrap(),
            Request::Open {
                id: "chat".into(),
                ambient: true
            }
        );
        assert_eq!(p("workspace 3").unwrap(), Request::Workspace { n: 3 });
        assert_eq!(
            p("move-to 2 foot").unwrap(),
            Request::MoveTo {
                n: 2,
                id: Some("foot".into())
            }
        );
        assert_eq!(p("watch").unwrap(), Request::Subscribe);
        assert_eq!(
            p("focus-screen DP-1").unwrap(),
            Request::FocusScreen {
                name: "DP-1".into()
            }
        );
        assert!(p("focus-screen").is_err());
        let Request::Screenshot { path } = p("screenshot shot.png").unwrap() else {
            panic!("not a screenshot request");
        };
        assert!(
            path.starts_with('/') && path.ends_with("/shot.png"),
            "{path}"
        );
        assert!(p("screenshot").is_err());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(p("").is_err());
        assert!(p("bring").is_err());
        assert!(p("slot x").is_err());
        assert!(p("group").is_err());
        assert!(p("fly").is_err());
    }
}
