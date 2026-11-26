//! The screen itself: DRM/KMS for the picture, libinput for keyboards and
//! pointers, and libseat for the right to use them, so it runs from a TTY
//! or a display manager without being root (logind or seatd hand over the
//! devices).
//!
//! One GPU (the primary one) and one screen: the first connected one, in
//! its preferred mode. Frames are drawn on demand and paced by the
//! screen's vblank; switching to another VT (Ctrl+Alt+F1…) pauses
//! everything and coming back redraws from scratch.

use crate::comp::{Backend, Comp, Ready};
use crate::render;
use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
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
use smithay::backend::udev::{all_gpus, primary_gpu};
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
    renderer: GlesRenderer,
    compositor: Compositor,
    crtc: crtc::Handle,
    /// The session has the devices (not switched away to another VT).
    active: bool,
    /// A frame is on its way to the screen; the next waits for vblank.
    pending: bool,
    /// Something changed while a frame was pending.
    again: bool,
    /// A render is already queued for when the loop is idle.
    queued: bool,
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
    let (mut drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
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

    let (conn, crtc, drm_mode) = pick_screen(&drm)?;
    let name = format!(
        "{}-{}",
        interface_name(conn.interface()),
        conn.interface_id()
    );
    let (mm_w, mm_h) = conn.size().unwrap_or((0, 0));
    let surface = drm.create_surface(crtc, drm_mode, &[conn.handle()])?;

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
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node);
    let formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();
    let compositor = DrmCompositor::new(
        &output,
        surface,
        None,
        allocator,
        exporter,
        [Fourcc::Xrgb8888, Fourcc::Argb8888],
        formats.clone(),
        drm.cursor_size(),
        Some(gbm),
    )
    .map_err(|e| format!("cannot drive {name}: {e:?}"))?;
    eprintln!(
        "nordlys-comp: {name} at {w}×{h}@{}Hz, scale {scale}, on {}",
        drm_mode.vrefresh(),
        gpu.display()
    );

    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput
        .udev_assign_seat(&seat)
        .map_err(|()| "libinput couldn't take the seat's input devices")?;

    let handle = event_loop.handle();
    handle.insert_source(
        LibinputInputBackend::new(libinput.clone()),
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

    Ok(Ready {
        backend: Backend::Tty(Box::new(Tty {
            session,
            libinput,
            drm,
            renderer,
            compositor,
            crtc,
            active: true,
            pending: false,
            again: false,
            queued: false,
        })),
        output,
        formats,
        device: render_node.map(|n| n.dev_id()),
    })
}

/// The first connected screen, its preferred mode, and a CRTC to drive it.
fn pick_screen(
    drm: &DrmDevice,
) -> Result<
    (
        connector::Info,
        crtc::Handle,
        smithay::reexports::drm::control::Mode,
    ),
    String,
> {
    let res = drm
        .resource_handles()
        .map_err(|e| format!("cannot read the GPU's screens: {e}"))?;
    let connected: Vec<connector::Info> = res
        .connectors()
        .iter()
        .filter_map(|c| drm.get_connector(*c, false).ok())
        .filter(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
        .collect();
    // A laptop's own panel first, when it's on.
    let mut ordered: Vec<&connector::Info> = connected.iter().collect();
    ordered.sort_by_key(|c| {
        !matches!(
            c.interface(),
            connector::Interface::EmbeddedDisplayPort | connector::Interface::LVDS
        )
    });
    for conn in ordered {
        let mode = conn
            .modes()
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .copied()
            .unwrap_or(conn.modes()[0]);
        let crtc = conn
            .encoders()
            .iter()
            .filter_map(|e| drm.get_encoder(*e).ok())
            .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
            .next();
        if let Some(crtc) = crtc {
            return Ok((conn.clone(), crtc, mode));
        }
    }
    Err("no connected screen".into())
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

/// 2 on dense screens (about 190 dpi and up), else 1; `NORDLYS_SCALE`
/// overrides it.
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

fn session_event(comp: &mut Comp, event: SessionEvent) {
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    match event {
        SessionEvent::PauseSession => {
            t.active = false;
            t.libinput.suspend();
            t.drm.pause();
            t.pending = false;
        }
        SessionEvent::ActivateSession => {
            if let Err(()) = t.libinput.resume() {
                eprintln!("nordlys-comp: libinput didn't come back");
            }
            if let Err(e) = t.drm.activate(false) {
                eprintln!("nordlys-comp: the screen didn't come back: {e}");
            }
            if let Err(e) = t.compositor.reset_state() {
                eprintln!("nordlys-comp: screen state: {e}");
            }
            t.compositor.reset_buffers();
            t.active = true;
            t.pending = false;
            comp.request_redraw();
        }
    }
}

fn vblank(comp: &mut Comp, crtc: crtc::Handle) {
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    if crtc != t.crtc {
        return;
    }
    if let Err(e) = t.compositor.frame_submitted() {
        eprintln!("nordlys-comp: frame: {e:?}");
    }
    t.pending = false;
    if std::mem::take(&mut t.again) {
        render(comp);
    }
}

/// Draw when the loop is next idle, or after the frame on its way.
pub fn schedule(comp: &mut Comp) {
    let Backend::Tty(t) = &mut comp.backend else {
        return;
    };
    if t.pending {
        t.again = true;
        return;
    }
    if t.queued {
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

pub fn render(comp: &mut Comp) {
    let Backend::Tty(mut t) = std::mem::replace(&mut comp.backend, Backend::Taken) else {
        return;
    };
    if !t.active {
        comp.backend = Backend::Tty(t);
        return;
    }
    if t.pending {
        t.again = true;
        comp.backend = Backend::Tty(t);
        return;
    }
    comp.cursor.drawn = true;
    let elements = render::elements(comp, &mut t.renderer, true);
    let ground = comp.ground();
    let result = t
        .compositor
        .render_frame(&mut t.renderer, &elements, ground, FrameFlags::DEFAULT);
    match result {
        Ok(frame) => {
            if !frame.is_empty {
                match t.compositor.queue_frame(()) {
                    Ok(()) => t.pending = true,
                    Err(e) => eprintln!("nordlys-comp: cannot show the frame: {e:?}"),
                }
            }
        }
        Err(e) => eprintln!("nordlys-comp: render failed: {e:?}"),
    }
    render::shots(comp, &mut t.renderer, &elements);
    comp.backend = Backend::Tty(t);
    comp.frame_done();
}
