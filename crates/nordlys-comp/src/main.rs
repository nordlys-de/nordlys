//! nordlys-comp: the Nordlys compositor.
//!
//! The Smithay backend isn't written yet. Until it is, `--headless` runs
//! the real Stage model behind the real IPC socket with no display, so
//! nordlysctl, the shell and scripts can be built against it today.

use nordlys_config::Config;
use nordlys_ipc::{Request, Response, StageState};
use nordlys_stage::{Home, Stage};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::ExitCode;

struct State {
    stage: Stage<String>,
    config: Config,
    config_path: PathBuf,
}

impl State {
    fn snapshot(&self) -> StageState {
        StageState {
            main: self.stage.main().cloned(),
            side: self.stage.side().cloned(),
            shelf: self.stage.shelf().cloned().collect(),
        }
    }

    fn apply(&mut self, req: Request) -> Result<(), String> {
        let s = &mut self.stage;
        let ok = |done: bool, why: &str| if done { Ok(()) } else { Err(why.to_owned()) };
        match req {
            Request::State => Ok(()),
            Request::Open { id, ambient } => {
                s.open(id, if ambient { Home::Ambient } else { Home::Shelf });
                Ok(())
            }
            Request::Close { id } => ok(s.close(&id), &format!("no window {id:?}")),
            Request::Bring { id } => ok(s.bring(&id), &format!("no window {id:?}")),
            Request::Previous => ok(s.previous(), "shelf is empty"),
            Request::Cycle => ok(s.cycle(), "shelf is empty"),
            Request::TwoUp => ok(s.toggle_two_up(), "two-up needs a window on the shelf"),
            Request::Swap => ok(s.swap(), "two-up is off"),
            Request::SendToShelf => ok(s.send_to_shelf(), "stage is empty"),
            Request::Slot { n } => ok(s.slot(n), &format!("shelf has no slot {n}")),
            Request::Group { name } => {
                let g = self
                    .config
                    .group(&name)
                    .ok_or(format!("no group {name:?}"))?
                    .clone();
                // Open in reverse so the first app ends on stage and the
                // rest line up on the shelf in the order the user chose.
                for app in g.apps.iter().rev() {
                    if g.reuse_running && self.stage.contains(app) {
                        self.stage.bring(app);
                    } else {
                        self.stage.open(app.clone(), Home::Shelf);
                    }
                }
                Ok(())
            }
            Request::Reload => {
                self.config = Config::load(&self.config_path).map_err(|e| e.to_string())?;
                Ok(())
            }
        }
    }

    fn handle(&mut self, line: &str) -> Response {
        let result = nordlys_ipc::from_line::<Request>(line)
            .map_err(|e| format!("bad request: {e}"))
            .and_then(|r| self.apply(r));
        match result {
            Ok(()) => Response::Ok {
                state: self.snapshot(),
            },
            Err(message) => Response::Error { message },
        }
    }
}

fn serve(state: &mut State, stream: UnixStream) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let resp = state.handle(&line);
        out.write_all(nordlys_ipc::to_line(&resp).as_bytes())?;
        let st = state.snapshot();
        eprintln!(
            "stage: {} {} | shelf: {}",
            st.main.as_deref().unwrap_or("-"),
            st.side
                .as_deref()
                .map(|s| format!("+ {s}"))
                .unwrap_or_default(),
            st.shelf.join(", ")
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    let headless = std::env::args().any(|a| a == "--headless");
    if !headless {
        eprintln!(
            "nordlys-comp: the display backend is not written yet.\n\
             Run `nordlys-comp --headless` for the Stage model over IPC."
        );
        return ExitCode::from(2);
    }

    let config_path = nordlys_config::default_path();
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nordlys-comp: {e}");
            return ExitCode::FAILURE;
        }
    };

    let sock = nordlys_ipc::socket_path();
    if UnixStream::connect(&sock).is_ok() {
        eprintln!("nordlys-comp: already running on {}", sock.display());
        return ExitCode::FAILURE;
    }
    let _ = std::fs::remove_file(&sock); // stale socket from a crash
    let listener = match UnixListener::bind(&sock) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nordlys-comp: cannot bind {}: {e}", sock.display());
            return ExitCode::FAILURE;
        }
    };
    eprintln!("nordlys-comp: headless, listening on {}", sock.display());

    let mut state = State {
        stage: Stage::new(),
        config,
        config_path,
    };
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                if let Err(e) = serve(&mut state, s) {
                    eprintln!("nordlys-comp: client error: {e}");
                }
            }
            Err(e) => eprintln!("nordlys-comp: accept failed: {e}"),
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let config = Config::from_toml(include_str!("../../../config/nordlys.toml")).unwrap();
        State {
            stage: Stage::new(),
            config,
            config_path: PathBuf::new(),
        }
    }

    #[test]
    fn group_launch_puts_first_app_on_stage_rest_on_shelf_in_order() {
        let mut s = state();
        let r = s.handle(r#"{"cmd":"group","name":"d"}"#);
        let Response::Ok { state } = r else {
            panic!("{r:?}")
        };
        assert_eq!(state.main.as_deref(), Some("helix"));
        assert_eq!(state.shelf, ["foot", "nautilus"]);
    }

    #[test]
    fn errors_are_reported_not_fatal() {
        let mut s = state();
        assert!(matches!(
            s.handle(r#"{"cmd":"bring","id":"nope"}"#),
            Response::Error { .. }
        ));
        assert!(matches!(s.handle("not json"), Response::Error { .. }));
        assert!(matches!(
            s.handle(r#"{"cmd":"state"}"#),
            Response::Ok { .. }
        ));
    }
}
