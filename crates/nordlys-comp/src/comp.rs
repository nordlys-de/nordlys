//! The compositor proper: everything that doesn't depend on where the
//! pixels go. Two backends drive it: [`crate::winit`] (a window on the
//! desktop you're in) and [`crate::tty`] (the screen itself, from a TTY).
//!
//! Apps started in it are laid out by the Stage model from
//! [`nordlys_stage`]: the newest window takes the stage, the one it
//! replaces waits on the shelf (unmapped), two-up splits the stage, and
//! each workspace keeps a stage of its own. The IPC socket, nordlysctl and
//! nordlys-shell work against it exactly as against the headless mode,
//! because both share [`crate::State`]: a key binding is served as the
//! same request a client would send.
//!
//! Layer-shell surfaces (the bar, rails, launcher, toasts) are supported,
//! and what they reserve is taken off the stage. Dialogs float, centred
//! over their parent, and take the keyboard while they're up. The lock
//! screen uses ext-session-lock-v1: while it's up nothing else is drawn
//! and nothing else gets input. hypridle and friends get ext-idle-notify.
//!
//! With more than one screen, each shows a workspace of its own (the
//! Stage [`nordlys_stage::Desk`] keeps which), side by side in one
//! logical space. The screen under the pointer has the keyboard, as on
//! the Hyprland session; Nordlys+Ctrl+Left/Right moves it by keyboard.

use crate::layout::{self, Rect};
use crate::render;
use crate::{Effect, Shared, State, blur, cursor, lock, preview, xwayland};
use nordlys_config::{Config, Rgb};
use nordlys_ipc::{Request, Response};
use nordlys_stage::Home;
use smithay::backend::allocator::Format;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
    KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::{
    LayerSurface, PopupKind, PopupManager, Space, Window, WindowSurfaceType, layer_map_for_output,
};
use smithay::input::keyboard::{FilterResult, Keycode, ModifiersState, XkbConfig, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::channel::{self, Channel};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, LoopSignal, PostAction};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{
    ClientData, ClientId, DisconnectReason, GlobalId,
};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle, Resource};
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface,
    with_states,
};
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
};
use smithay::wayland::idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState};
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
    set_data_device_focus,
};
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
};
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData,
    WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::viewporter::ViewporterState;
use smithay::{
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_idle_inhibit,
    delegate_idle_notify, delegate_layer_shell, delegate_output, delegate_primary_selection,
    delegate_seat, delegate_session_lock, delegate_shm, delegate_viewporter,
    delegate_xdg_decoration, delegate_xdg_shell,
};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The slowest frame rate: what keeps clients (and screenshots) going
/// while nothing asks for frames.
const IDLE_FRAME: Duration = Duration::from_millis(250);

/// The accent ring around the window that has the keyboard, in two-up.
const RING: i32 = 2;
const RING_OFFSET: i32 = 5;

/// Where the compositor draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A window on the desktop you're already in.
    Nested,
    /// The screen itself, from a TTY or a display manager.
    Tty,
}

/// Which modifier is the Nordlys key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NordlysKey {
    Alt,
    Super,
}

impl NordlysKey {
    fn held(self, m: &ModifiersState) -> bool {
        match self {
            NordlysKey::Alt => m.alt,
            NordlysKey::Super => m.logo,
        }
    }

    fn name(self) -> &'static str {
        match self {
            NordlysKey::Alt => "Alt",
            NordlysKey::Super => "Super",
        }
    }
}

/// What a key binding does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Served exactly like the same request from a client.
    Ipc(Request),
    CloseFocused,
    MoveFocused(u32),
    WorkspaceBy(i32),
    FocusMain,
    FocusSide,
    /// Give the keyboard to the next screen (or the previous one).
    ScreenBy(i32),
    /// Send the focused window to the next screen's workspace, and follow.
    MoveToScreen(i32),
    Terminal,
    Launcher {
        commands: bool,
    },
    /// A command, through `sh -c`.
    Spawn(&'static str),
    /// Switch to this virtual terminal (TTY only).
    Vt(i32),
    Quit,
}

/// Programs the session keys start, each trying the usual suspects.
const EDITOR: &str = "for p in \"$NORDLYS_EDITOR\" gnome-text-editor kate code zeditor mousepad; do command -v \"$p\" >/dev/null && exec \"$p\"; done";
const FILES: &str = "for p in \"$NORDLYS_FILES\" nautilus thunar nemo dolphin; do command -v \"$p\" >/dev/null && exec \"$p\"; done";
const BROWSER: &str = "for p in \"$BROWSER\" firefox chromium zen-browser librewolf brave; do command -v \"$p\" >/dev/null && exec \"$p\"; done";
const SETTINGS: &str = "command -v nordlys-settings >/dev/null && exec nordlys-settings; exec xdg-open \"${XDG_CONFIG_HOME:-$HOME/.config}/nordlys/nordlys.toml\"";
const LOCK: &str = "exec nordlys-shell lock";
const SCREENSHOT: &str = "d=\"$(xdg-user-dir PICTURES 2>/dev/null || echo \"$HOME/Pictures\")/Screenshots\"; mkdir -p \"$d\"; exec nordlysctl screenshot \"$d/$(date +%Y-%m-%d_%H-%M-%S).png\"";

/// The Nordlys key bindings (the Nordlys key is already held). `sym` is
/// the key without Shift, so Shift+1 still reads as 1. With Ctrl, the
/// arrows act on screens; nothing else takes Ctrl.
fn binding(shift: bool, ctrl: bool, sym: u32) -> Option<Action> {
    use Action::*;
    let ipc = |r| Some(Ipc(r));
    if ctrl {
        return match (shift, sym) {
            (false, keysyms::KEY_Left) => Some(ScreenBy(-1)),
            (false, keysyms::KEY_Right) => Some(ScreenBy(1)),
            (true, keysyms::KEY_Left) => Some(MoveToScreen(-1)),
            (true, keysyms::KEY_Right) => Some(MoveToScreen(1)),
            _ => None,
        };
    }
    match (shift, sym) {
        (false, keysyms::KEY_Return) => Some(Terminal),
        (false, keysyms::KEY_space) => Some(Launcher { commands: false }),
        (true, keysyms::KEY_space) | (false, keysyms::KEY_g) => Some(Launcher { commands: true }),
        (false, keysyms::KEY_k) => ipc(Request::Previous),
        (false, keysyms::KEY_j) => ipc(Request::Cycle),
        (false, keysyms::KEY_t) => ipc(Request::TwoUp),
        (false, keysyms::KEY_Tab) => ipc(Request::Swap),
        (false, keysyms::KEY_s) => ipc(Request::SendToShelf),
        (false, keysyms::KEY_q) => Some(CloseFocused),
        (false, keysyms::KEY_e) => Some(Spawn(EDITOR)),
        (false, keysyms::KEY_n) => Some(Spawn(FILES)),
        (false, keysyms::KEY_b) => Some(Spawn(BROWSER)),
        (false, keysyms::KEY_comma) => Some(Spawn(SETTINGS)),
        (false, keysyms::KEY_l | keysyms::KEY_Escape) => Some(Spawn(LOCK)),
        (false, keysyms::KEY_bracketleft) => Some(WorkspaceBy(-1)),
        (false, keysyms::KEY_bracketright) => Some(WorkspaceBy(1)),
        (false, keysyms::KEY_Left) => Some(FocusMain),
        (false, keysyms::KEY_Right) => Some(FocusSide),
        (true, keysyms::KEY_Escape) => Some(Quit),
        (shift, d) if (keysyms::KEY_1..=keysyms::KEY_9).contains(&d) => {
            let n = d - keysyms::KEY_1 + 1;
            Some(if shift {
                MoveFocused(n)
            } else {
                Ipc(Request::Workspace { n })
            })
        }
        _ => None,
    }
}

