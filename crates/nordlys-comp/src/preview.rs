//! Pictures of the windows waiting on the shelf, for the shell's flyout.
//!
//! Shelved windows are unmapped, so nobody sees them; every
//! `stage.shelf_fps`th of a second they're sent a frame (so they keep
//! drawing) and, if they drew something new, drawn small into a texture
//! and written to `nordlys.previews/<id>.rgba` next to the IPC socket (see
//! [`nordlys_ipc::preview_path`]). Raw pixels, so there's no encoding to
//! pay for; the shell reads them while its flyout is open.

use crate::render;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::element::{AsRenderElements, Element, Id, RenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::desktop::Window;
use smithay::utils::{Point, Scale, Size};
use std::collections::HashMap;
use std::path::PathBuf;

/// Largest preview, in pixels.
const MAX_W: f64 = 480.0;
const MAX_H: f64 = 300.0;

pub struct Previews {
    dir: PathBuf,
    /// What each window's picture was made of, to skip unchanged ones.
    seen: HashMap<String, Vec<(Id, CommitCounter)>>,
}

impl Previews {
    pub fn new(dir: PathBuf) -> Self {
        // Pictures from a compositor that didn't clean up are stale.
        let _ = std::fs::remove_dir_all(&dir);
        Previews {
            dir,
            seen: HashMap::new(),
        }
    }

    /// Picture `w` as window `id`, if it changed since the last one.
    pub fn capture(&mut self, r: &mut GlesRenderer, id: &str, w: &Window) {
        let geo = w.geometry();
        if geo.size.w <= 0 || geo.size.h <= 0 {
            return;
        }
        let s = (MAX_W / f64::from(geo.size.w))
            .min(MAX_H / f64::from(geo.size.h))
            .min(1.0);
        let scale = Scale::from(s);
        let size: Size<i32, smithay::utils::Physical> =
            geo.size.to_f64().to_physical(scale).to_i32_round();
        if size.w <= 0 || size.h <= 0 {
            return;
        }
        // The window's own content starts at its geometry, past any
        // client-side shadow.
        let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            w.render_elements(r, Point::from((0, 0)), scale, 1.0);
        let shift = Point::from((0, 0)) - geo.loc.to_f64().to_physical(scale).to_i32_round();
        let elements: Vec<RelocateRenderElement<_>> = elements
            .into_iter()
            .map(|e| RelocateRenderElement::from_element(e, shift, Relocate::Relative))
            .collect();
        let made_of: Vec<(Id, CommitCounter)> = elements
            .iter()
            .map(|e| (e.id().clone(), e.current_commit()))
            .collect();
        if self.seen.get(id) == Some(&made_of) {
            return;
        }
        match render::read_back(r, &elements, size, s, [0.0, 0.0, 0.0, 0.0]) {
            Ok(mut px) => {
                unpremultiply(&mut px);
                if let Err(e) = self.write(id, size.w as u32, size.h as u32, &px) {
                    eprintln!("nordlys-comp: preview of {id}: {e}");
                    return;
                }
                self.seen.insert(id.to_owned(), made_of);
            }
            Err(e) => eprintln!("nordlys-comp: preview of {id}: {e}"),
        }
    }

    fn write(&self, id: &str, w: u32, h: u32, px: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = nordlys_ipc::preview_path(&self.dir, id);
        let part = path.with_extension("part");
        std::fs::write(&part, nordlys_ipc::encode_preview(w, h, px))?;
        std::fs::rename(&part, &path)
    }

    pub fn forget(&mut self, id: &str) {
        self.seen.remove(id);
        let _ = std::fs::remove_file(nordlys_ipc::preview_path(&self.dir, id));
    }

    /// Drop pictures of windows that are gone.
    pub fn forget_except(&mut self, ids: &[String]) {
        let gone: Vec<String> = self
            .seen
            .keys()
            .filter(|k| !ids.contains(k))
            .cloned()
            .collect();
        for id in gone {
            self.forget(&id);
        }
    }

    pub fn clear(&mut self) {
        self.seen.clear();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The GPU gives premultiplied colour; the preview format is straight.
fn unpremultiply(px: &mut [u8]) {
    for p in px.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

// Elements in a preview are relocated surfaces; this keeps the bound
// spelled out for the compiler.
#[allow(dead_code)]
fn _assert_renderable(e: &RelocateRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>) {
    fn is<E: RenderElement<GlesRenderer>>(_: &E) {}
    is(e);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpremultiply_restores_straight_colour() {
        let mut px = [64, 32, 0, 128, 10, 20, 30, 255, 9, 9, 9, 0];
        unpremultiply(&mut px);
        assert_eq!(&px[..4], &[128, 64, 0, 128]);
        assert_eq!(&px[4..8], &[10, 20, 30, 255]);
        assert_eq!(&px[8..], &[9, 9, 9, 0]);
    }
}
