//! The screens themselves: DRM/KMS for the picture, libinput for keyboards
//! and pointers, and libseat for the right to use them, so it runs from a
//! TTY or a display manager without being root (logind or seatd hand over
//! the devices).
//!
//! Every GPU with screen connectors, and every screen connected to them,
//! each in its preferred mode on a CRTC of its own, laid out left to right:
//! a laptop's own panel first, then the others by connector name. One GPU,
//! the main one (the one the firmware booted with, or `NORDLYS_GPU`), draws
//! everything and is the one clients get their buffers from. A screen on
//! another GPU (on many laptops the HDMI port is wired to the discrete GPU)
//! gets each frame copied over: the main GPU draws it into a linear buffer
//! that the screen's GPU reads directly, or, if it can't, through memory.
//! That GPU starts a renderer only while one of its screens is lit, so it
//! can sleep otherwise.
//!
//! Screens and GPUs plugged in or out while it runs are picked up from
//! udev. Each screen's frames are drawn on demand and paced by its own
//! vblank; switching to another VT (Ctrl+Alt+F1…) pauses everything and
//! coming back redraws from scratch.

use crate::comp::{Backend, Comp, Ready, libc_dev_t};
use crate::render;
use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Buffer as _, Format, Fourcc, Modifier, Swapchain};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEvent, DrmNode, NodeType,
};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::DamageBag;
use smithay::backend::renderer::{Bind, ExportMem, ImportDma, ImportMem, Renderer, TextureMapping};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu};
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::{EventLoop, LoopHandle, RegistrationToken};
use smithay::reexports::drm::control::{Device as _, ModeTypeFlags, connector, crtc};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{Buffer as BufferCoords, DeviceFd, Rectangle, Size, Transform};
use std::error::Error;
use std::path::{Path, PathBuf};

type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

pub struct Tty {
    session: LibSeatSession,
    libinput: Libinput,
    /// The main GPU's renderer: everything is drawn with it.
    renderer: GlesRenderer,
    /// The GPUs with screen connectors; the main one first.
    gpus: Vec<Gpu>,
    /// What the main GPU can import from clients.
    formats: Vec<Format>,
    screens: Vec<Screen>,
    /// The session has the devices (not switched away to another VT).
    active: bool,
    /// A render is already queued for when the loop is idle.
    queued: bool,
}

/// One GPU and the screens it can drive.
struct Gpu {
    /// Its device number, to know its udev events and its screens.
    dev: libc_dev_t,
    path: PathBuf,
    drm: DrmDevice,
    gbm: GbmDevice<DrmDeviceFd>,
    render_node: Option<DrmNode>,
    /// On a GPU other than the main one, the renderer that puts the frames
    /// copied in on its screens; only while it has one.
    copier: Option<GlesRenderer>,
    /// Its vblank events, to stop listening when it's unplugged.
    token: Option<RegistrationToken>,
}

/// One connected screen and what drives it.
struct Screen {
    output: Output,
    /// The GPU it's plugged into.
    gpu: libc_dev_t,
    connector: connector::Handle,
    crtc: crtc::Handle,
    /// A laptop's own panel: it goes first.
    panel: bool,
    compositor: Compositor,
    /// On a GPU other than the main one: how its frames get there.
    copy: Option<Copy>,
    /// A frame is on its way to the screen; the next waits for vblank.
    pending: bool,
    /// Something changed while a frame was pending.
    again: bool,
}

/// A screen's frames drawn on the main GPU, for another GPU to show.
struct Copy {
    /// Linear buffers on the main GPU the frames are drawn into.
    swapchain: Swapchain<GbmAllocator<DrmDeviceFd>>,
    damage: OutputDamageTracker,
    /// What changed from frame to frame, for the screen's own GPU.
    bag: DamageBag<i32, BufferCoords>,
    id: Id,
    /// The screen's GPU couldn't read the main GPU's buffers, so frames go
    /// through memory (slower, but works between any two GPUs).
    cpu: bool,
}