/// Keys that work without the Nordlys key: media keys, Print, and the
/// VT switch (Ctrl+Alt+F1…F12). `sym` is the key as typed, modifiers
/// applied, since that's how the keymap marks the VT keys.
fn global_binding(sym: u32) -> Option<Action> {
    use Action::*;
    match sym {
        s if (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12).contains(&s) => {
            Some(Vt((s - keysyms::KEY_XF86Switch_VT_1 + 1) as i32))
        }
        keysyms::KEY_XF86AudioRaiseVolume => Some(Spawn("exec nordlys-shell osd volume up")),
        keysyms::KEY_XF86AudioLowerVolume => Some(Spawn("exec nordlys-shell osd volume down")),
        keysyms::KEY_XF86AudioMute => Some(Spawn("exec nordlys-shell osd volume mute")),
        keysyms::KEY_XF86AudioMicMute => Some(Spawn("exec nordlys-shell osd mic mute")),
        keysyms::KEY_XF86MonBrightnessUp => Some(Spawn("exec nordlys-shell osd brightness up")),
        keysyms::KEY_XF86MonBrightnessDown => Some(Spawn("exec nordlys-shell osd brightness down")),
        keysyms::KEY_XF86AudioPlay | keysyms::KEY_XF86AudioPause => {
            Some(Spawn("exec playerctl play-pause"))
        }
        keysyms::KEY_XF86AudioNext => Some(Spawn("exec playerctl next")),
        keysyms::KEY_XF86AudioPrev => Some(Spawn("exec playerctl previous")),
        keysyms::KEY_Print => Some(Spawn(SCREENSHOT)),
        _ => None,
    }
}

#[derive(Default)]
struct ClientState {
    compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

/// Where the pixels go.
pub enum Backend {
    Winit(Box<crate::winit::Winit>),
    Tty(Box<crate::tty::Tty>),
    /// Only while a backend has itself out to draw a frame.
    Taken,
}

/// The lock screen (ext-session-lock-v1).
pub enum Lock {
    Unlocked,
    Locked {
        /// Answered once every screen has drawn a frame without the
        /// desktop on it.
        pending: Option<SessionLocker>,
        /// The lock client's surface for each screen.
        surfaces: Vec<(Output, LockSurface)>,
        /// The screens that have drawn since the lock came.
        drawn: Vec<Output>,
    },
}

pub struct Comp {
    pub start: Instant,
    pub mode: Mode,
    pub dh: DisplayHandle,
    pub handle: LoopHandle<'static, Comp>,
    pub signal: LoopSignal,
    pub socket_name: String,
    pub ipc_path: PathBuf,
    nordlys_key: NordlysKey,

    pub backend: Backend,
    /// Every screen, in order, placed side by side in `space`.
    pub outputs: Vec<Output>,
    /// Each screen's wl_output global.
    globals: Vec<(Output, GlobalId)>,
    /// The screen that had the keyboard at the last layout, to bring the
    /// pointer along when the keyboard moves to another one.
    shown_focus: Option<String>,
    /// Each screen's stage area at the last layout, to notice a bar
    /// moving it.
    areas: Vec<(String, Rect)>,
    pub ring: [SolidColorBuffer; 4],
    pub blur: blur::Blur,
    pub cursor: cursor::Cursor,
    pub previews: preview::Previews,
    /// The X11 display xwayland-satellite serves, if it runs.
    pub x_display: Option<String>,

    pub space: Space<Window>,
    pub popups: PopupManager,
    /// Windows the Stage model knows, by id.
    pub windows: Vec<(String, Window)>,
    /// Toplevels that haven't made their first commit yet.
    unregistered: Vec<Window>,
    /// Dialogs: toplevels with a parent. They float over it.
    pub floating: Vec<Window>,
    /// The stage window with the keyboard (main or side).
    pub focus: Option<String>,
    /// A layer surface (the launcher, say) holding the keyboard.
    pub layer_focus: Option<WlSurface>,
    pub lock: Lock,
    /// Keys whose press was a binding, so their release isn't sent on.
    suppressed: Vec<Keycode>,
    /// Where to save the next frame.
    pub shots: Vec<PathBuf>,
    /// When the last frame was drawn.
    pub last_frame: Instant,
    /// Surfaces asking the screen to stay awake (video players).
    inhibitors: Vec<WlSurface>,

    pub shared: Shared,

    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    _xdg_decoration_state: XdgDecorationState,
    layer_shell_state: WlrLayerShellState,
    shm_state: ShmState,
    dmabuf_state: DmabufState,
    _dmabuf_global: Option<DmabufGlobal>,
    _output_manager_state: OutputManagerState,
    _viewporter_state: ViewporterState,
    seat_state: SeatState<Comp>,
    data_device_state: DataDeviceState,
    primary_selection_state: PrimarySelectionState,
    lock_state: SessionLockManagerState,
    idle_state: IdleNotifierState<Comp>,
    _idle_inhibit_state: IdleInhibitManagerState,
    pub seat: Seat<Comp>,
}

/// What a backend hands over once it can draw.
pub struct Ready {
    pub backend: Backend,
    /// The screens, already sized and placed.
    pub outputs: Vec<Output>,
    /// For dmabuf: the formats the renderer imports and its device.
    pub formats: Vec<Format>,
    pub device: Option<libc_dev_t>,
}

#[allow(non_camel_case_types)]
pub type libc_dev_t = u64;

pub fn run(
    mode: Mode,
    config: Config,
    config_path: PathBuf,
    startup: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<'static, Comp> = EventLoop::try_new()?;
    let display: Display<Comp> = Display::new()?;
    let dh = display.handle();

    let socket = ListeningSocketSource::new_auto()?;
    let socket_name = socket.socket_name().to_string_lossy().into_owned();

    // Nested, the compositor gets a socket of its own, so the desktop
    // around it (and its shell) keeps whatever it was using. On the
    // screen it's the desktop, and takes the usual one.
    let ipc_path = match mode {
        Mode::Tty => nordlys_ipc::socket_path(),
        Mode::Nested => std::env::var_os("NORDLYS_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                nordlys_ipc::socket_path().with_file_name(format!("nordlys-{socket_name}.sock"))
            }),
    };
    let listener = crate::bind(&ipc_path)?;

    let (wake, woken): (channel::Sender<()>, Channel<()>) = channel::channel();
    let mut state = State::new(config, config_path);
    state.live = true;
    let mut shared = Shared::new(state);
    shared.wake = Some(wake);
    crate::start_ipc(&shared, listener);

    let ready = match mode {
        Mode::Nested => crate::winit::init(&event_loop, &dh, &socket_name)?,
        Mode::Tty => crate::tty::init(&event_loop, &dh)?,
    };

    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_wl_seat(&dh, "nordlys");
    seat.add_keyboard(XkbConfig::default(), 400, 30)?;
    seat.add_pointer();

    let config = lock(&shared.state).config.clone();
    let accent = render::color(config.palette().accent, 0.9);
    let nordlys_key = match (std::env::var("NORDLYS_MOD").as_deref(), mode) {
        (Ok("super" | "logo"), _) => NordlysKey::Super,
        (Ok("alt"), _) => NordlysKey::Alt,
        (_, Mode::Tty) => NordlysKey::Super,
        (_, Mode::Nested) => NordlysKey::Alt,
    };

