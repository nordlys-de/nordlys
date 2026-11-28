//! What a screen's frame is made of, for either backend: the lock screen
//! alone while it's up; otherwise the cursor, overlay and top layers (with
//! their blur behind them), the accent ring, the windows, then the bottom
//! and background layers. Front to back, as Smithay wants them. Each
//! screen is drawn on its own, from the part of the desktop it shows.

use crate::comp::{Comp, Lock};
use nordlys_config::Rgb;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{AsRenderElements, Kind, render_elements};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::desktop::layer_map_for_output;
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Rectangle, Scale, Size, Transform};
use smithay::wayland::shell::wlr_layer::Layer;
use std::path::Path;

render_elements! {
    pub Elem<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Blur=crate::blur::BlurElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

/// Premultiplied RGBA for the renderer.
pub fn color(c: Rgb, alpha: f32) -> [f32; 4] {
    let f = |v: u8| v as f32 / 255.0 * alpha;
    [f(c.0), f(c.1), f(c.2), alpha]
}

pub fn scale(output: &Output) -> Scale<f64> {
    Scale::from(output.current_scale().fractional_scale())
}

/// The output's size in physical pixels.
pub fn output_size(output: &Output) -> Size<i32, Physical> {
    output
        .current_mode()
        .map(|m| m.size)
        .unwrap_or_else(|| (1, 1).into())
}

/// Every element of `output`'s next frame, front to back, placed on that
/// output (its top-left corner is 0, 0). `with_cursor` when the backend
/// has to draw the pointer itself (on the screen; nested, the desktop
/// around draws it).
pub fn elements(
    comp: &mut Comp,
    r: &mut GlesRenderer,
    output: &Output,
    with_cursor: bool,
) -> Vec<Elem> {
    let scale = scale(output);
    let region = comp.space.output_geometry(output).unwrap_or_default();
    let origin = region.loc;
    let mut out: Vec<Elem> = Vec::new();
    if with_cursor {
        let pos = comp
            .seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        if region.to_f64().contains(pos) {
            out.extend(comp.cursor.elements(r, pos - origin.to_f64(), scale));
        }
    }

    if let Lock::Locked { surfaces, .. } = &comp.lock {
        // Only the lock screen; until it has drawn, nothing but the ground.
        if let Some((_, s)) = surfaces.iter().find(|(o, _)| o == output) {
            out.extend(
                render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<_>>(
                    r,
                    s.wl_surface(),
                    (0, 0),
                    scale,
                    1.0,
                    Kind::Unspecified,
                )
                .into_iter()
                .map(Elem::from),
            );
        }
        return out;
    }

    // Everything under the top layer: what glass surfaces blur.
    let mut below: Vec<Elem> = Vec::new();
    for (i, (x, y, w, h)) in comp.ring_rects().into_iter().enumerate().take(4) {
        let rect = Rectangle::<i32, Logical>::new((x, y).into(), (w, h).into());
        if !rect.overlaps(region) {
            continue;
        }
        let accent = color(comp.accent(), 0.9);
        let buf: &mut SolidColorBuffer = &mut comp.ring[i];
        buf.update((w, h), accent);
        below.push(Elem::from(SolidColorRenderElement::from_buffer(
            buf,
            (rect.loc - origin).to_physical_precise_round(scale),
            scale,
            1.0,
            Kind::Unspecified,
        )));
    }
    below.extend(
        comp.space
            .render_elements_for_region(r, &region, scale, 1.0)
            .into_iter()
            .map(Elem::from),
    );
    below.extend(layer_elements(
        output,
        r,
        &[Layer::Bottom, Layer::Background],
        scale,
    ));

    // The glass on top, each with its blur behind it.
    let map = layer_map_for_output(output);
    // A fullscreen window hides the bar and shelf; notifications, the
    // launcher and the OSD (the overlay layer) still show over it.
    let layers: &[Layer] = if comp.covered(output) {
        &[Layer::Overlay]
    } else {
        &[Layer::Overlay, Layer::Top]
    };
    let glass: Vec<_> = layers
        .iter()
        .copied()
        .flat_map(|l| map.layers_on(l).rev().cloned().collect::<Vec<_>>())
        .filter_map(|l| Some((map.layer_geometry(&l)?, l)))
        .collect();
    drop(map);
    let screen = output.name();
    let blurs = comp.blur.wants(glass.iter().map(|(_, l)| l));
    if blurs {
        let size = output_size(output);
        let ground = comp.ground();
        comp.blur
            .backdrop(r, &screen, &below, size, scale.x, ground);
    }
    for (geo, layer) in &glass {
        let loc = geo.loc.to_physical_precise_round(scale);
        out.extend(
            layer
                .render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(r, loc, scale, 1.0)
                .into_iter()
                .map(Elem::from),
        );
        if blurs
            && let Some(b) =
                comp.blur
                    .element(r, &screen, layer, geo.to_physical_precise_round(scale))
        {
            out.push(Elem::from(b));
        }
    }
    out.extend(below);
    out
}

fn layer_elements(
    output: &Output,
    r: &mut GlesRenderer,
    layers: &[Layer],
    scale: Scale<f64>,
) -> Vec<Elem> {
    let map = layer_map_for_output(output);
    let mut out = Vec::new();
    for &layer in layers {
        for l in map.layers_on(layer).rev() {
            let Some(geo) = map.layer_geometry(l) else {
                continue;
            };
            let loc = geo.loc.to_physical_precise_round(scale);
            out.extend(
                l.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(r, loc, scale, 1.0)
                    .into_iter()
                    .map(Elem::from),
            );
        }
    }
    out
}

/// Draw `elements` into a texture of `size` and read the pixels back:
/// RGBA, straight from the GPU, top row first.
pub fn read_back(
    r: &mut GlesRenderer,
    elements: &[impl smithay::backend::renderer::element::RenderElement<GlesRenderer>],
    size: Size<i32, Physical>,
    scale: f64,
    clear: [f32; 4],
) -> Result<Vec<u8>, String> {
    let buffer_size = size.to_logical(1).to_buffer(1, Transform::Normal);
    let mut tex: GlesTexture = r
        .create_buffer(Fourcc::Abgr8888, buffer_size)
        .map_err(|e| format!("{e:?}"))?;
    let mut target = r.bind(&mut tex).map_err(|e| format!("{e:?}"))?;
    let mut damage = OutputDamageTracker::new(size, scale, Transform::Normal);
    damage
        .render_output(r, &mut target, 0, elements, clear)
        .map_err(|e| format!("{e:?}"))?;
    let region = Rectangle::from_size(buffer_size);
    let mapping = r
        .copy_framebuffer(&target, region, Fourcc::Abgr8888)
        .map_err(|e| format!("{e:?}"))?;
    r.map_texture(&mapping)
        .map(<[u8]>::to_vec)
        .map_err(|e| format!("{e:?}"))
}

/// Save the frame for every screenshot asked for since the last one:
/// what the focused screen shows, once `output` (the one just drawn) is it.
pub fn shots(comp: &mut Comp, r: &mut GlesRenderer, output: &Output, elements: &[Elem]) {
    if comp.shots.is_empty() || comp.focused_output().as_ref() != Some(output) {
        return;
    }
    let size = output_size(output);
    let pixels = read_back(r, elements, size, scale(output).x, comp.ground());
    for path in std::mem::take(&mut comp.shots) {
        let saved = pixels
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|px| save_png(&path, size.w as u32, size.h as u32, px));
        match saved {
            Ok(()) => eprintln!("nordlys-comp: saved {}", path.display()),
            Err(e) => eprintln!("nordlys-comp: screenshot failed: {e}"),
        }
    }
}

/// Write RGBA pixels, top row first, as a PNG. Written beside the target
/// and renamed, so nobody sees half a file.
pub fn save_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let row = w as usize * 4;
    if rgba.len() < row * h as usize {
        return Err("short read from the GPU".into());
    }
    let part = path.with_extension("png.part");
    let file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(&rgba[..row * h as usize]))
        .map_err(|e| e.to_string())?;
    std::fs::rename(&part, path).map_err(|e| e.to_string())
}