impl Copy {
    fn new(main: &GbmDevice<DrmDeviceFd>, output: &Output, (w, h): (u16, u16)) -> Self {
        Self {
            swapchain: Swapchain::new(
                GbmAllocator::new(main.clone(), GbmBufferFlags::RENDERING),
                u32::from(w),
                u32::from(h),
                Fourcc::Xrgb8888,
                vec![Modifier::Linear],
            ),
            damage: OutputDamageTracker::from_output(output),
            bag: DamageBag::new(4),
            id: Id::new(),
            cpu: false,
        }
    }

    /// Forget every buffer and frame, after the GPUs were away.
    fn reset(&mut self, output: &Output) {
        self.swapchain.reset_buffers();
        self.damage = OutputDamageTracker::from_output(output);
        self.id = Id::new();
    }
}

impl Tty {
    pub fn renderer(&mut self) -> Option<&mut GlesRenderer> {
        self.active.then_some(&mut self.renderer)
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            eprintln!("nordlys-comp: cannot switch to VT {vt}: {e}");
        }
    }

    fn outputs(&self) -> Vec<Output> {
        self.screens.iter().map(|s| s.output.clone()).collect()
    }

    /// Match the screens to what's connected on every GPU, and lay them
    /// out again. True if any came or went. `probe` asks the connectors
    /// afresh (slower; for hotplug, when the kernel's idea may be stale).
    fn scan(&mut self, probe: bool) -> bool {
        let mut changed = false;
        for i in 0..self.gpus.len() {
            match self.scan_gpu(i, probe) {
                Ok(c) => changed |= c,
                Err(e) => eprintln!("nordlys-comp: {e}"),
            }
        }
        // A GPU without lit screens lets its renderer go.
        for g in self.gpus.iter_mut().skip(1) {
            if g.copier.is_some() && !self.screens.iter().any(|s| s.gpu == g.dev) {
                g.copier = None;
            }
        }
        self.screens.sort_by_key(|s| (!s.panel, s.output.name()));
        self.place();
        changed
    }

    /// `scan` for the GPU at `i`.
    fn scan_gpu(&mut self, i: usize, probe: bool) -> Result<bool, String> {
        let gpu = &self.gpus[i];
        let dev = gpu.dev;
        let res = gpu
            .drm
            .resource_handles()
            .map_err(|e| format!("cannot read the screens of {}: {e}", gpu.path.display()))?;
        let connected: Vec<connector::Info> = res
            .connectors()
            .iter()
            .filter_map(|c| gpu.drm.get_connector(*c, probe).ok())
            .filter(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
            .collect();

        let before = self.screens.len();
        self.screens.retain(|s| {
            let keep = s.gpu != dev || connected.iter().any(|c| c.handle() == s.connector);
            if !keep {
                eprintln!("nordlys-comp: {} unplugged", s.output.name());
            }
            keep
        });
        let mut changed = self.screens.len() != before;

        for conn in &connected {
            if self
                .screens
                .iter()
                .any(|s| s.gpu == dev && s.connector == conn.handle())
            {
                continue;
            }
            let used: Vec<crtc::Handle> = self
                .screens
                .iter()
                .filter(|s| s.gpu == dev)
                .map(|s| s.crtc)
                .collect();
            let drm = &self.gpus[i].drm;
            let crtc = conn
                .encoders()
                .iter()
                .filter_map(|e| drm.get_encoder(*e).ok())
                .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
                .find(|c| !used.contains(c));
            let name = connector_name(conn);
            let Some(crtc) = crtc else {
                eprintln!("nordlys-comp: no display controller left for {name}; it stays dark");
                continue;
            };
            match self.drive(i, conn, crtc) {
                Ok(s) => {
                    self.screens.push(s);
                    changed = true;
                }
                Err(e) => eprintln!("nordlys-comp: cannot drive {name}: {e}"),
            }
        }
        Ok(changed)
    }

    /// Left to right, tops aligned, in logical pixels.
    fn place(&self) {
        let mut x = 0;
        for s in &self.screens {
            s.output
                .change_current_state(None, None, None, Some((x, 0).into()));
            x += logical_width(&s.output);
        }
    }

    /// A screen on `crtc` of the GPU at `i`, in the connector's preferred
    /// mode.
    fn drive(
        &mut self,
        i: usize,
        conn: &connector::Info,
        crtc: crtc::Handle,
    ) -> Result<Screen, String> {
        let drm_mode = conn
            .modes()
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .copied()
            .unwrap_or(conn.modes()[0]);
        let name = connector_name(conn);
        let (mm_w, mm_h) = conn.size().unwrap_or((0, 0));

        let main = i == 0;
        if !main && self.gpus[i].copier.is_none() {
            self.gpus[i].copier = Some(
                copier(&self.gpus[i].gbm)
                    .map_err(|e| format!("{} can't draw: {e}", self.gpus[i].path.display()))?,
            );
        }
        let surface = self.gpus[i]
            .drm
            .create_surface(crtc, drm_mode, &[conn.handle()])
            .map_err(|e| e.to_string())?;
        let gpu = &self.gpus[i];

        let (w, h) = drm_mode.size();
        let mode = Mode {
            size: (i32::from(w), i32::from(h)).into(),
            refresh: (drm_mode.vrefresh() * 1000) as i32,
        };
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (mm_w as i32, mm_h as i32).into(),
                subpixel: Subpixel::Unknown,
                make: "Unknown".into(),
                model: name.clone(),
            },
        );
        let scale = screen_scale(i32::from(w), mm_w);
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(scale)),
            Some((0, 0).into()),
        );
        output.set_preferred(mode);

        // What the renderer drawing into this screen's buffers can draw to.
        let renderer_formats: Vec<Format> = match &gpu.copier {
            Some(c) => c
                .egl_context()
                .dmabuf_render_formats()
                .iter()
                .copied()
                .collect(),
            None => self.formats.clone(),
        };
        let allocator = GbmAllocator::new(
            gpu.gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        let exporter = GbmFramebufferExporter::new(gpu.gbm.clone(), gpu.render_node);
        let compositor = DrmCompositor::new(
            &output,
            surface,
            None,
            allocator,
            exporter,
            [Fourcc::Xrgb8888, Fourcc::Argb8888],
            renderer_formats,
            gpu.drm.cursor_size(),
            Some(gpu.gbm.clone()),
        )
        .map_err(|e| format!("{e:?}"))?;
        let copy = (!main).then(|| Copy::new(&self.gpus[0].gbm, &output, (w, h)));
        eprintln!(
            "nordlys-comp: {name} at {w}×{h}@{}Hz, scale {scale}{}",
            drm_mode.vrefresh(),
            if main {
                String::new()
            } else {
                format!(", on {}", gpu.path.display())
            },
        );
        Ok(Screen {
            output,
            gpu: gpu.dev,
            connector: conn.handle(),
            crtc,
            panel: matches!(
                conn.interface(),
                connector::Interface::EmbeddedDisplayPort | connector::Interface::LVDS
            ),
            compositor,
            copy,
            pending: false,
            again: false,
        })
    }
}