    let outputs = ready.outputs.clone();
    let mut space = Space::default();
    let mut globals = Vec::new();
    for o in &outputs {
        globals.push((o.clone(), o.create_global::<Comp>(&dh)));
        space.map_output(o, o.current_location());
    }

    // dmabuf with feedback, so GPU clients (wgpu, Vulkan, browsers) find
    // the render node; plain v3 when the device can't be told.
    let mut dmabuf_state = DmabufState::new();
    let dmabuf_global = match ready.device.and_then(|dev| {
        DmabufFeedbackBuilder::new(dev, ready.formats.clone())
            .build()
            .ok()
    }) {
        Some(feedback) => dmabuf_state.create_global_with_default_feedback::<Comp>(&dh, &feedback),
        None => dmabuf_state.create_global::<Comp>(&dh, ready.formats.clone()),
    };

    let handle = event_loop.handle();
    let preview_dir = nordlys_ipc::preview_dir(&ipc_path);
    let mut comp = Comp {
        start: Instant::now(),
        mode,
        dh: dh.clone(),
        handle: handle.clone(),
        signal: event_loop.get_signal(),
        socket_name: socket_name.clone(),
        ipc_path: ipc_path.clone(),
        nordlys_key,
        backend: ready.backend,
        outputs,
        globals,
        shown_focus: None,
        areas: Vec::new(),
        ring: std::array::from_fn(|_| SolidColorBuffer::new((0, 0), accent)),
        blur: blur::Blur::new(&config.theme),
        cursor: cursor::Cursor::load(),
        previews: preview::Previews::new(preview_dir),
        x_display: None,
        space,
        popups: PopupManager::default(),
        windows: Vec::new(),
        unregistered: Vec::new(),
        floating: Vec::new(),
        focus: None,
        layer_focus: None,
        lock: Lock::Unlocked,
        suppressed: Vec::new(),
        shots: Vec::new(),
        last_frame: Instant::now(),
        inhibitors: Vec::new(),
        shared,
        compositor_state: CompositorState::new::<Comp>(&dh),
        xdg_shell_state: XdgShellState::new::<Comp>(&dh),
        _xdg_decoration_state: XdgDecorationState::new::<Comp>(&dh),
        layer_shell_state: WlrLayerShellState::new::<Comp>(&dh),
        shm_state: ShmState::new::<Comp>(&dh, vec![]),
        dmabuf_state,
        _dmabuf_global: Some(dmabuf_global),
        _output_manager_state: OutputManagerState::new_with_xdg_output::<Comp>(&dh),
        _viewporter_state: ViewporterState::new::<Comp>(&dh),
        seat_state,
        data_device_state: DataDeviceState::new::<Comp>(&dh),
        primary_selection_state: PrimarySelectionState::new::<Comp>(&dh),
        lock_state: SessionLockManagerState::new::<Comp, _>(&dh, |_| true),
        idle_state: IdleNotifierState::new(&dh, handle.clone()),
        _idle_inhibit_state: IdleInhibitManagerState::new::<Comp>(&dh),
        seat,
    };

    handle.insert_source(socket, |stream, _, comp: &mut Comp| {
        if let Err(e) = comp
            .dh
            .insert_client(stream, Arc::new(ClientState::default()))
        {
            eprintln!("nordlys-comp: client refused: {e}");
        }
    })?;
    handle.insert_source(
        Generic::new(
            display,
            Interest::READ,
            smithay::reexports::calloop::Mode::Level,
        ),
        |_, display, comp: &mut Comp| {
            // SAFETY: the display is never dropped while the loop runs.
            unsafe { display.get_mut().dispatch_clients(comp)? };
            Ok(PostAction::Continue)
        },
    )?;
    handle.insert_source(Timer::from_duration(IDLE_FRAME), |_, _, comp: &mut Comp| {
        if comp.last_frame.elapsed() >= IDLE_FRAME {
            comp.render();
        }
        TimeoutAction::ToDuration(IDLE_FRAME)
    })?;
    handle.insert_source(Timer::immediate(), |_, _, comp: &mut Comp| {
        comp.shelf_tick();
        TimeoutAction::ToDuration(comp.shelf_interval())
    })?;
    handle.insert_source(woken, |event, _, comp: &mut Comp| {
        if let channel::Event::Msg(()) = event {
            comp.sync();
        }
    })?;

    let names: Vec<String> = comp.outputs.iter().map(Output::name).collect();
    comp.shared.change(|s| s.desk.set_screens(&names));
    comp.shown_focus = comp.focused_output().map(|o| o.name());
    comp.x_display = xwayland::start(&socket_name);
    let key = nordlys_key.name();
    let place = match mode {
        Mode::Nested => "nested",
        Mode::Tty => "on the screen",
    };
    eprintln!(
        "nordlys-comp: {place}, WAYLAND_DISPLAY={socket_name}, IPC on {}{}\n\
         nordlys-comp: {key}+Enter terminal, {key}+Space launcher, {key}+Shift+Esc quits",
        ipc_path.display(),
        comp.x_display
            .as_deref()
            .map(|d| format!(", X11 apps on DISPLAY={d}"))
            .unwrap_or_default(),
    );
    if mode == Mode::Nested {
        eprintln!(
            "nordlys-comp: from outside: NORDLYS_SOCKET={} nordlysctl watch",
            ipc_path.display()
        );
    }
    for cmd in &startup {
        comp.spawn(cmd, None);
    }

    event_loop.run(None, &mut comp, |comp| {
        comp.space.refresh();
        comp.popups.cleanup();
        let _ = comp.dh.flush_clients();
    })?;

    // Tell clients still watching that we're gone, and free the names.
    comp.previews.clear();
    drop(UnixStream::connect(&ipc_path));
    let _ = std::fs::remove_file(&ipc_path);
    Ok(())
}

pub fn app_id(t: &ToplevelSurface) -> String {
    with_states(t.wl_surface(), |s| {
        s.data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().ok()?.app_id.clone())
    })
    .map(|id| id.rsplit('.').next().unwrap_or(&id).to_lowercase())
    .filter(|id| !id.is_empty())
    .unwrap_or_else(|| "window".into())
}

fn rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

/// The first of these terminals that's installed.
fn terminal() -> &'static str {
    const TERMINALS: [&str; 8] = [
        "foot",
        "kitty",
        "alacritty",
        "ghostty",
        "wezterm",
        "konsole",
        "gnome-terminal",
        "xterm",
    ];
    TERMINALS.into_iter().find(|t| on_path(t)).unwrap_or("foot")
}

pub fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

/// nordlys-shell: the one built next to this binary, else from PATH.
fn shell_bin() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.join("nordlys-shell")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| "nordlys-shell".into())
}

impl Comp {
    pub fn locked(&self) -> bool {
        matches!(self.lock, Lock::Locked { .. })
    }

    /// Draw a frame now, or as soon as the screen can take one.
    pub fn render(&mut self) {
        match self.backend {
            Backend::Winit(_) => crate::winit::render(self),
            Backend::Tty(_) => crate::tty::render(self),
            Backend::Taken => {}
        }
    }

    /// Something changed on screen: get a frame drawn.
    pub fn request_redraw(&mut self) {
        match &mut self.backend {
            Backend::Winit(w) => w.request_redraw(),
            Backend::Tty(_) => crate::tty::schedule(self),
            Backend::Taken => {}
        }
    }

