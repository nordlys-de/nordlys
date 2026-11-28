//! nordlys-comp: the Nordlys compositor.
//!
//! Three ways to run it:
//!
//! - `nordlys-comp --tty` takes the screen, keyboard and pointer itself
//!   (DRM/KMS, libinput, libseat): the real session, from a TTY or a
//!   display manager. See [`tty`]. It's the default with no desktop around.
//! - `nordlys-comp` (or `--nested`) opens a window on the desktop you're
//!   in and runs the same compositor inside it. See [`winit`].
//! - `nordlys-comp --headless` runs the Stage model behind the IPC socket
//!   with no display at all, for scripts and tests.
//!
//! The compositor proper (protocols, input, layout, lock) is [`comp`],
//! drawing is [`render`] with [`blur`] and [`cursor`], shelf pictures are
//! [`preview`], and X11 apps come through [`xwayland`]. Every IPC client
//! gets a thread of its own, so one that stays connected (`subscribe`)
//! never holds up the others.

mod blur;
mod comp;
mod cursor;
mod layout;
mod preview;
mod render;
mod tty;
mod winit;
mod xwayland;

use nordlys_config::Config;
use nordlys_ipc::{Request, Response, ScreenState, StageState, WorkspaceState};
use nordlys_stage::Desk;
use smithay::reexports::calloop::channel::Sender;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};

struct State {
    desk: Desk<String>,
    config: Config,
    config_path: PathBuf,
    /// Windows are real (a display backend runs). Closing then asks the
    /// app, and app groups start their apps instead of naming them.
    live: bool,
    /// What the display backend still has to do for the requests served.
    effects: Vec<Effect>,
}

/// Work a request leaves for the display backend.
#[derive(Debug, Clone, PartialEq)]
enum Effect {
    /// Ask this window's app to close it.
    Close(String),
    /// Start a command, in this folder.
    Spawn { cmd: String, dir: Option<PathBuf> },
    /// Save the next frame here.
    Screenshot(PathBuf),
}

/// What `handle` asks the connection to do next.
#[derive(Debug, PartialEq)]
enum After {
    Answer,
    Subscribe,
}

impl State {
    fn new(config: Config, config_path: PathBuf) -> Self {
        Self {
            desk: Desk::new(),
            config,
            config_path,
            live: false,
            effects: Vec::new(),
        }
    }

    fn snapshot(&self) -> StageState {
        let s = self.desk.stage();
        StageState {
            main: s.main().cloned(),
            side: s.side().cloned(),
            shelf: s.shelf().cloned().collect(),
            workspace: self.desk.active(),
            workspaces: self
                .desk
                .workspaces()
                .map(|(n, st)| WorkspaceState {
                    n,
                    windows: (st.main().is_some() as u32
                        + st.side().is_some() as u32
                        + st.shelf().count() as u32),
                })
                .collect(),
            screens: self
                .desk
                .screens()
                .map(|(name, n)| {
                    let st = self.desk.workspace(n);
                    ScreenState {
                        name: name.to_owned(),
                        workspace: n,
                        focused: self.desk.focused_screen() == Some(name),
                        main: st.and_then(|s| s.main().cloned()),
                        side: st.and_then(|s| s.side().cloned()),
                        shelf: st.map(|s| s.shelf().cloned().collect()).unwrap_or_default(),
                    }
                })
                .collect(),
        }
    }

    /// A name no open window has yet: `foot`, then `foot#2`, `foot#3`…
    fn fresh_id(&self, app: &str) -> String {
        if !self.desk.contains(&app.to_owned()) {
            return app.to_owned();
        }
        (2..)
            .map(|i| format!("{app}#{i}"))
            .find(|id| !self.desk.contains(id))
            .expect("an unused id exists")
    }

    /// The lowest-numbered workspace with no windows.
    fn empty_workspace(&self) -> u32 {
        let busy: Vec<u32> = self
            .desk
            .workspaces()
            .filter(|(_, st)| st.main().is_some() || st.shelf().next().is_some())
            .map(|(n, _)| n)
            .collect();
        (1..).find(|n| !busy.contains(n)).expect("a free workspace")
    }

