//! The screens themselves: DRM/KMS for the picture, libinput for keyboards
//! and pointers, and libseat for the right to use them, so it runs from a
//! TTY or a display manager without being root (logind or seatd hand over
//! the devices).
//!
//! One GPU (the primary one) and every screen connected to it, each in its
//! preferred mode on a CRTC of its own, laid out left to right: a laptop's
//! own panel first, then the others by connector name. Screens plugged in
//! or out while it runs are picked up from udev. Each screen's frames are
//! drawn on demand and paced by its own vblank; switching to another VT
//! (Ctrl+Alt+F1…) pauses everything and coming back redraws from scratch.

use crate::comp::{Backend, Comp, Ready, libc_dev_t};
use crate::render;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Format, Fourcc};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu};
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::{Device as _, ModeTypeFlags, connector, crtc};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{DeviceFd, Transform};
use std::path::PathBuf;

type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

pub struct Tty {
    session: LibSeatSession,
    libinput: Libinput,
    drm: DrmDevice,
    gbm: GbmDevice<DrmDeviceFd>,
    renderer: GlesRenderer,
    render_node: Option<DrmNode>,
    formats: Vec<Format>,
    /// The GPU's device number, to know its udev events.
    dev: Option<libc_dev_t>,
    screens: Vec<Screen>,
    /// The session has the devices (not switched away to another VT).
    active: bool,
    /// A render is already queued for when the loop is idle.
    queued: bool,
}

/// One connected screen and what drives it.
struct Screen {
    output: Output,
    connector: connector::Handle,
    crtc: crtc::Handle,
    /// A laptop's own panel: it goes first.
    panel: bool,
    compositor: Compositor,
    /// A frame is on its way to the screen; the next waits for vblank.
    pending: bool,
    /// Something changed while a frame was pending.
    again: bool,
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

