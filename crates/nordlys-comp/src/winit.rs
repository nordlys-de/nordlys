//! The nested backend: a window on the desktop you're already in, with a
//! Wayland display of its own inside. The desktop around draws the
//! pointer and decides when frames are wanted.

use crate::comp::{Backend, Comp, Ready};
use crate::render;
use smithay::backend::allocator::Format;
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::utils::{Rectangle, Transform};

pub struct Winit {
    backend: WinitGraphicsBackend<GlesRenderer>,
    damage: OutputDamageTracker,
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

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };
    let output = Output::new(
        "nested".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Nordlys".into(),
            model: "Nested".into(),
        },
    );
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    let damage = OutputDamageTracker::from_output(&output);

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
        output,
        formats,
        device,
    })
}

fn event_for(comp: &mut Comp, event: WinitEvent) {
    match event {
        WinitEvent::Resized { size, .. } => {
            let mode = Mode {
                size,
                refresh: 60_000,
            };
            comp.output
                .change_current_state(Some(mode), None, None, None);
            comp.output.set_preferred(mode);
            comp.output_changed();
        }
        WinitEvent::Input(event) => comp.input(event),
        WinitEvent::Redraw => render(comp),
        WinitEvent::CloseRequested => comp.signal.stop(),
        WinitEvent::Focus(_) => {}
    }
}

pub fn render(comp: &mut Comp) {
    let Backend::Winit(mut w) = std::mem::replace(&mut comp.backend, Backend::Taken) else {
        return;
    };
    let elements = render::elements(comp, w.renderer(), false);
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
        render::shots(comp, w.renderer(), &elements);
    }
    w.request_redraw();
    comp.backend = Backend::Winit(w);
    comp.frame_done();
}