    /// Run `f` with the renderer, when there's one to use (a TTY that's
    /// been switched away from has none).
    pub fn with_renderer(&mut self, f: impl FnOnce(&mut Comp, &mut GlesRenderer)) {
        let mut backend = std::mem::replace(&mut self.backend, Backend::Taken);
        match &mut backend {
            Backend::Winit(w) => f(self, w.renderer()),
            Backend::Tty(t) => {
                if let Some(r) = t.renderer() {
                    f(self, r);
                }
            }
            Backend::Taken => {}
        }
        self.backend = backend;
    }

    /// A screen changed size or scale: lay everything out again.
    pub fn outputs_changed(&mut self) {
        for o in &self.outputs {
            layer_map_for_output(o).arrange();
        }
        if let Lock::Locked { surfaces, .. } = &self.lock {
            for (o, s) in surfaces {
                configure_lock(s, o);
            }
        }
        self.relayout();
    }

    /// The screens now connected, sized and placed (hotplug). Screens
    /// that went away take their layer surfaces with them; their
    /// workspaces stay, to be visited again.
    pub fn set_outputs(&mut self, outputs: Vec<Output>) {
        for gone in self.outputs.iter().filter(|o| !outputs.contains(o)) {
            self.space.unmap_output(gone);
            let layers: Vec<LayerSurface> = layer_map_for_output(gone).layers().cloned().collect();
            for l in layers {
                l.layer_surface().send_close();
                layer_map_for_output(gone).unmap_layer(&l);
            }
            self.blur.forget_screen(&gone.name());
            if let Some(i) = self.globals.iter().position(|(o, _)| o == gone) {
                // Clients get a moment to let go of it before it's gone.
                let (_, id) = self.globals.remove(i);
                self.dh.disable_global::<Comp>(id.clone());
                let removed = self.handle.insert_source(
                    Timer::from_duration(Duration::from_secs(5)),
                    move |_, _, comp: &mut Comp| {
                        comp.dh.remove_global::<Comp>(id.clone());
                        TimeoutAction::Drop
                    },
                );
                if let Err(e) = removed {
                    eprintln!("nordlys-comp: {e}");
                }
            }
            if let Lock::Locked { surfaces, .. } = &mut self.lock {
                surfaces.retain(|(o, _)| o != gone);
            }
        }
        for o in &outputs {
            if !self.globals.iter().any(|(g, _)| g == o) {
                self.globals
                    .push((o.clone(), o.create_global::<Comp>(&self.dh)));
            }
            self.space.map_output(o, o.current_location());
        }
        self.outputs = outputs;
        let names: Vec<String> = self.outputs.iter().map(Output::name).collect();
        self.shared.change(|s| s.desk.set_screens(&names));
        // The pointer may have been on a screen that's gone.
        let pos = self.pointer_location();
        if self.output_at(pos).is_none()
            && let Some(o) = self.focused_output()
        {
            let centre = self.output_centre(&o);
            self.move_pointer(centre);
        }
        self.outputs_changed();
    }

    /// The screen with the keyboard (the first, if the Stage hasn't named
    /// one yet).
    pub fn focused_output(&self) -> Option<Output> {
        let name = lock(&self.shared.state)
            .desk
            .focused_screen()
            .map(str::to_owned);
        name.and_then(|n| self.output_named(&n).cloned())
            .or_else(|| self.outputs.first().cloned())
    }

    fn output_named(&self, name: &str) -> Option<&Output> {
        self.outputs.iter().find(|o| o.name() == name)
    }

    /// The screen at `pos`, in the logical space they share.
    fn output_at(&self, pos: Point<f64, Logical>) -> Option<Output> {
        self.space.output_under(pos).next().cloned()
    }

    /// Where `o`'s top-left corner is in the logical space.
    fn output_origin(&self, o: &Output) -> Point<i32, Logical> {
        self.space
            .output_geometry(o)
            .map(|g| g.loc)
            .unwrap_or_default()
    }

    fn output_centre(&self, o: &Output) -> Point<f64, Logical> {
        let g = self.space.output_geometry(o).unwrap_or_default();
        Point::from((
            f64::from(g.loc.x) + f64::from(g.size.w) / 2.0,
            f64::from(g.loc.y) + f64::from(g.size.h) / 2.0,
        ))
    }

    /// The smallest rectangle around every screen.
    fn bounds(&self) -> Rectangle<i32, Logical> {
        self.outputs
            .iter()
            .filter_map(|o| self.space.output_geometry(o))
            .reduce(|a, b| a.merge(b))
            .unwrap_or_default()
    }

    // ---- Stage ------------------------------------------------------

    fn window(&self, id: &str) -> Option<&Window> {
        self.windows.iter().find(|(i, _)| i == id).map(|(_, w)| w)
    }