/// Open the GPU at `path` and listen to its vblanks. None if it has no
/// screen connectors (a render-only GPU), so it isn't kept awake.
fn open_gpu(
    session: &mut LibSeatSession,
    handle: &LoopHandle<'static, Comp>,
    path: &Path,
) -> Result<Option<Gpu>, Box<dyn Error>> {
    let fd = session
        .open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
    if drm.resource_handles()?.connectors().is_empty() {
        return Ok(None);
    }
    let gbm = GbmDevice::new(fd.clone())?;
    let node = DrmNode::from_file(&fd)?;
    let render_node = node
        .node_with_type(NodeType::Render)
        .and_then(Result::ok)
        .or(Some(node));
    let mut gpu = Gpu {
        dev: node.dev_id(),
        path: path.to_owned(),
        drm,
        gbm,
        render_node,
        copier: None,
        token: None,
    };
    gpu.token = Some(listen(handle, gpu.dev, notifier)?);
    Ok(Some(gpu))
}

fn listen(
    handle: &LoopHandle<'static, Comp>,
    dev: libc_dev_t,
    notifier: DrmDeviceNotifier,
) -> Result<RegistrationToken, Box<dyn Error>> {
    let token = handle.insert_source(notifier, move |event, _, comp: &mut Comp| match event {
        DrmEvent::VBlank(crtc) => vblank(comp, dev, crtc),
        DrmEvent::Error(e) => eprintln!("nordlys-comp: DRM: {e}"),
    })?;
    Ok(token)
}

