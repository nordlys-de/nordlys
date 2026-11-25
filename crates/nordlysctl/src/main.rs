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
  group <name|key>       Super G  - launch an app group
  open <id> [--ambient]  register a window (testing)
  close <id>
  reload                 re-read nordlys.toml";

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
        "-h" | "--help" | "help" => return Err(USAGE.to_owned()),
        other => return Err(format!("unknown command {other:?}\n\n{USAGE}")),
    })
}

fn send(req: &Request) -> Result<String, String> {
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
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(line)
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
    let line = match send(&req) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nordlysctl: {e}");
            return ExitCode::FAILURE;
        }
    };
    if json {
        print!("{line}");
        return ExitCode::SUCCESS;
    }
    match nordlys_ipc::from_line::<Response>(&line) {
        Ok(Response::Ok { state }) => {
            let stage = match (&state.main, &state.side) {
                (Some(m), Some(s)) => format!("{m}  |  {s}"),
                (Some(m), None) => m.clone(),
                _ => "(empty)".into(),
            };
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