    fn id_of(&self, surface: &WlSurface) -> Option<String> {
        self.windows
            .iter()
            .find(|(_, w)| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .map(|(id, _)| id.clone())
    }

    /// Carry out what IPC requests left behind, and lay out again.
    fn sync(&mut self) {
        let effects = std::mem::take(&mut lock(&self.shared.state).effects);
        for e in effects {
            match e {
                Effect::Close(id) => {
                    if let Some(t) = self.window(&id).and_then(|w| w.toplevel()) {
                        t.send_close();
                    }
                }
                Effect::Spawn { cmd, dir } => self.spawn(&cmd, dir.as_deref()),
                Effect::Screenshot(path) => {
                    self.shots.push(path);
                    self.request_redraw();
                }
            }
        }
        // nordlys.toml may have changed: the theme's blur, say.
        let theme = lock(&self.shared.state).config.theme.clone();
        self.blur.configure(&theme);
        self.relayout();
    }

    /// The area windows may use on `o`: the screen minus what its layer
    /// surfaces (the bar, the rails) reserve, in the shared logical space.
    fn stage_area(&self, o: &Output) -> Rect {
        let at = self.output_origin(o);
        let zone = layer_map_for_output(o).non_exclusive_zone();
        let r = rect(zone);
        Rect::new(r.x + at.x, r.y + at.y, r.w, r.h)
    }

    /// Every screen's stage area, by name.
    fn stage_areas(&self) -> Vec<(String, Rect)> {
        self.outputs
            .iter()
            .map(|o| (o.name(), self.stage_area(o)))
            .collect()
    }

    /// Put every window where the Stage model says: on each screen, the
    /// main and side of the workspace it shows mapped in their frames;
    /// everything else unmapped; dialogs over their parent. Then hand the
    /// keyboard to whoever should have it, on the focused screen.
    pub fn relayout(&mut self) {
        let (screens, layout, split) = {
            let st = lock(&self.shared.state);
            let screens: Vec<(String, Option<String>, Option<String>)> = st
                .desk
                .screens()
                .map(|(name, n)| {
                    let stage = st.desk.workspace(n);
                    (
                        name.to_owned(),
                        stage.and_then(|s| s.main().cloned()),
                        stage.and_then(|s| s.side().cloned()),
                    )
                })
                .collect();
            (screens, st.config.stage.layout, st.config.stage.split)
        };
        let focused = self.focused_output().map(|o| o.name());
        self.areas = self.stage_areas();
        // (window, frame, screen)
        let mut visible: Vec<(String, Rect, String)> = Vec::new();
        for (screen, main, side) in screens {
            let Some(area) = self
                .areas
                .iter()
                .find(|(n, _)| *n == screen)
                .map(|(_, a)| *a)
            else {
                continue;
            };
            let (main_r, side_r) = layout::arrange(area, layout, split, side.is_some());
            visible.extend(main.map(|m| (m, main_r, screen.clone())));
            visible.extend(side.zip(side_r).map(|(s, r)| (s, r, screen.clone())));
        }
        // The keyboard stays on the focused screen.
        let on_focused = |id: &str| {
            visible
                .iter()
                .any(|(v, _, s)| v == id && Some(s) == focused.as_ref())
        };
        if !self.focus.as_deref().is_some_and(on_focused) {
            self.focus = visible
                .iter()
                .find(|(_, _, s)| Some(s) == focused.as_ref())
                .map(|(id, _, _)| id.clone());
        }

        for (id, w) in &self.windows {
            let Some(t) = w.toplevel() else { continue };
            match visible.iter().find(|(v, _, _)| v == id) {
                Some((_, r, _)) => {
                    t.with_pending_state(|s| {
                        s.size = Some((r.w, r.h).into());
                        for tiled in [
                            xdg_toplevel::State::TiledLeft,
                            xdg_toplevel::State::TiledRight,
                            xdg_toplevel::State::TiledTop,
                            xdg_toplevel::State::TiledBottom,
                        ] {
                            s.states.set(tiled);
                        }
                    });
                    w.set_activated(self.focus.as_deref() == Some(id));
                    if t.is_initial_configure_sent() {
                        t.send_pending_configure();
                    }
                    self.space.map_element(w.clone(), (r.x, r.y), false);
                }
                None => {
                    w.set_activated(false);
                    self.space.unmap_elem(w);
                }
            }
        }

        // Dialogs show while their parent does, on the parent's screen.
        let shown: Vec<(WlSurface, String)> = visible
            .iter()
            .filter_map(|(id, _, screen)| {
                Some((
                    self.window(id)?.toplevel()?.wl_surface().clone(),
                    screen.clone(),
                ))
            })
            .collect();
        for w in &self.floating {
            let Some(t) = w.toplevel() else { continue };
            let area = t
                .parent()
                .and_then(|p| shown.iter().find(|(s, _)| *s == p))
                .and_then(|(_, screen)| self.areas.iter().find(|(n, _)| n == screen))
                .map(|(_, a)| *a);
            match area {
                Some(area) => {
                    let g = w.geometry();
                    let f = layout::float(area, g.size.w, g.size.h);
                    self.space.map_element(w.clone(), (f.x, f.y), true);
                }
                None => self.space.unmap_elem(w),
            }
        }

        // The keyboard moved to another screen without the pointer (a key,
        // a workspace shown there): bring the pointer along.
        if focused != self.shown_focus {
            self.shown_focus = focused.clone();
            let pointer_on = self.output_at(self.pointer_location()).map(|o| o.name());
            if pointer_on != focused
                && let Some(o) = focused
                    .as_deref()
                    .and_then(|n| self.output_named(n))
                    .cloned()
            {
                let centre = self.output_centre(&o);
                self.move_pointer(centre);
            }
        }
        self.apply_focus();
        self.request_redraw();
    }

    /// Give the keyboard to the lock screen while it's up; else to a
    /// layer surface that asked for it, else the newest dialog on screen,
    /// else the focused stage window.
    fn apply_focus(&mut self) {
        let target = match &self.lock {
            Lock::Locked { surfaces, .. } => {
                let on = self.focused_output();
                surfaces
                    .iter()
                    .find(|(o, _)| Some(o) == on.as_ref())
                    .or(surfaces.first())
                    .map(|(_, s)| s.wl_surface().clone())
            }
            Lock::Unlocked => self
                .layer_focus
                .clone()
                .or_else(|| {
                    self.floating
                        .iter()
                        .rev()
                        .find(|w| self.space.element_location(w).is_some())
                        .and_then(|w| Some(w.toplevel()?.wl_surface().clone()))
                })
                .or_else(|| {
                    let id = self.focus.as_deref()?;
                    Some(self.window(id)?.toplevel()?.wl_surface().clone())
                }),
        };
        let kb = self.seat.get_keyboard().expect("seat has a keyboard");
        if kb.current_focus() != target {
            kb.set_focus(self, target, SERIAL_COUNTER.next_serial());
        }
    }

    /// The first commit of a toplevel: it joins the Stage (or floats, if
    /// it's a dialog) and is told its size.
    fn register(&mut self, w: Window) {
        let Some(t) = w.toplevel().cloned() else {
            return;
        };
        if t.parent().is_some() {
            self.floating.push(w);
        } else {
            let app = app_id(&t);
            let id = self.shared.change(|s| {
                let id = s.fresh_id(&app);
                s.desk.open(id.clone(), Home::Shelf);
                id
            });
            self.focus = Some(id.clone());
            self.windows.push((id, w));
        }
        self.relayout();
        t.send_configure();
    }

    pub fn spawn(&self, cmd: &str, dir: Option<&Path>) {
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(cmd)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env("NORDLYS_SOCKET", &self.ipc_path)
            .env("XDG_SESSION_TYPE", "wayland")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("SWAYSOCK")
            .stdin(Stdio::null())
            .process_group(0);
        if std::env::var_os("XDG_CURRENT_DESKTOP").is_none() || self.mode == Mode::Nested {
            c.env("XDG_CURRENT_DESKTOP", "Nordlys");
        }
        // X11 apps go to xwayland-satellite, never to a desktop around us.
        match &self.x_display {
            Some(d) => c.env("DISPLAY", d),
            None => c.env_remove("DISPLAY"),
        };
        if let Some(d) = dir.filter(|d| d.is_dir()) {
            c.current_dir(d);
        }
        match c.spawn() {
            // Reaped on a thread of its own, so it never lingers as a zombie.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(e) => eprintln!("nordlys-comp: cannot start {cmd:?}: {e}"),
        }
    }

    fn act(&mut self, a: Action) {
        let request = |comp: &mut Comp, r: Request| {
            if let (Response::Error { message }, _) = comp.shared.request(Ok(r)) {
                eprintln!("nordlys-comp: {message}");
            }
            comp.sync();
        };
        match a {
            Action::Ipc(r) => request(self, r),
            Action::CloseFocused => {
                let dialog = self
                    .floating
                    .iter()
                    .rev()
                    .find(|w| self.space.element_location(w).is_some());
                let w = dialog.or_else(|| self.window(self.focus.as_deref()?));
                if let Some(t) = w.and_then(|w| w.toplevel()) {
                    t.send_close();
                }
            }
            Action::MoveFocused(n) => {
                if let Some(id) = self.focus.clone() {
                    request(self, Request::MoveTo { n, id: Some(id) });
                }
            }
            Action::WorkspaceBy(d) => {
                let now = lock(&self.shared.state).desk.active() as i32;
                let n = (now + d).max(1) as u32;
                request(self, Request::Workspace { n });
            }
            Action::FocusMain | Action::FocusSide => {
                let st = lock(&self.shared.state);
                let s = st.desk.stage();
                let want = if a == Action::FocusMain {
                    s.main()
                } else {
                    s.side()
                };
                let want = want.cloned();
                drop(st);
                if want.is_some() {
                    self.focus = want;
                    self.relayout();
                }
            }
            Action::ScreenBy(d) => {
                if let Some(o) = self.screen_by(d) {
                    request(self, Request::FocusScreen { name: o.name() });
                }
            }
            Action::MoveToScreen(d) => {
                let target = self.screen_by(d).and_then(|o| {
                    let st = lock(&self.shared.state);
                    st.desk
                        .screens()
                        .find(|(n, _)| *n == o.name())
                        .map(|(_, w)| w)
                });
                if let (Some(n), Some(id)) = (target, self.focus.clone()) {
                    request(
                        self,
                        Request::MoveTo {
                            n,
                            id: Some(id.clone()),
                        },
                    );
                    request(self, Request::Bring { id });
                }
            }
            Action::Terminal => self.spawn(terminal(), None),
            Action::Launcher { commands } => {
                let bin = shell_bin();
                let q = if commands { " --query '>'" } else { "" };
                self.spawn(&format!("'{}' launcher{q}", bin.display()), None);
            }
            Action::Spawn(cmd) => self.spawn(cmd, None),
            Action::Vt(n) => {
                if let Backend::Tty(t) = &mut self.backend {
                    t.change_vt(n);
                }
            }
            Action::Quit => self.signal.stop(),
        }
    }

    /// The screen `d` places from the focused one, wrapping around; None
    /// with one screen.
    fn screen_by(&self, d: i32) -> Option<Output> {
        let n = self.outputs.len() as i32;
        if n < 2 {
            return None;
        }
        let at = self
            .focused_output()
            .and_then(|f| self.outputs.iter().position(|o| *o == f))
            .unwrap_or(0) as i32;
        self.outputs.get((at + d).rem_euclid(n) as usize).cloned()
    }

    // ---- Input ------------------------------------------------------

    fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.output_at(pos)?;
        let origin = self.output_origin(&output);
        if let Lock::Locked { surfaces, .. } = &self.lock {
            // Each lock surface covers its screen from the top-left corner.
            let (_, s) = surfaces.iter().find(|(o, _)| *o == output)?;
            return Some((s.wl_surface().clone(), origin.to_f64()));
        }
        let map = layer_map_for_output(&output);
        let local = pos - origin.to_f64();
        let in_layer = |layer: Layer| {
            let l = map.layer_under(layer, local)?;
            let at = map.layer_geometry(l)?.loc + origin;
            l.surface_under(pos - at.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + at).to_f64()))
        };
        in_layer(Layer::Overlay)
            .or_else(|| in_layer(Layer::Top))
            .or_else(|| {
                self.space.element_under(pos).and_then(|(w, at)| {
                    w.surface_under(pos - at.to_f64(), WindowSurfaceType::ALL)
                        .map(|(s, p)| (s, (p + at).to_f64()))
                })
            })
            .or_else(|| in_layer(Layer::Bottom))
            .or_else(|| in_layer(Layer::Background))
    }

    fn pointer_location(&self) -> Point<f64, Logical> {
        self.seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default()
    }

    /// The pointer moved to `pos`. It stays on the screens: past an edge
    /// with no screen beyond, it stops at the edge of the one it was on.
    /// Onto another screen, that screen gets the keyboard.
    fn pointer_to(&mut self, pos: Point<f64, Logical>, time: u32) {
        let pos = if self.output_at(pos).is_some() {
            pos
        } else {
            let from = self
                .output_at(self.pointer_location())
                .or_else(|| self.outputs.first().cloned());
            let g = from
                .and_then(|o| self.space.output_geometry(&o))
                .unwrap_or_default();
            Point::from((
                pos.x.clamp(
                    f64::from(g.loc.x),
                    f64::from(g.loc.x + g.size.w - 1).max(f64::from(g.loc.x)),
                ),
                pos.y.clamp(
                    f64::from(g.loc.y),
                    f64::from(g.loc.y + g.size.h - 1).max(f64::from(g.loc.y)),
                ),
            ))
        };
        if let Some(o) = self.output_at(pos)
            && self.focused_output().as_ref() != Some(&o)
        {
            let name = o.name();
            // Already where it's going, so the layout doesn't move it.
            self.shown_focus = Some(name.clone());
            self.shared.change(|s| s.desk.focus_screen(&name));
            self.move_pointer_at(pos, time);
            self.relayout();
            return;
        }
        self.move_pointer_at(pos, time);
    }

    /// Put the pointer at `pos` now (no focus follows), as a warp.
    fn move_pointer(&mut self, pos: Point<f64, Logical>) {
        let time = self.start.elapsed().as_millis() as u32;
        self.move_pointer_at(pos, time);
    }

    fn move_pointer_at(&mut self, pos: Point<f64, Logical>, time: u32) {
        let under = self.surface_under(pos);
        let pointer = self.seat.get_pointer().expect("seat has a pointer");
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
        if self.cursor.drawn {
            self.request_redraw();
        }
    }

    pub fn input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        if !matches!(
            event,
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }
        ) {
            self.idle_state.notify_activity(&self.seat);
        }
        match event {
            InputEvent::Keyboard { event } => {
                let code = event.key_code();
                let state = event.state();
                let key = self.nordlys_key;
                let locked = self.locked();
                let kb = self.seat.get_keyboard().expect("seat has a keyboard");
                let action = kb.input(
                    self,
                    code,
                    state,
                    SERIAL_COUNTER.next_serial(),
                    event.time_msec(),
                    |comp, mods, sym| {
                        if state == KeyState::Released {
                            if let Some(i) = comp.suppressed.iter().position(|c| *c == code) {
                                comp.suppressed.remove(i);
                                return FilterResult::Intercept(None);
                            }
                            return FilterResult::Forward;
                        }
                        let global = global_binding(sym.modified_sym().raw())
                            // Locked, only the VT switch gets past the lock screen.
                            .filter(|a| !locked || matches!(a, Action::Vt(_)));
                        let found = global.or_else(|| {
                            (!locked && key.held(mods))
                                .then(|| sym.raw_latin_sym_or_raw_current_sym())
                                .flatten()
                                .and_then(|s| binding(mods.shift, mods.ctrl, s.raw()))
                        });
                        match found {
                            Some(a) => {
                                comp.suppressed.push(code);
                                FilterResult::Intercept(Some(a))
                            }
                            None => FilterResult::Forward,
                        }
                    },
                );
                if let Some(Some(a)) = action {
                    self.act(a);
                }
            }
            InputEvent::PointerMotion { event } => {
                let pointer = self.seat.get_pointer().expect("seat has a pointer");
                let pos = pointer.current_location() + event.delta();
                self.pointer_to(pos, event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                // Absolute devices (tablets, a VM's pointer) span every screen.
                let b = self.bounds();
                let pos = event.position_transformed(b.size) + b.loc.to_f64();
                self.pointer_to(pos, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                let pointer = self.seat.get_pointer().expect("seat has a pointer");
                let serial = SERIAL_COUNTER.next_serial();
                if event.state() == ButtonState::Pressed && !self.locked() {
                    self.click_focus(pointer.current_location());
                }
                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state(),
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let amount = |axis| {
                    event
                        .amount(axis)
                        .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0)
                };
                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let a = amount(axis);
                    if a != 0.0 {
                        frame = frame.value(axis, a);
                        if let Some(v120) = event.amount_v120(axis) {
                            frame = frame.v120(axis, v120 as i32);
                        }
                    } else if source == AxisSource::Finger {
                        frame = frame.stop(axis);
                    }
                }
                let pointer = self.seat.get_pointer().expect("seat has a pointer");
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    /// A click gives the keyboard to what's under it: a stage window, or a
    /// layer surface that takes the keyboard on demand.
    fn click_focus(&mut self, pos: Point<f64, Logical>) {
        let layer = self.output_at(pos).and_then(|o| {
            let local = pos - self.output_origin(&o).to_f64();
            let map = layer_map_for_output(&o);
            [Layer::Overlay, Layer::Top]
                .into_iter()
                .find_map(|l| map.layer_under(l, local).cloned())
        });
        if let Some(l) = layer {
            if l.can_receive_keyboard_focus() {
                self.layer_focus = Some(l.wl_surface().clone());
                self.apply_focus();
            }
            return;
        }
        // A click on a window takes the keyboard back from an on-demand
        // layer surface, but never from an exclusive one.
        if self.layer_focus.as_ref().is_some_and(|s| !exclusive(s)) {
            self.layer_focus = None;
        }
        let id = self
            .space
            .element_under(pos)
            .and_then(|(w, _)| Some(w.toplevel()?.wl_surface().clone()))
            .and_then(|s| self.id_of(&s));
        if let Some(id) = id
            && self.focus.as_ref() != Some(&id)
        {
            self.focus = Some(id);
            self.relayout();
            return;
        }
        self.apply_focus();
    }

    // ---- Drawing ----------------------------------------------------

    /// The accent ring around the window with the keyboard, in two-up,
    /// as (x, y, w, h) rectangles in logical pixels.
    pub fn ring_rects(&self) -> Vec<(i32, i32, i32, i32)> {
        let two_up = lock(&self.shared.state).desk.stage().side().is_some();
        let geo = self
            .focus
            .as_deref()
            .filter(|_| two_up && self.layer_focus.is_none())
            .and_then(|id| self.window(id))
            .and_then(|w| self.space.element_geometry(w));
        let Some(g) = geo else { return Vec::new() };
        let (o, t) = (RING_OFFSET, RING);
        let (x, y, w, h) = (g.loc.x - o, g.loc.y - o, g.size.w + 2 * o, g.size.h + 2 * o);
        vec![
            (x, y, w, t),
            (x, y + h - t, w, t),
            (x, y, t, h),
            (x + w - t, y, t, h),
        ]
    }

    pub fn ground(&self) -> [f32; 4] {
        render::color(lock(&self.shared.state).config.palette().ground, 1.0)
    }

    pub fn accent(&self) -> Rgb {
        lock(&self.shared.state).config.palette().accent
    }

    /// A frame was drawn on `out`: tell clients there they may draw the
    /// next, and answer a lock request once every screen has the desktop
    /// off it.
    pub fn frame_done(&mut self, out: &Output) {
        self.last_frame = Instant::now();
        let now = self.start.elapsed();
        let screens = self.outputs.clone();
        if let Lock::Locked {
            pending,
            surfaces,
            drawn,
        } = &mut self.lock
        {
            if let Some((_, s)) = surfaces.iter().find(|(o, _)| o == out) {
                send_frames_surface(s.wl_surface(), out, now);
            }
            if !drawn.contains(out) {
                drawn.push(out.clone());
            }
            if screens.iter().all(|o| drawn.contains(o))
                && let Some(locker) = pending.take()
            {
                locker.lock();
            }
            // Clients behind the lock still get frames, slowly, so they
            // don't hang; nothing of theirs is drawn.
        }
        for w in self.space.elements_for_output(out) {
            w.send_frame(out, now, Some(Duration::ZERO), |_, _| Some(out.clone()));
        }
        for l in layer_map_for_output(out).layers() {
            l.send_frame(out, now, Some(Duration::ZERO), |_, _| Some(out.clone()));
        }
        if let CursorImageStatus::Surface(s) = &self.cursor.status
            && self.output_at(self.pointer_location()).as_ref() == Some(out)
        {
            send_frames_surface(s, out, now);
        }
    }

    /// How often windows waiting on the shelf may redraw (and their
    /// pictures for the shelf be taken): `stage.shelf_fps`.
    fn shelf_interval(&self) -> Duration {
        let fps = lock(&self.shared.state).config.stage.shelf_fps.max(1);
        Duration::from_millis(1000 / u64::from(fps))
    }

    /// Windows off the stage get a frame now and then, so they stay
    /// current, and the shelf gets a fresh picture of each.
    fn shelf_tick(&mut self) {
        let now = self.start.elapsed();
        let Some(out) = self.focused_output() else {
            return;
        };
        let waiting: Vec<(String, Window)> = self
            .windows
            .iter()
            .filter(|(_, w)| self.space.element_location(w).is_none())
            .cloned()
            .collect();
        for (_, w) in &waiting {
            w.send_frame(&out, now, None, |_, _| Some(out.clone()));
        }
        if self.locked() {
            return;
        }
        let ids: Vec<String> = self.windows.iter().map(|(id, _)| id.clone()).collect();
        self.previews.forget_except(&ids);
        self.with_renderer(|comp, r| {
            for (id, w) in &waiting {
                comp.previews.capture(r, id, w);
            }
        });
    }
}

fn send_frames_surface(s: &WlSurface, out: &Output, now: Duration) {
    smithay::desktop::utils::send_frames_surface_tree(s, out, now, Some(Duration::ZERO), |_, _| {
        Some(out.clone())
    });
}

fn exclusive(layer_surface: &WlSurface) -> bool {
    with_states(layer_surface, |s| {
        s.cached_state
            .get::<smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState>()
            .current()
            .keyboard_interactivity
            == KeyboardInteractivity::Exclusive
    })
}

fn configure_lock(surface: &LockSurface, output: &Output) {
    let size = output
        .current_mode()
        .map(|m| {
            m.size
                .to_f64()
                .to_logical(output.current_scale().fractional_scale())
                .to_i32_round::<i32>()
        })
        .unwrap_or_default();
    surface.with_pending_state(|s| {
        s.size = Some((size.w.max(1) as u32, size.h.max(1) as u32).into());
    });
    surface.send_configure();
}

// ---- Protocol handlers ------------------------------------------------

impl BufferHandler for Comp {
    fn buffer_destroyed(&mut self, _: &WlBuffer) {}
}

impl CompositorHandler for Comp {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("every client is inserted with ClientState")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            let is_root = |w: &&Window| w.toplevel().is_some_and(|t| t.wl_surface() == &root);
            if let Some(w) = self
                .windows
                .iter()
                .map(|(_, w)| w)
                .chain(&self.floating)
                .find(is_root)
            {
                w.on_commit();
            }
            if let Some(i) = self.unregistered.iter().position(|w| is_root(&w)) {
                let w = self.unregistered.remove(i);
                w.on_commit();
                self.register(w);
            }
        }

        // Popups (menus, tooltips) get their first configure here.
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(p)) = self.popups.find_popup(surface)
            && !p.is_initial_configure_sent()
        {
            let _ = p.send_configure();
        }

        self.commit_layer(surface);
        self.request_redraw();
    }
}