    /// An open window of `app` (`foot`, `foot#2`…), if any.
    fn running(&self, app: &str) -> Option<String> {
        let mut ids = self.desk.workspaces().flat_map(|(_, st)| {
            st.main()
                .into_iter()
                .chain(st.side())
                .chain(st.shelf())
                .cloned()
                .collect::<Vec<_>>()
        });
        ids.find(|id| id == app || id.strip_prefix(app).is_some_and(|r| r.starts_with('#')))
    }

    fn apply(&mut self, req: Request) -> Result<After, String> {
        let ok = |done: bool, why: &str| {
            if done {
                Ok(After::Answer)
            } else {
                Err(why.to_owned())
            }
        };
        let d = &mut self.desk;
        match req {
            Request::State | Request::Version => Ok(After::Answer),
            Request::Subscribe => Ok(After::Subscribe),
            Request::Open { .. } if self.live => {
                Err("windows open themselves here; start the app instead".into())
            }
            Request::Open { id } => {
                d.open(id);
                Ok(After::Answer)
            }
            // The window goes when its app has closed it.
            Request::Close { id } if self.live => {
                if !d.contains(&id) {
                    return Err(format!("no window {id:?}"));
                }
                self.effects.push(Effect::Close(id));
                Ok(After::Answer)
            }
            Request::Close { id } => ok(d.close(&id), &format!("no window {id:?}")),
            Request::Bring { id } => ok(d.bring(&id), &format!("no window {id:?}")),
            Request::Previous => ok(d.stage_mut().previous(), "shelf is empty"),
            Request::Cycle => ok(d.stage_mut().cycle(), "shelf is empty"),
            Request::TwoUp => ok(
                d.stage_mut().toggle_two_up(),
                "two-up needs a window on the shelf",
            ),
            Request::Swap => ok(d.stage_mut().swap(), "two-up is off"),
            Request::SendToShelf => ok(d.stage_mut().send_to_shelf(), "stage is empty"),
            Request::Slot { n } => ok(d.stage_mut().slot(n), &format!("shelf has no slot {n}")),
            Request::Workspace { n } => ok(d.switch(n), "workspaces start at 1"),
            Request::FocusScreen { name } => {
                ok(d.focus_screen(&name), &format!("no screen {name:?}"))
            }
            Request::MoveTo { n, id } => {
                let id = id
                    .or_else(|| d.stage().main().cloned())
                    .ok_or("stage is empty")?;
                ok(d.move_to(&id, n), &format!("can't move {id:?} to {n}"))
            }
            Request::Group { name } => {
                let g = self
                    .config
                    .group(&name)
                    .ok_or(format!("no group {name:?}"))?
                    .clone();
                if self.live {
                    self.launch_live(&g);
                    return Ok(After::Answer);
                }
                // Open in reverse so the first app ends on stage and the
                // rest line up on the shelf in the order the user chose.
                for app in g.apps.iter().rev() {
                    if g.reuse_running && self.desk.contains(app) {
                        self.desk.bring(app);
                    } else {
                        let id = self.fresh_id(app);
                        self.desk.open(id);
                    }
                }
                Ok(After::Answer)
            }
            Request::Reload => {
                self.config = Config::load(&self.config_path).map_err(|e| e.to_string())?;
                Ok(After::Answer)
            }
            Request::Screenshot { .. } if !self.live => Err("headless: nothing to show".into()),
            Request::Screenshot { path } => {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err("the screenshot path must be absolute".into());
                }
                self.effects.push(Effect::Screenshot(path));
                Ok(After::Answer)
            }
        }
    }

    /// Start a group's apps for real. They're started last first, so the
    /// first usually maps last and takes the stage.
    fn launch_live(&mut self, g: &nordlys_config::Group) {
        if g.open_in == nordlys_config::OpenIn::NewWorkspace {
            let n = self.empty_workspace();
            self.desk.switch(n);
        }
        for app in g.apps.iter().rev() {
            match self.running(app).filter(|_| g.reuse_running) {
                // Brought into the group's workspace, not visited.
                Some(id) => {
                    let n = self.desk.active();
                    if self.desk.workspace_of(&id) == Some(n) {
                        self.desk.bring(&id);
                    } else {
                        self.desk.move_to(&id, n);
                    }
                }
                None => self.effects.push(Effect::Spawn {
                    cmd: app.clone(),
                    dir: g.dir_path(),
                }),
            }
        }
    }

    #[cfg(test)]
    fn handle(&mut self, line: &str) -> (Response, After) {
        let parsed =
            nordlys_ipc::from_line::<Request>(line).map_err(|e| format!("bad request: {e}"));
        self.handle_parsed(parsed)
    }

    fn handle_parsed(&mut self, parsed: Result<Request, String>) -> (Response, After) {
        let version = matches!(parsed, Ok(Request::Version));
        match parsed.and_then(|r| self.apply(r)) {
            Ok(_) if version => (
                Response::Version {
                    version: nordlys_ipc::VERSION,
                },
                After::Answer,
            ),
            Ok(after) => (
                Response::Ok {
                    state: self.snapshot(),
                },
                after,
            ),
            Err(message) => (Response::Error { message }, After::Answer),
        }
    }
}

