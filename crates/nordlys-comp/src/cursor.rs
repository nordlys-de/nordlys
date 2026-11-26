//! The pointer, drawn by the compositor itself on the screen (nested, the
//! desktop around draws its own). Apps may hand over a cursor surface;
//! otherwise it's the theme's arrow: `$XCURSOR_THEME` at `$XCURSOR_SIZE`,
//! else Adwaita at 24, else a plain square so there's always something.

use crate::render::Elem;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::utils::{Logical, Point, Scale, Transform};
use smithay::wayland::compositor::with_states;

pub struct Cursor {
    pub status: CursorImageStatus,
    /// Drawn by the compositor (on the screen), so moving it needs frames.
    pub drawn: bool,
    arrow: MemoryRenderBuffer,
    hotspot: Point<i32, Logical>,
}

impl Cursor {
    pub fn load() -> Self {
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24u32);
        let theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "Adwaita".into());
        let (arrow, hotspot) = [theme.as_str(), "Adwaita", "default"]
            .into_iter()
            .find_map(|t| themed(t, size))
            .unwrap_or_else(fallback);
        Cursor {
            status: CursorImageStatus::default_named(),
            drawn: false,
            arrow,
            hotspot,
        }
    }

    pub fn elements(
        &self,
        r: &mut GlesRenderer,
        pos: Point<f64, Logical>,
        scale: Scale<f64>,
    ) -> Vec<Elem> {
        match &self.status {
            CursorImageStatus::Hidden => Vec::new(),
            CursorImageStatus::Surface(s) => {
                let hotspot = with_states(s, |st| {
                    st.data_map
                        .get::<CursorImageSurfaceData>()
                        .and_then(|d| d.lock().ok().map(|d| d.hotspot))
                })
                .unwrap_or_default();
                let at = (pos - hotspot.to_f64()).to_physical(scale).to_i32_round();
                render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<_>>(
                    r,
                    s,
                    at,
                    scale,
                    1.0,
                    Kind::Cursor,
                )
                .into_iter()
                .map(Elem::from)
                .collect()
            }
            CursorImageStatus::Named(_) => {
                let at = (pos - self.hotspot.to_f64()).to_physical(scale);
                MemoryRenderBufferRenderElement::from_buffer(
                    r,
                    at,
                    &self.arrow,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                )
                .map(|e| vec![Elem::from(e)])
                .unwrap_or_default()
            }
        }
    }
}

/// The theme's default arrow, nearest to `size`.
fn themed(theme: &str, size: u32) -> Option<(MemoryRenderBuffer, Point<i32, Logical>)> {
    let theme = xcursor::CursorTheme::load(theme);
    let path = ["default", "left_ptr", "arrow"]
        .into_iter()
        .find_map(|name| theme.load_icon(name))?;
    let bytes = std::fs::read(path).ok()?;
    let images = xcursor::parser::parse_xcursor(&bytes)?;
    let best = images
        .iter()
        .min_by_key(|i| (i.size as i64 - size as i64).abs())?;
    // xcursor hands over RGBA bytes, premultiplied: Abgr8888 to DRM.
    let buf = MemoryRenderBuffer::from_slice(
        &best.pixels_rgba,
        Fourcc::Abgr8888,
        (best.width as i32, best.height as i32),
        1,
        Transform::Normal,
        None,
    );
    Some((buf, Point::from((best.xhot as i32, best.yhot as i32))))
}

/// No theme installed: a small white square with a dark edge.
fn fallback() -> (MemoryRenderBuffer, Point<i32, Logical>) {
    const S: usize = 10;
    let mut px = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let edge = x == 0 || y == 0 || x == S - 1 || y == S - 1;
            let v = if edge { 30 } else { 240 };
            px[(y * S + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    let buf = MemoryRenderBuffer::from_slice(
        &px,
        Fourcc::Abgr8888,
        (S as i32, S as i32),
        1,
        Transform::Normal,
        None,
    );
    (buf, Point::from((0, 0)))
}