/// A renderer on `gbm`'s GPU.
fn copier(gbm: &GbmDevice<DrmDeviceFd>) -> Result<GlesRenderer, Box<dyn Error>> {
    // SAFETY: the GBM device lives as long as the display made from it.
    let egl = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&egl)?;
    // SAFETY: the context is current only on this thread.
    Ok(unsafe { GlesRenderer::new(context)? })
}

/// The same GPU, however its path is spelled.
fn same_path(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    real(a) == real(b)
}

pub fn init(
    event_loop: &EventLoop<'static, Comp>,
    _dh: &DisplayHandle,
) -> Result<Ready, Box<dyn std::error::Error>> {
    let (mut session, notifier) = LibSeatSession::new().map_err(|e| {
        format!(
            "no seat to run on ({e}). Run it from a TTY or a display manager, with \
             logind or seatd"
        )
    })?;
    let seat = session.seat();
    let handle = event_loop.handle();

    let all = all_gpus(&seat).unwrap_or_default();
    let main: PathBuf = std::env::var_os("NORDLYS_GPU")
        .map(PathBuf::from)
        .or_else(|| primary_gpu(&seat).ok().flatten())
        .or_else(|| all.first().cloned())
        .ok_or("no GPU found")?;
    let main_gpu = open_gpu(&mut session, &handle, &main)?
        .ok_or_else(|| format!("{} has no screen connectors", main.display()))?;
    let renderer = copier(&main_gpu.gbm)?;
    let formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();
    let device = main_gpu.render_node.map(|n| n.dev_id());

    let mut gpus = vec![main_gpu];
    for path in all.iter().filter(|p| !same_path(p, &main)) {
        match open_gpu(&mut session, &handle, path) {
            Ok(Some(g)) => gpus.push(g),
            Ok(None) => {}
            Err(e) => eprintln!("nordlys-comp: leaving {} out: {e}", path.display()),
        }
    }

    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput
        .udev_assign_seat(&seat)
        .map_err(|()| "libinput couldn't take the seat's input devices")?;

    let mut tty = Tty {
        session,
        libinput: libinput.clone(),
        renderer,
        gpus,
        formats: formats.clone(),
        screens: Vec::new(),
        active: true,
        queued: false,
    };
    tty.scan(false);
    if tty.screens.is_empty() {
        return Err("no connected screen".into());
    }
    let n = tty.screens.len();
    eprintln!(
        "nordlys-comp: {n} screen{}, drawn on {}",
        if n == 1 { "" } else { "s" },
        main.display()
    );

    handle.insert_source(
        LibinputInputBackend::new(libinput),
        |mut event, _, comp: &mut Comp| {
            if let InputEvent::DeviceAdded { device } = &mut event {
                configure_device(device);
            }
            comp.input(event);
        },
    )?;
    handle.insert_source(notifier, |event, _, comp: &mut Comp| {
        session_event(comp, event)
    })?;
    // Screens plugged in or out show up as a change to their GPU.
    match UdevBackend::new(&seat) {
        Ok(udev) => {
            handle.insert_source(udev, |event, _, comp: &mut Comp| hotplug(comp, event))?;
        }
        Err(e) => eprintln!("nordlys-comp: no hotplug, udev isn't there: {e}"),
    }

    let outputs = tty.outputs();
    Ok(Ready {
        backend: Backend::Tty(Box::new(tty)),
        outputs,
        formats,
        device,
    })
}