/// The state and everyone waiting to hear about changes to it.
#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State>>,
    subscribers: Arc<Mutex<Vec<UnixStream>>>,
    /// Pinged after every request, so a display backend can lay out
    /// windows again and carry out the effects.
    wake: Option<Sender<()>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A client thread that panicked leaves the state as it was; carry on.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn new(state: State) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
            subscribers: Arc::default(),
            wake: None,
        }
    }

    /// Tell every subscriber, dropping the ones that hung up.
    fn broadcast(&self, state: &StageState) {
        let line = nordlys_ipc::to_line(&Response::Ok {
            state: state.clone(),
        });
        lock(&self.subscribers).retain_mut(|s| s.write_all(line.as_bytes()).is_ok());
    }

    /// Change the state; if the stage changed, log it and tell subscribers.
    fn change<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let (r, changed) = {
            let mut state = lock(&self.state);
            let before = state.snapshot();
            let r = f(&mut state);
            let now = state.snapshot();
            (r, (now != before).then_some(now))
        };
        if let Some(now) = changed {
            log(&now);
            self.broadcast(&now);
        }
        r
    }

    /// Serve one request, as if it came over the socket.
    fn request(&self, parsed: Result<Request, String>) -> (Response, After) {
        let r = self.change(|s| s.handle_parsed(parsed));
        if let Some(wake) = &self.wake {
            let _ = wake.send(());
        }
        r
    }
}

/// Re-read nordlys.toml whenever it's saved. A file that doesn't parse
/// is reported and the last good one stays.
fn watch_config(shared: Shared, path: PathBuf) {
    let stamp = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut last = stamp(&path);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let now = stamp(&path);
        if now == last {
            continue;
        }
        last = now;
        match Config::load(&path) {
            Ok(c) => {
                lock(&shared.state).config = c;
                if let Some(wake) = &shared.wake {
                    let _ = wake.send(());
                }
                eprintln!("nordlys-comp: reloaded {}", path.display());
            }
            Err(e) => eprintln!("nordlys-comp: keeping the old settings: {e}"),
        }
    }
}

fn log(st: &StageState) {
    let on = st
        .screens
        .iter()
        .find(|s| s.focused)
        .map(|s| format!(" on {}", s.name))
        .unwrap_or_default();
    eprintln!(
        "workspace {}{on}: {} {} | shelf: {}",
        st.workspace,
        st.main.as_deref().unwrap_or("-"),
        st.side
            .as_deref()
            .map(|s| format!("+ {s}"))
            .unwrap_or_default(),
        st.shelf.join(", ")
    );
}

fn serve(shared: &Shared, stream: UnixStream) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let parsed =
            nordlys_ipc::from_line::<Request>(&line).map_err(|e| format!("bad request: {e}"));
        let (resp, after) = shared.request(parsed);
        out.write_all(nordlys_ipc::to_line(&resp).as_bytes())?;
        if after == After::Subscribe {
            lock(&shared.subscribers).push(out);
            return Ok(());
        }
    }
    Ok(())
}

/// Take the socket, unless another nordlys-comp is already on it.
fn bind(sock: &Path) -> Result<UnixListener, String> {
    if UnixStream::connect(sock).is_ok() {
        return Err(format!("already running on {}", sock.display()));
    }
    let _ = std::fs::remove_file(sock); // stale socket from a crash
    UnixListener::bind(sock).map_err(|e| format!("cannot bind {}: {e}", sock.display()))
}