impl Comp {
    fn commit_layer(&mut self, surface: &WlSurface) {
        let Some(output) = self.outputs.iter().find(|o| {
            layer_map_for_output(o)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some()
        }) else {
            return;
        };
        let output = output.clone();
        let (first, wants_keys) = {
            let mut map = layer_map_for_output(&output);
            let Some(layer) = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .cloned()
            else {
                return;
            };
            let first = !with_states(surface, |s| {
                s.data_map
                    .get::<LayerSurfaceData>()
                    .is_some_and(|d| d.lock().is_ok_and(|d| d.initial_configure_sent))
            });
            map.arrange();
            if first {
                layer.layer_surface().send_configure();
            }
            let wants = matches!(layer.layer(), Layer::Top | Layer::Overlay)
                && layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive;
            (first, wants)
        };
        let has = self.layer_focus.as_ref() == Some(surface);
        if wants_keys && !has {
            self.layer_focus = Some(surface.clone());
        } else if !wants_keys && has && exclusive(surface) {
            self.layer_focus = None;
        }
        // What the layer reserves may have changed; so may the keyboard.
        if first || wants_keys != has || self.stage_areas() != self.areas {
            self.relayout();
        }
    }
}

impl XdgShellHandler for Comp {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.unregistered.push(Window::new_wayland_window(surface));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let s = surface.wl_surface();
        let mine = |w: &Window| w.toplevel().is_some_and(|t| t.wl_surface() == s);
        self.unregistered.retain(|w| !mine(w));
        if let Some(i) = self.floating.iter().position(&mine) {
            let w = self.floating.remove(i);
            self.space.unmap_elem(&w);
        }
        if let Some(i) = self.windows.iter().position(|(_, w)| mine(w)) {
            let (id, w) = self.windows.remove(i);
            self.space.unmap_elem(&w);
            self.previews.forget(&id);
            self.shared.change(|st| st.desk.close(&id));
            if self.focus.as_ref() == Some(&id) {
                self.focus = None;
            }
        }
        self.relayout();
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        surface.with_pending_state(|s| s.geometry = positioner.get_geometry());
        if let Err(e) = self.popups.track_popup(PopupKind::from(surface)) {
            eprintln!("nordlys-comp: popup: {e}");
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|s| {
            s.geometry = positioner.get_geometry();
            s.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn grab(&mut self, _: PopupSurface, _: WlSeat, _: Serial) {}

    // The Stage decides where windows go; moving and resizing by hand
    // (and maximizing) don't apply.
    fn move_request(&mut self, _: ToplevelSurface, _: WlSeat, _: Serial) {}
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
    }

    fn title_changed(&mut self, _: ToplevelSurface) {}
}

impl XdgDecorationHandler for Comp {
    // Nordlys draws no title bars, and asks apps not to draw their own.
    fn new_decoration(&mut self, t: ToplevelSurface) {
        t.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
    }