fn connector_name(conn: &connector::Info) -> String {
    format!(
        "{}-{}",
        interface_name(conn.interface()),
        conn.interface_id()
    )
}

fn interface_name(i: connector::Interface) -> &'static str {
    use connector::Interface::*;
    match i {
        DVII | DVID | DVIA => "DVI",
        VGA => "VGA",
        LVDS => "LVDS",
        DisplayPort => "DP",
        HDMIA | HDMIB => "HDMI-A",
        EmbeddedDisplayPort => "eDP",
        Virtual => "Virtual",
        DSI => "DSI",
        _ => "Unknown",
    }
}

/// How wide `output` is in logical pixels, at its scale.
fn logical_width(output: &Output) -> i32 {
    output
        .current_mode()
        .map(|m| {
            m.size
                .to_f64()
                .to_logical(output.current_scale().fractional_scale())
                .to_i32_round::<i32>()
                .w
        })
        .unwrap_or(0)
}

/// 2 on dense screens (about 190 dpi and up), else 1; `NORDLYS_SCALE`
/// overrides it for every screen.
fn screen_scale(width_px: i32, width_mm: u32) -> i32 {
    if let Some(s) = std::env::var("NORDLYS_SCALE")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|s| (1..=4).contains(s))
    {
        return s;
    }
    if width_mm == 0 {
        return 1;
    }
    let dpi = f64::from(width_px) / (f64::from(width_mm) / 25.4);
    if dpi >= 190.0 { 2 } else { 1 }
}

/// Touchpads tap to click and scroll naturally, as on the Hyprland session.
fn configure_device(device: &mut smithay::reexports::input::Device) {
    if device.config_tap_finger_count() > 0 {
        let _ = device.config_tap_set_enabled(true);
        let _ = device.config_scroll_set_natural_scroll_enabled(true);
        let _ = device.config_dwt_set_enabled(true);
    }
}

/// Look at the connectors again, and tell the compositor what changed.
fn rescan(comp: &mut Comp, probe: bool) {
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    if t.scan(probe) {
        let outputs = t.outputs();
        comp.set_outputs(outputs);
    }
}

fn hotplug(comp: &mut Comp, event: UdevEvent) {
    let handle = comp.handle.clone();
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    if !t.active {
        return;
    }
    match event {
        UdevEvent::Changed { device_id } => {
            if t.gpus.iter().any(|g| g.dev == device_id) {
                rescan(comp, true);
            }
        }
        // A GPU plugged in (a dock, an external GPU).
        UdevEvent::Added { device_id, path } => {
            if t.gpus.iter().any(|g| g.dev == device_id) {
                return;
            }
            match open_gpu(&mut t.session, &handle, &path) {
                Ok(Some(g)) => {
                    eprintln!("nordlys-comp: {} plugged in", path.display());
                    t.gpus.push(g);
                    rescan(comp, true);
                }
                Ok(None) => {}
                Err(e) => eprintln!("nordlys-comp: leaving {} out: {e}", path.display()),
            }
        }
        UdevEvent::Removed { device_id } => {
            // The main GPU can't go: everything is drawn with it.
            let Some(i) = t.gpus.iter().skip(1).position(|g| g.dev == device_id) else {
                return;
            };
            let gpu = t.gpus.remove(i + 1);
            eprintln!("nordlys-comp: {} unplugged", gpu.path.display());
            t.screens.retain(|s| s.gpu != device_id);
            if let Some(token) = gpu.token {
                handle.remove(token);
            }
            let outputs = t.outputs();
            comp.set_outputs(outputs);
        }
    }
}