/// Serve IPC clients and watch nordlys.toml, each on threads of their own.
fn start_ipc(shared: &Shared, listener: UnixListener) {
    {
        let shared = shared.clone();
        let path = lock(&shared.state).config_path.clone();
        std::thread::spawn(move || watch_config(shared, path));
    }
    let shared = shared.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let shared = shared.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = serve(&shared, s) {
                            eprintln!("nordlys-comp: client error: {e}");
                        }
                    });
                }
                Err(e) => eprintln!("nordlys-comp: accept failed: {e}"),
            }
        }
    });
}

const USAGE: &str = "\
usage: nordlys-comp [--tty | --nested] [--run CMD]...
       nordlys-comp --headless

  --tty       the session itself: drive the screens and input devices from
              a TTY or display manager (the default with no desktop around),
              Super as the Nordlys key
  --nested    a compositor in a window on the current desktop (the default
              when there is one): Alt as the Nordlys key so the desktop
              around it keeps Super
  --run CMD   start CMD inside once it's up (repeatable)
  --headless  the Stage model over IPC, no display";

fn main() -> ExitCode {
    let mut headless = false;
    let mut mode = None;
    let mut run = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--headless" => headless = true,
            "--nested" | "--winit" => mode = Some(comp::Mode::Nested),
            "--tty" | "--drm" => mode = Some(comp::Mode::Tty),
            "--run" => match args.next() {
                Some(cmd) => run.push(cmd),
                None => {
                    eprintln!("nordlys-comp: --run needs a command\n{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("nordlys-comp: unknown argument {other:?}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let has_desktop = ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()));
    let mode = mode.unwrap_or(if has_desktop {
        comp::Mode::Nested
    } else {
        comp::Mode::Tty
    });

    let config_path = nordlys_config::default_path();
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nordlys-comp: {e}");
            return ExitCode::FAILURE;
        }
    };

    if !headless {
        return match comp::run(mode, config, config_path, run) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nordlys-comp: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let sock = nordlys_ipc::socket_path();
    let listener = match bind(&sock) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nordlys-comp: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("nordlys-comp: headless, listening on {}", sock.display());
    let shared = Shared::new(State::new(config, config_path));
    start_ipc(&shared, listener);
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let config = Config::from_toml(include_str!("../../../config/nordlys.toml")).unwrap();
        State::new(config, PathBuf::new())
    }

    fn ok(s: &mut State, line: &str) -> StageState {
        match s.handle(line) {
            (Response::Ok { state }, _) => state,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn group_launch_puts_first_app_on_stage_rest_on_shelf_in_order() {
        let mut s = state();
        let state = ok(&mut s, r#"{"cmd":"group","name":"d"}"#);
        assert_eq!(state.main.as_deref(), Some("helix"));
        assert_eq!(state.shelf, ["foot", "nautilus"]);
    }

    #[test]
    fn a_second_launch_gets_windows_of_its_own() {
        let mut s = state();
        // "Research" doesn't reuse running apps.
        ok(&mut s, r#"{"cmd":"group","name":"r"}"#);
        let state = ok(&mut s, r#"{"cmd":"group","name":"r"}"#);
        assert_eq!(state.main.as_deref(), Some("firefox#2"));
        assert_eq!(state.shelf, ["helix#2", "firefox", "helix"]);
    }

    #[test]
    fn errors_are_reported_not_fatal() {
        let mut s = state();
        assert!(matches!(
            s.handle(r#"{"cmd":"bring","id":"nope"}"#).0,
            Response::Error { .. }
        ));
        assert!(matches!(s.handle("not json").0, Response::Error { .. }));
        ok(&mut s, r#"{"cmd":"state"}"#);
    }

    #[test]
    fn version_and_subscribe() {
        let mut s = state();
        assert_eq!(
            s.handle(r#"{"cmd":"version"}"#),
            (
                Response::Version {
                    version: nordlys_ipc::VERSION
                },
                After::Answer
            )
        );
        assert_eq!(s.handle(r#"{"cmd":"subscribe"}"#).1, After::Subscribe);
    }

    #[test]
    fn workspaces_over_ipc() {
        let mut s = state();
        ok(&mut s, r#"{"cmd":"open","id":"helix"}"#);
        ok(&mut s, r#"{"cmd":"open","id":"foot"}"#);
        let st = ok(&mut s, r#"{"cmd":"move-to","n":2}"#);
        assert_eq!(st.main.as_deref(), Some("helix"));
        assert_eq!(
            st.workspaces,
            [
                WorkspaceState { n: 1, windows: 1 },
                WorkspaceState { n: 2, windows: 1 }
            ]
        );
        let st = ok(&mut s, r#"{"cmd":"workspace","n":2}"#);
        assert_eq!((st.workspace, st.main.as_deref()), (2, Some("foot")));
    }

    #[test]
    fn screens_over_ipc() {
        let mut s = state();
        s.desk.set_screens(&["eDP-1".into(), "DP-1".into()]);
        ok(&mut s, r#"{"cmd":"open","id":"helix"}"#);
        let st = ok(&mut s, r#"{"cmd":"focus-screen","name":"DP-1"}"#);
        assert_eq!(st.workspace, 2);
        assert_eq!(st.screens.len(), 2);
        assert_eq!(st.screens[0].main.as_deref(), Some("helix"));
        assert!(st.screens[1].focused && !st.screens[0].focused);
        assert!(matches!(
            s.handle(r#"{"cmd":"focus-screen","name":"HDMI-A-9"}"#).0,
            Response::Error { .. }
        ));
        // Headless (no screens named), nothing is listed.
        assert!(ok(&mut state(), r#"{"cmd":"state"}"#).screens.is_empty());
    }

    #[test]
    fn subscribers_hear_about_changes_while_others_are_served() {
        use std::os::unix::net::UnixStream;
        let shared = Shared::new(state());
        // A subscriber that stays connected.
        let (watcher, server_side) = UnixStream::pair().unwrap();
        let sh = shared.clone();
        let t = std::thread::spawn(move || serve(&sh, server_side));
        (&watcher)
            .write_all(nordlys_ipc::to_line(&Request::Subscribe).as_bytes())
            .unwrap();
        let mut lines = BufReader::new(watcher).lines();
        assert!(lines.next().unwrap().unwrap().contains("\"ok\""));
        t.join().unwrap().unwrap();

        // Another client changes the stage; the subscriber is told.
        let (client, server_side) = UnixStream::pair().unwrap();
        let sh = shared.clone();
        let t = std::thread::spawn(move || serve(&sh, server_side));
        (&client)
            .write_all(b"{\"cmd\":\"open\",\"id\":\"foot\"}\n")
            .unwrap();
        let mut answer = String::new();
        BufReader::new(&client).read_line(&mut answer).unwrap();
        drop(client);
        t.join().unwrap().unwrap();
        let pushed: Response = nordlys_ipc::from_line(&lines.next().unwrap().unwrap()).unwrap();
        let Response::Ok { state } = pushed else {
            panic!("{pushed:?}")
        };
        assert_eq!(state.main.as_deref(), Some("foot"));
    }

    fn live() -> State {
        let mut s = state();
        s.live = true;
        s
    }

    #[test]
    fn live_close_asks_the_app_and_keeps_the_window_until_it_goes() {
        let mut s = live();
        s.desk.open("foot".into());
        let st = ok(&mut s, r#"{"cmd":"close","id":"foot"}"#);
        assert_eq!(st.main.as_deref(), Some("foot"));
        assert_eq!(s.effects, [Effect::Close("foot".into())]);
        assert!(matches!(
            s.handle(r#"{"cmd":"close","id":"nope"}"#).0,
            Response::Error { .. }
        ));
    }

    #[test]
    fn live_groups_start_apps_on_an_empty_workspace() {
        let mut s = live();
        s.desk.open("foot".into());
        let st = ok(&mut s, r#"{"cmd":"group","name":"d"}"#);
        assert_eq!(st.workspace, 2);
        let cmds: Vec<_> = s
            .effects
            .iter()
            .map(|e| match e {
                Effect::Spawn { cmd, .. } => cmd.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        // "Nordlys dev" reuses running apps: foot is brought, not started.
        assert_eq!(cmds, ["nautilus", "helix"]);
        assert_eq!(s.desk.workspace_of(&"foot".into()), Some(2));
    }

    #[test]
    fn live_mode_refuses_phantom_windows() {
        let mut s = live();
        assert!(matches!(
            s.handle(r#"{"cmd":"open","id":"x"}"#).0,
            Response::Error { .. }
        ));
    }

    #[test]
    fn running_matches_numbered_copies_only() {
        let mut s = state();
        s.desk.open("foot#2".into());
        s.desk.open("footbar".into());
        assert_eq!(s.running("foot").as_deref(), Some("foot#2"));
        assert_eq!(s.running("helix"), None);
    }
}