    fn request_mode(&mut self, t: ToplevelSurface, _: DecorationMode) {
        self.new_decoration(t.clone());
        if t.is_initial_configure_sent() {
            t.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, t: ToplevelSurface) {
        self.request_mode(t, DecorationMode::ServerSide);
    }
}

impl WlrLayerShellHandler for Comp {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        // On the screen it asked for, else the one with the keyboard (where
        // the launcher should open).
        let Some(output) = output
            .as_ref()
            .and_then(Output::from_resource)
            .filter(|o| self.outputs.contains(o))
            .or_else(|| self.focused_output())
        else {
            surface.send_close();
            return;
        };
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            eprintln!("nordlys-comp: layer surface: {e}");
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        if let Err(e) = self.popups.track_popup(PopupKind::from(popup)) {
            eprintln!("nordlys-comp: popup: {e}");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        for o in &self.outputs {
            let mut map = layer_map_for_output(o);
            let layer = map
                .layers()
                .find(|l| l.layer_surface() == &surface)
                .cloned();
            if let Some(l) = layer {
                map.unmap_layer(&l);
            }
        }
        self.blur.forget(surface.wl_surface());
        if self.layer_focus.as_ref() == Some(surface.wl_surface()) {
            self.layer_focus = None;
        }
        self.relayout();
    }
}

impl SessionLockHandler for Comp {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        // Taken off any layer surface, so the launcher can't keep keys.
        self.layer_focus = None;
        self.lock = Lock::Locked {
            pending: Some(confirmation),
            surfaces: Vec::new(),
            drawn: Vec::new(),
        };
        self.apply_focus();
        self.request_redraw();
    }