fn session_event(comp: &mut Comp, event: SessionEvent) {
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    match event {
        SessionEvent::PauseSession => {
            t.active = false;
            t.libinput.suspend();
            for g in &mut t.gpus {
                g.drm.pause();
            }
            for s in &mut t.screens {
                s.pending = false;
            }
        }
        SessionEvent::ActivateSession => {
            if let Err(()) = t.libinput.resume() {
                eprintln!("nordlys-comp: libinput didn't come back");
            }
            for g in &mut t.gpus {
                if let Err(e) = g.drm.activate(false) {
                    eprintln!(
                        "nordlys-comp: the screens on {} didn't come back: {e}",
                        g.path.display()
                    );
                }
            }
            for s in &mut t.screens {
                if let Err(e) = s.compositor.reset_state() {
                    eprintln!("nordlys-comp: {} state: {e}", s.output.name());
                }
                s.compositor.reset_buffers();
                if let Some(c) = &mut s.copy {
                    c.reset(&s.output);
                }
                s.pending = false;
            }
            t.active = true;
            // Screens may have come or gone while we were away.
            rescan(comp, true);
            comp.request_redraw();
        }
    }
}

fn vblank(comp: &mut Comp, dev: libc_dev_t, crtc: crtc::Handle) {
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    let Some(s) = t
        .screens
        .iter_mut()
        .find(|s| s.gpu == dev && s.crtc == crtc)
    else {
        return;
    };
    if let Err(e) = s.compositor.frame_submitted() {
        eprintln!("nordlys-comp: frame on {}: {e:?}", s.output.name());
    }
    s.pending = false;
    if std::mem::take(&mut s.again) {
        draw(comp, Some((dev, crtc)));
    }
}

/// Draw when the loop is next idle; screens with a frame on its way draw
/// after it.
pub fn schedule(comp: &mut Comp) {
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    let mut free = false;
    for s in &mut t.screens {
        if s.pending {
            s.again = true;
        } else {
            free = true;
        }
    }
    if !free || t.queued {
        return;
    }
    t.queued = true;
    comp.handle.insert_idle(|comp| {
        if let Backend::Tty(t) = &mut comp.backend {
            t.queued = false;
        }
        render(comp);
    });
}

/// Draw every screen that can take a frame now.
pub fn render(comp: &mut Comp) {
    draw(comp, None);
}

/// Draw the screens that can take a frame, or just the one on `only` (a
/// GPU and its CRTC).
fn draw(comp: &mut Comp, only: Option<(libc_dev_t, crtc::Handle)>) {
    let Backend::Tty(mut t) = std::mem::replace(&mut comp.backend, Backend::Taken) else {
        return;
    };
    if !t.active {
        comp.backend = Backend::Tty(t);
        return;
    }
    comp.cursor.drawn = true;
    let mut drawn = Vec::new();
    let Tty {
        screens,
        renderer,
        gpus,
        ..
    } = &mut *t;
    for s in screens.iter_mut() {
        if only.is_some_and(|k| k != (s.gpu, s.crtc)) {
            continue;
        }
        if s.pending {
            s.again = true;
            continue;
        }
        let elements = render::elements(comp, renderer, &s.output, true);
        let ground = comp.ground();
        let shown = match &mut s.copy {
            None => s
                .compositor
                .render_frame(renderer, &elements, ground, FrameFlags::DEFAULT)
                .map(|frame| !frame.is_empty)
                .map_err(|e| format!("{e:?}")),
            Some(copy) => match gpus
                .iter_mut()
                .find(|g| g.dev == s.gpu)
                .and_then(|g| g.copier.as_mut())
            {
                Some(copier) => copy_frame(
                    renderer,
                    copier,
                    copy,
                    &mut s.compositor,
                    &s.output,
                    &elements,
                    ground,
                ),
                None => Err("its GPU has no renderer".into()),
            },
        };
        match shown {
            Ok(true) => match s.compositor.queue_frame(()) {
                Ok(()) => s.pending = true,
                Err(e) => eprintln!(
                    "nordlys-comp: cannot show the frame on {}: {e:?}",
                    s.output.name()
                ),
            },
            Ok(false) => {}
            Err(e) => eprintln!("nordlys-comp: render on {} failed: {e}", s.output.name()),
        }
        render::shots(comp, renderer, &s.output, &elements);
        drawn.push(s.output.clone());
    }
    comp.backend = Backend::Tty(t);
    for o in &drawn {
        comp.frame_done(o);
    }
}