    /// Match the screens to what's connected: drop the ones unplugged,
    /// drive the ones plugged in, and lay them out again. True if any
    /// came or went. `probe` asks the connectors afresh (slower; for
    /// hotplug, when the kernel's idea may be stale).
    fn scan(&mut self, probe: bool) -> Result<bool, String> {
        let res = self
            .drm
            .resource_handles()
            .map_err(|e| format!("cannot read the GPU's screens: {e}"))?;
        let connected: Vec<connector::Info> = res
            .connectors()
            .iter()
            .filter_map(|c| self.drm.get_connector(*c, probe).ok())
            .filter(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
            .collect();

        let before = self.screens.len();
        self.screens.retain(|s| {
            let keep = connected.iter().any(|c| c.handle() == s.connector);
            if !keep {
                eprintln!("nordlys-comp: {} unplugged", s.output.name());
            }
            keep
        });
        let mut changed = self.screens.len() != before;

        for conn in &connected {
            if self.screens.iter().any(|s| s.connector == conn.handle()) {
                continue;
            }
            let used: Vec<crtc::Handle> = self.screens.iter().map(|s| s.crtc).collect();
            let crtc = conn
                .encoders()
                .iter()
                .filter_map(|e| self.drm.get_encoder(*e).ok())
                .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
                .find(|c| !used.contains(c));
            let name = connector_name(conn);
            let Some(crtc) = crtc else {
                eprintln!("nordlys-comp: no display controller left for {name}; it stays dark");
                continue;
            };
            match self.drive(conn, crtc) {
                Ok(s) => {
                    self.screens.push(s);
                    changed = true;
                }
                Err(e) => eprintln!("nordlys-comp: cannot drive {name}: {e}"),
            }
        }

        self.screens.sort_by_key(|s| (!s.panel, s.output.name()));
        self.place();
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

    /// A screen on `crtc`, in the connector's preferred mode.
    fn drive(&mut self, conn: &connector::Info, crtc: crtc::Handle) -> Result<Screen, String> {
        let drm_mode = conn
            .modes()
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .copied()
            .unwrap_or(conn.modes()[0]);
        let name = connector_name(conn);
        let (mm_w, mm_h) = conn.size().unwrap_or((0, 0));
        let surface = self
            .drm
            .create_surface(crtc, drm_mode, &[conn.handle()])
            .map_err(|e| e.to_string())?;

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

        let allocator = GbmAllocator::new(
            self.gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        let exporter = GbmFramebufferExporter::new(self.gbm.clone(), self.render_node);
        let compositor = DrmCompositor::new(
            &output,
            surface,
            None,
            allocator,
            exporter,
            [Fourcc::Xrgb8888, Fourcc::Argb8888],
            self.formats.clone(),
            self.drm.cursor_size(),
            Some(self.gbm.clone()),
        )
        .map_err(|e| format!("{e:?}"))?;
        eprintln!(
            "nordlys-comp: {name} at {w}×{h}@{}Hz, scale {scale}",
            drm_mode.vrefresh(),
        );
        Ok(Screen {
            output,
            connector: conn.handle(),
            crtc,
            panel: matches!(
                conn.interface(),
                connector::Interface::EmbeddedDisplayPort | connector::Interface::LVDS
            ),
            compositor,
            pending: false,
            again: false,
        })
    }
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

    let gpu: PathBuf = primary_gpu(&seat)
        .ok()
        .flatten()
        .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
        .ok_or("no GPU found")?;
    let fd = session
        .open(
            &gpu,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|e| format!("cannot open {}: {e}", gpu.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd.clone())?;
    // SAFETY: the GBM device lives as long as the display made from it.
    let egl = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&egl)?;
    // SAFETY: the context is current only on this thread.
    let renderer = unsafe { GlesRenderer::new(context)? };

    let node = DrmNode::from_file(&fd).ok();
    let render_node = node
        .and_then(|n| n.node_with_type(NodeType::Render)?.ok())
        .or(node);
    let formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput
        .udev_assign_seat(&seat)
        .map_err(|()| "libinput couldn't take the seat's input devices")?;

    let mut tty = Tty {
        session,
        libinput: libinput.clone(),
        drm,
        gbm,
        renderer,
        render_node,
        formats: formats.clone(),
        dev: node.map(|n| n.dev_id()),
        screens: Vec::new(),
        active: true,
        queued: false,
    };
    tty.scan(false)?;
    if tty.screens.is_empty() {
        return Err("no connected screen".into());
    }
    eprintln!(
        "nordlys-comp: {} screen{} on {}",
        tty.screens.len(),
        if tty.screens.len() == 1 { "" } else { "s" },
        gpu.display()
    );

    let handle = event_loop.handle();
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
    handle.insert_source(drm_notifier, |event, _, comp: &mut Comp| match event {
        DrmEvent::VBlank(crtc) => vblank(comp, crtc),
        DrmEvent::Error(e) => eprintln!("nordlys-comp: DRM: {e}"),
    })?;
    // Screens plugged in or out show up as a change to the GPU.
    match UdevBackend::new(&seat) {
        Ok(udev) => {
            handle.insert_source(udev, |event, _, comp: &mut Comp| {
                if let UdevEvent::Changed { device_id } = event {
                    hotplug(comp, device_id);
                }
            })?;
        }
        Err(e) => eprintln!("nordlys-comp: no hotplug, udev isn't there: {e}"),
    }

    let outputs = tty.outputs();
    Ok(Ready {
        backend: Backend::Tty(Box::new(tty)),
        outputs,
        formats,
        device: render_node.map(|n| n.dev_id()),
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
    match t.scan(probe) {
        Ok(true) => {
            let outputs = t.outputs();
            comp.set_outputs(outputs);
        }
        Ok(false) => {}
        Err(e) => eprintln!("nordlys-comp: {e}"),
    }
}

fn hotplug(comp: &mut Comp, device: libc_dev_t) {
    let Backend::Tty(t) = &comp.backend else {
        return;
    };
    if t.active && t.dev == Some(device) {
        rescan(comp, true);
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
            t.drm.pause();
            for s in &mut t.screens {
                s.pending = false;
            }
        }
        SessionEvent::ActivateSession => {
            if let Err(()) = t.libinput.resume() {
                eprintln!("nordlys-comp: libinput didn't come back");
            }
            if let Err(e) = t.drm.activate(false) {
                eprintln!("nordlys-comp: the screens didn't come back: {e}");
            }
            for s in &mut t.screens {
                if let Err(e) = s.compositor.reset_state() {
                    eprintln!("nordlys-comp: {} state: {e}", s.output.name());
                }
                s.compositor.reset_buffers();
                s.pending = false;
            }
            t.active = true;
            // Screens may have come or gone while we were away.
            rescan(comp, true);
            comp.request_redraw();
        }
    }
}

fn vblank(comp: &mut Comp, crtc: crtc::Handle) {
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    let Some(s) = t.screens.iter_mut().find(|s| s.crtc == crtc) else {
        return;
    };
    if let Err(e) = s.compositor.frame_submitted() {
        eprintln!("nordlys-comp: frame on {}: {e:?}", s.output.name());
    }
    s.pending = false;
    if std::mem::take(&mut s.again) {
        draw(comp, Some(crtc));
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

/// Draw the screens that can take a frame, or just the one on `only`.
fn draw(comp: &mut Comp, only: Option<crtc::Handle>) {
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
        screens, renderer, ..
    } = &mut *t;
    for s in screens.iter_mut() {
        if only.is_some_and(|c| c != s.crtc) {
            continue;
        }
        if s.pending {
            s.again = true;
            continue;
        }
        let elements = render::elements(comp, renderer, &s.output, true);
        let ground = comp.ground();
        match s
            .compositor
            .render_frame(renderer, &elements, ground, FrameFlags::DEFAULT)
        {
            Ok(frame) => {
                if !frame.is_empty {
                    match s.compositor.queue_frame(()) {
                        Ok(()) => s.pending = true,
                        Err(e) => eprintln!(
                            "nordlys-comp: cannot show the frame on {}: {e:?}",
                            s.output.name()
                        ),
                    }
                }
            }
            Err(e) => eprintln!("nordlys-comp: render on {} failed: {e:?}", s.output.name()),
        }
        render::shots(comp, renderer, &s.output, &elements);
        drawn.push(s.output.clone());
    }
    comp.backend = Backend::Tty(t);
    for o in &drawn {
        comp.frame_done(o);
    }
}