    fn unlock(&mut self) {
        self.lock = Lock::Unlocked;
        self.relayout();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output).filter(|o| self.outputs.contains(o))
        else {
            return;
        };
        configure_lock(&surface, &output);
        if let Lock::Locked { surfaces, .. } = &mut self.lock {
            surfaces.retain(|(o, _)| *o != output);
            surfaces.push((output, surface));
        }
        self.apply_focus();
        self.request_redraw();
    }
}

impl IdleNotifierHandler for Comp {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_state
    }
}

impl IdleInhibitHandler for Comp {
    fn inhibit(&mut self, surface: WlSurface) {
        self.inhibitors.push(surface);
        self.idle_state.set_is_inhibited(true);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.inhibitors.retain(|s| s != &surface && s.is_alive());
        self.idle_state
            .set_is_inhibited(!self.inhibitors.is_empty());
    }
}

impl ShmHandler for Comp {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for Comp {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        let mut ok = false;
        self.with_renderer(|_, r| ok = r.import_dmabuf(&dmabuf, None).is_ok());
        if ok {
            let _ = notifier.successful::<Comp>();
        } else {
            notifier.failed();
        }
    }
}

impl SeatHandler for Comp {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| self.dh.get_client(s.id()).ok());
        set_data_device_focus(&self.dh, seat, client.clone());
        set_primary_focus(&self.dh, seat, client);
    }

    fn cursor_image(&mut self, _: &Seat<Self>, image: CursorImageStatus) {
        self.cursor.status = image;
        if self.cursor.drawn {
            self.request_redraw();
        }
    }
}

impl SelectionHandler for Comp {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Comp {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl PrimarySelectionHandler for Comp {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl ClientDndGrabHandler for Comp {}
impl ServerDndGrabHandler for Comp {}
impl OutputHandler for Comp {}

delegate_compositor!(Comp);
delegate_xdg_shell!(Comp);
delegate_xdg_decoration!(Comp);
delegate_layer_shell!(Comp);
delegate_shm!(Comp);
delegate_dmabuf!(Comp);
delegate_seat!(Comp);
delegate_data_device!(Comp);
delegate_primary_selection!(Comp);
delegate_output!(Comp);
delegate_viewporter!(Comp);
delegate_session_lock!(Comp);
delegate_idle_notify!(Comp);
delegate_idle_inhibit!(Comp);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_read_shift_from_the_modifier_not_the_symbol() {
        assert_eq!(
            binding(false, false, keysyms::KEY_3),
            Some(Action::Ipc(Request::Workspace { n: 3 }))
        );
        assert_eq!(
            binding(true, false, keysyms::KEY_3),
            Some(Action::MoveFocused(3))
        );
        assert_eq!(
            binding(false, false, keysyms::KEY_k),
            Some(Action::Ipc(Request::Previous))
        );
        assert_eq!(
            binding(true, false, keysyms::KEY_Escape),
            Some(Action::Quit)
        );
        assert_eq!(
            binding(false, false, keysyms::KEY_Escape),
            Some(Action::Spawn(LOCK))
        );
        assert_eq!(binding(false, false, keysyms::KEY_z), None);
    }

    #[test]
    fn ctrl_arrows_act_on_screens_and_ctrl_takes_nothing_else() {
        assert_eq!(
            binding(false, true, keysyms::KEY_Right),
            Some(Action::ScreenBy(1))
        );
        assert_eq!(
            binding(false, true, keysyms::KEY_Left),
            Some(Action::ScreenBy(-1))
        );
        assert_eq!(
            binding(true, true, keysyms::KEY_Right),
            Some(Action::MoveToScreen(1))
        );
        assert_eq!(
            binding(false, false, keysyms::KEY_Left),
            Some(Action::FocusMain)
        );
        assert_eq!(binding(false, true, keysyms::KEY_3), None);
    }

    #[test]
    fn vt_keys_and_media_keys_need_no_nordlys_key() {
        assert_eq!(
            global_binding(keysyms::KEY_XF86Switch_VT_1),
            Some(Action::Vt(1))
        );
        assert_eq!(
            global_binding(keysyms::KEY_XF86Switch_VT_12),
            Some(Action::Vt(12))
        );
        assert!(matches!(
            global_binding(keysyms::KEY_XF86AudioRaiseVolume),
            Some(Action::Spawn(_))
        ));
        assert_eq!(global_binding(keysyms::KEY_a), None);
    }
}