/// Draw `elements` with the main GPU into one of `copy`'s buffers, then
/// put that on the screen with its own GPU's `copier`. True if there's a
/// new frame to queue.
fn copy_frame(
    main: &mut GlesRenderer,
    copier: &mut GlesRenderer,
    copy: &mut Copy,
    compositor: &mut Compositor,
    output: &Output,
    elements: &[render::Elem],
    ground: [f32; 4],
) -> Result<bool, String> {
    let slot = copy
        .swapchain
        .acquire()
        .map_err(|e| format!("no buffer on the main GPU: {e}"))?
        .ok_or("every buffer on the main GPU is in use")?;
    let dmabuf = match slot.userdata().get::<Dmabuf>() {
        Some(d) => d.clone(),
        None => {
            let d = slot.export().map_err(|e| e.to_string())?;
            slot.userdata().insert_if_missing_threadsafe(|| d.clone());
            d
        }
    };
    let size = dmabuf.size();
    let mut bound = dmabuf.clone();
    let mut target = main.bind(&mut bound).map_err(|e| e.to_string())?;
    let result = copy
        .damage
        .render_output(main, &mut target, usize::from(slot.age()), elements, ground)
        .map_err(|e| format!("{e:?}"))?;
    let Some(damage) = result.damage.filter(|d| !d.is_empty()).cloned() else {
        return Ok(false);
    };
    let sync = result.sync;
    main.wait(&sync).map_err(|e| e.to_string())?;

    let mut texture = None;
    if !copy.cpu {
        match copier.import_dmabuf(&dmabuf, None) {
            Ok(t) => texture = Some(t),
            Err(e) => {
                eprintln!(
                    "nordlys-comp: {} can't read the main GPU's buffers ({e}); copying through memory",
                    output.name()
                );
                copy.cpu = true;
            }
        }
    }
    let texture: GlesTexture = match texture {
        Some(t) => t,
        None => {
            let mapping = main
                .copy_framebuffer(&target, Rectangle::from_size(size), Fourcc::Abgr8888)
                .map_err(|e| e.to_string())?;
            let pixels = main.map_texture(&mapping).map_err(|e| e.to_string())?;
            copier
                .import_memory(pixels, Fourcc::Abgr8888, size, mapping.flipped())
                .map_err(|e| e.to_string())?
        }
    };
    drop(target);
    copy.swapchain.submitted(&slot);

    copy.bag.add(damage.iter().map(|r| {
        Rectangle::<i32, BufferCoords>::new(
            (r.loc.x, r.loc.y).into(),
            Size::from((r.size.w, r.size.h)),
        )
    }));
    let element = TextureRenderElement::from_texture_with_damage(
        copy.id.clone(),
        copier.context_id(),
        (0.0, 0.0),
        texture,
        output.current_scale().integer_scale(),
        Transform::Normal,
        None,
        None,
        None,
        Some(vec![Rectangle::from_size(size)]),
        copy.bag.snapshot(),
        Kind::Unspecified,
    );
    let elements = [element];
    let frame = compositor
        .render_frame(copier, &elements, ground, FrameFlags::DEFAULT)
        .map_err(|e| format!("{e:?}"))?;
    Ok(!frame.is_empty)
}
