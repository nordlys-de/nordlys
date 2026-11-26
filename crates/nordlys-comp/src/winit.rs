//! The nested backend: a window on the desktop you're already in, with a
//! Wayland display of its own inside. The desktop around draws the
//! pointer and decides when frames are wanted.
//!
//! `NORDLYS_NESTED_SCREENS=2` (up to 4) splits the window into that many
//! screens side by side, `nested-1`, `nested-2`…, to try what several
//! screens do without the hardware. With one, the screen is `nested`.

use crate::comp::{Backend, Comp, Ready};
use crate::render;
use smithay::backend::allocator::Format;
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{Physical, Point, Rectangle, Size, Transform};

pub struct Winit {
    backend: WinitGraphicsBackend<GlesRenderer>,
    /// For the whole window, every screen in it.
    damage: OutputDamageTracker,
}

/// How many screens the window is split into.
fn screen_count() -> i32 {
    std::env::var("NORDLYS_NESTED_SCREENS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| (1..=4).contains(n))
        .unwrap_or(1)
}

/// A tracker for the whole window. GL draws it upside down.
fn tracker(size: Size<i32, Physical>) -> OutputDamageTracker {
    OutputDamageTracker::new(size, 1.0, Transform::Flipped180)
}

/// Size and place each screen: equal slices of the window, left to right.
fn place(outputs: &[Output], window: Size<i32, Physical>) {
    let n = outputs.len().max(1) as i32;
    let w = (window.w / n).max(1);
    for (i, o) in outputs.iter().enumerate() {
        let i = i as i32;
        // The last takes what division left over.
        let this = if i == n - 1 {
            window.w - w * (n - 1)
        } else {
            w
        };
        let mode = Mode {
            size: (this.max(1), window.h.max(1)).into(),
            refresh: 60_000,
        };
        o.change_current_state(Some(mode), None, None, Some((w * i, 0).into()));
        o.set_preferred(mode);
    }
}

impl Winit {
    pub fn renderer(&mut self) -> &mut GlesRenderer {
        self.backend.renderer()
    }

    pub fn request_redraw(&mut self) {
        self.backend.window().request_redraw();
    }
}

pub fn init(
    event_loop: &EventLoop<'static, Comp>,
    _dh: &DisplayHandle,
    socket_name: &str,
) -> Result<Ready, Box<dyn std::error::Error>> {
    let (mut backend, winit) = winit::init_from_attributes::<GlesRenderer>(
        smithay::reexports::winit::window::Window::default_attributes()
            .with_title(format!("Nordlys (nested, {socket_name})"))
            .with_inner_size(smithay::reexports::winit::dpi::LogicalSize::new(
                1280.0, 800.0,
            )),
    )
    .map_err(|e| format!("cannot open a window: {e}"))?;

    let count = screen_count();
    let outputs: Vec<Output> = (1..=count)
        .map(|i| {
            let name = if count == 1 {
                "nested".to_owned()
            } else {
                format!("nested-{i}")
            };
            let o = Output::new(
                name,
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Nordlys".into(),
                    model: "Nested".into(),
                },
            );
            o.change_current_state(None, Some(Transform::Normal), None, None);
            o
        })
        .collect();
    place(&outputs, backend.window_size());
    let damage = tracker(backend.window_size());

    let formats: Vec<Format> = backend.renderer().dmabuf_formats().into_iter().collect();
    let device = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .ok()
        .and_then(|d| d.try_get_render_node().ok().flatten())
        .map(|n| n.dev_id());

    event_loop
        .handle()
        .insert_source(winit, |event, _, comp: &mut Comp| event_for(comp, event))?;

    Ok(Ready {
        backend: Backend::Winit(Box::new(Winit { backend, damage })),
        outputs,
        formats,
        device,
    })
}

fn event_for(comp: &mut Comp, event: WinitEvent) {
    match event {
        WinitEvent::Resized { size, .. } => {
            if let Backend::Winit(w) = &mut comp.backend {
                w.damage = tracker(size);
            }
            let outputs = comp.outputs.clone();
            place(&outputs, size);
            comp.set_outputs(outputs);
        }
        WinitEvent::Input(event) => comp.input(event),
        WinitEvent::Redraw => render(comp),
        WinitEvent::CloseRequested => {
            eprintln!("nordlys-comp: the window was closed");
            comp.signal.stop();
        }
        WinitEvent::Focus(_) => {}
    }
}

pub fn render(comp: &mut Comp) {
    let Backend::Winit(mut w) = std::mem::replace(&mut comp.backend, Backend::Taken) else {
        return;
    };
    // Each screen's elements, moved to its slice of the window.
    let outputs = comp.outputs.clone();
    let per_screen: Vec<(Output, Vec<render::Elem>)> = outputs
        .iter()
        .map(|out| {
            (
                out.clone(),
                render::elements(comp, w.renderer(), out, false),
            )
        })
        .collect();
    let elements: Vec<_> = per_screen
        .iter()
        .flat_map(|(out, screen)| {
            let at = out.current_location();
            let offset = Point::<i32, Physical>::from((at.x, at.y));
            screen
                .iter()
                .map(move |e| RelocateRenderElement::from_element(e, offset, Relocate::Relative))
        })
        .collect();
    let ground = comp.ground();
    let size = w.backend.window_size();
    let age = w.backend.buffer_age().unwrap_or(0);
    let drawn = match w.backend.bind() {
        Ok((renderer, mut fb)) => {
            match w
                .damage
                .render_output(renderer, &mut fb, age, &elements, ground)
            {
                Ok(res) => Some(res.damage.cloned()),
                Err(e) => {
                    eprintln!("nordlys-comp: render failed: {e:?}");
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("nordlys-comp: cannot draw: {e:?}");
            None
        }
    };
    if let Some(damage) = drawn {
        let full = [Rectangle::from_size(size)];
        let damage = damage.unwrap_or_default();
        let shown = if damage.is_empty() {
            &full[..]
        } else {
            &damage[..]
        };
        if let Err(e) = w.backend.submit(Some(shown)) {
            eprintln!("nordlys-comp: cannot show the frame: {e:?}");
        }
        for (out, screen) in &per_screen {
            render::shots(comp, w.renderer(), out, screen);
        }
    }
    w.request_redraw();
    comp.backend = Backend::Winit(w);
    for out in &outputs {
        comp.frame_done(out);
    }
}
