//! Frosted glass: dual-kawase blur behind the shell's layer surfaces.
//!
//! Everything below the top layer is drawn into a backdrop texture, which
//! keeps a damage tracker of its own: when nothing behind the glass
//! changed, the blur from last frame is reused as is. Otherwise it's
//! blurred by halving it a few times with the kawase "down" filter and
//! growing it back with the "up" one, ending at half resolution.
//!
//! Each glass surface then gets a [`BlurElement`] right behind it. It
//! samples the surface's own picture as a mask and only blurs where that
//! is at least [`ALPHA_FLOOR`] opaque, like Hyprland's `ignore_alpha`: so
//! a bar that draws rounded pills on a clear strip gets frosted pills,
//! not a frosted strip.
//!
//! `theme.blur` in nordlys.toml is the radius; 0, or
//! `theme.reduce-transparency`, turns it off.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName,
    UniformType, UniformValue, ffi,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{Bind, Frame, Offscreen, Renderer, Texture};
use smithay::desktop::LayerSurface;
use smithay::output::OutputModeSource;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform};
use std::collections::HashMap;

/// Shell surfaces that are glass, by layer-shell namespace (the same
/// list the Hyprland session blurs).
const GLASS: [&str; 7] = [
    "nordlys-bar",
    "nordlys-launcher",
    "nordlys-shelf",
    "nordlys-ambient",
    "nordlys-notify",
    "nordlys-osd",
    "nordlys-welcome",
];

/// Pixels of a glass surface fainter than this stay unblurred.
pub const ALPHA_FLOOR: f32 = 0.3;

const PRELUDE: &str = "#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
precision mediump float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
";

const DOWN: &str = "
uniform vec2 halfpixel;
uniform float offset;
void main() {
    vec2 o = halfpixel * offset;
    vec4 sum = texture2D(tex, v_coords) * 4.0;
    sum += texture2D(tex, v_coords - o);
    sum += texture2D(tex, v_coords + o);
    sum += texture2D(tex, v_coords + vec2(o.x, -o.y));
    sum += texture2D(tex, v_coords - vec2(o.x, -o.y));
    gl_FragColor = sum / 8.0 * alpha;
}
";

const UP: &str = "
uniform vec2 halfpixel;
uniform float offset;
void main() {
    vec2 o = halfpixel * offset;
    vec4 sum = texture2D(tex, v_coords + vec2(-o.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(-o.x, o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(0.0, o.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(o.x, o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(o.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(o.x, -o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(0.0, -o.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(-o.x, -o.y)) * 2.0;
    gl_FragColor = sum / 12.0 * alpha;
}
";

/// Draws the blurred backdrop where the glass surface (bound as `mask` on
/// texture unit 1) is opaque enough. `crop` maps the backdrop's texture
/// coordinates to the surface's: (x, y, w, h), normalised.
const MASKED: &str = "
uniform sampler2D mask;
uniform vec4 crop;
uniform float mask_flip;
uniform float floor_alpha;
void main() {
    vec2 m = (v_coords - crop.xy) / crop.zw;
    if (mask_flip > 0.5) {
        m.y = 1.0 - m.y;
    }
    if (texture2D(mask, m).a < floor_alpha) {
        discard;
    }
    gl_FragColor = vec4(texture2D(tex, v_coords).rgb, 1.0) * alpha;
}
";

struct Programs {
    down: GlesTexProgram,
    up: GlesTexProgram,
    masked: GlesTexProgram,
}

pub struct Blur {
    radius: u8,
    enabled: bool,
    programs: Option<Result<Programs, ()>>,
    /// Everything under the glass, full size, and its damage tracker.
    backdrop: Option<(GlesTexture, OutputDamageTracker)>,
    /// Halved again and again; then grown back into `up`.
    down: Vec<GlesTexture>,
    up: Vec<GlesTexture>,
    /// Where the backdrop changed this frame (physical pixels).
    changed: Vec<Rectangle<i32, Physical>>,
    /// Per glass surface: its element id and when its blur last changed.
    surfaces: HashMap<ObjectId, (Id, CommitCounter)>,
}

impl Blur {
    pub fn new(theme: &nordlys_config::Theme) -> Self {
        let mut b = Blur {
            radius: 0,
            enabled: false,
            programs: None,
            backdrop: None,
            down: Vec::new(),
            up: Vec::new(),
            changed: Vec::new(),
            surfaces: HashMap::new(),
        };
        b.configure(theme);
        b
    }

    /// Follow nordlys.toml.
    pub fn configure(&mut self, theme: &nordlys_config::Theme) {
        let enabled = theme.blur > 0
            && !theme.reduce_transparency
            && std::env::var("NORDLYS_BLUR").as_deref() != Ok("0");
        if enabled != self.enabled || theme.blur != self.radius {
            // Start over: new passes, and every glass surface redrawn.
            self.backdrop = None;
            self.down.clear();
            self.up.clear();
        }
        self.enabled = enabled;
        self.radius = theme.blur;
    }

    pub fn forget(&mut self, surface: &WlSurface) {
        self.surfaces.remove(&surface.id());
    }

    /// Does any of these layer surfaces want blur (and can we)?
    pub fn wants<'a>(&self, layers: impl Iterator<Item = &'a LayerSurface>) -> bool {
        self.enabled && !matches!(self.programs, Some(Err(()))) && layers.into_iter().any(is_glass)
    }

    /// (passes, offset) for the radius: each pass doubles the reach.
    fn passes(&self) -> (usize, f32) {
        let r = f32::from(self.radius.max(1));
        let passes = match self.radius {
            0..=8 => 2,
            9..=20 => 3,
            21..=48 => 4,
            _ => 5,
        };
        let reach = (1u32 << (passes + 1)) as f32;
        (passes, (r / reach * 2.0).clamp(1.0, 6.0))
    }

    fn programs(&mut self, r: &mut GlesRenderer) -> Option<&Programs> {
        if self.programs.is_none() {
            let built = (|| -> Result<Programs, GlesError> {
                let pass = [
                    UniformName::new("halfpixel", UniformType::_2f),
                    UniformName::new("offset", UniformType::_1f),
                ];
                Ok(Programs {
                    down: r.compile_custom_texture_shader(format!("{PRELUDE}{DOWN}"), &pass)?,
                    up: r.compile_custom_texture_shader(format!("{PRELUDE}{UP}"), &pass)?,
                    masked: r.compile_custom_texture_shader(
                        format!("{PRELUDE}{MASKED}"),
                        &[
                            UniformName::new("mask", UniformType::_1i),
                            UniformName::new("crop", UniformType::_4f),
                            UniformName::new("mask_flip", UniformType::_1f),
                            UniformName::new("floor_alpha", UniformType::_1f),
                        ],
                    )?,
                })
            })();
            if let Err(e) = &built {
                eprintln!("nordlys-comp: no blur, the shaders didn't build: {e:?}");
            }
            self.programs = Some(built.map_err(|_| ()));
        }
        self.programs.as_ref()?.as_ref().ok()
    }

    /// Draw what's under the glass and blur it, unless nothing under it
    /// changed since the last frame.
    pub fn backdrop(
        &mut self,
        r: &mut GlesRenderer,
        below: &[crate::render::Elem],
        size: Size<i32, Physical>,
        scale: f64,
        ground: [f32; 4],
    ) {
        self.changed.clear();
        if self.programs(r).is_none() {
            return;
        }
        if let Err(e) = self.try_backdrop(r, below, size, scale, ground) {
            eprintln!("nordlys-comp: blur failed, turning it off: {e:?}");
            self.programs = Some(Err(()));
        }
    }

    fn try_backdrop(
        &mut self,
        r: &mut GlesRenderer,
        below: &[crate::render::Elem],
        size: Size<i32, Physical>,
        scale: f64,
        ground: [f32; 4],
    ) -> Result<(), GlesError> {
        let buffer = |s: Size<i32, Physical>| s.to_logical(1).to_buffer(1, Transform::Normal);
        let (passes, offset) = self.passes();
        let fresh = self.backdrop.as_ref().is_none_or(|(t, d)| {
            t.size() != buffer(size)
                || d.mode()
                    != &OutputModeSource::Static {
                        size,
                        scale: scale.into(),
                        transform: Transform::Normal,
                    }
        }) || self.down.len() != passes;
        if fresh {
            let tex = r.create_buffer(Fourcc::Abgr8888, buffer(size))?;
            self.backdrop = Some((
                tex,
                OutputDamageTracker::new(size, scale, Transform::Normal),
            ));
            self.down.clear();
            self.up.clear();
            let mut s = size;
            for _ in 0..passes {
                s = Size::from(((s.w / 2).max(1), (s.h / 2).max(1)));
                self.down
                    .push(r.create_buffer(Fourcc::Abgr8888, buffer(s))?);
            }
            // Back up to the first halving: down[passes-2] … down[0] sizes.
            for i in (0..passes - 1).rev() {
                let s = self.down[i].size();
                self.up.push(r.create_buffer(Fourcc::Abgr8888, s)?);
            }
        }

        let (tex, tracker) = self.backdrop.as_mut().expect("made above");
        let damage = {
            let mut target = r.bind(tex)?;
            // The backdrop texture is kept, so it's always one frame old.
            let age = if fresh { 0 } else { 1 };
            match tracker.render_output(r, &mut target, age, below, ground) {
                Ok(res) => res.damage.cloned().unwrap_or_default(),
                Err(e) => {
                    eprintln!("nordlys-comp: blur backdrop: {e:?}");
                    return Ok(());
                }
            }
        };
        if damage.is_empty() && !fresh {
            return Ok(());
        }
        self.changed = if fresh {
            vec![Rectangle::from_size(size)]
        } else {
            damage
        };

        let programs = self.programs.as_ref().and_then(|p| p.as_ref().ok());
        let Some(Programs { down, up, .. }) = programs else {
            return Ok(());
        };
        let (down, up) = (down.clone(), up.clone());
        let mut src = self.backdrop.as_ref().expect("made above").0.clone();
        for dst in self.down.iter_mut() {
            pass(r, &src, dst, &down, offset)?;
            src = dst.clone();
        }
        for dst in self.up.iter_mut() {
            pass(r, &src, dst, &up, offset)?;
            src = dst.clone();
        }
        Ok(())
    }

    /// The blurred backdrop at its smallest useful size: the end of the
    /// "up" passes, half the output.
    fn result(&self) -> Option<&GlesTexture> {
        self.up.last().or(self.down.first())
    }

    /// The element that frosts `layer`, placed at `geo` (physical), or
    /// None if it isn't glass or its picture isn't on the GPU yet.
    pub fn element(
        &mut self,
        r: &mut GlesRenderer,
        layer: &LayerSurface,
        geo: Rectangle<i32, Physical>,
    ) -> Option<BlurElement> {
        if !is_glass(layer) || geo.is_empty() {
            return None;
        }
        let program = self.programs.as_ref()?.as_ref().ok()?.masked.clone();
        let tex = self.result()?.clone();
        let mask = smithay::backend::renderer::utils::with_renderer_surface_state(
            layer.wl_surface(),
            |s| s.texture::<GlesTexture>(r.context_id()).cloned(),
        )
        .flatten()?;
        let full = self.backdrop.as_ref()?.0.size();
        let small = tex.size();
        let (sx, sy) = (
            f64::from(small.w) / f64::from(full.w.max(1)),
            f64::from(small.h) / f64::from(full.h.max(1)),
        );
        let src = Rectangle::<f64, Buffer>::new(
            Point::from((f64::from(geo.loc.x) * sx, f64::from(geo.loc.y) * sy)),
            Size::from((f64::from(geo.size.w) * sx, f64::from(geo.size.h) * sy)),
        );

        // A new commit when the backdrop changed near this surface; the
        // surface redrawing itself already damages the area under it.
        let reach = i32::from(self.radius) * 2;
        let near = Rectangle::new(
            geo.loc - Point::from((reach, reach)),
            geo.size + Size::from((reach * 2, reach * 2)),
        );
        let touched = self.changed.iter().any(|d| d.overlaps(near));
        let entry = self
            .surfaces
            .entry(layer.wl_surface().id())
            .or_insert_with(|| (Id::new(), CommitCounter::default()));
        if touched {
            entry.1.increment();
        }
        Some(BlurElement {
            id: entry.0.clone(),
            commit: entry.1,
            tex,
            src,
            dst: geo,
            mask,
            program,
        })
    }
}

fn is_glass(layer: &LayerSurface) -> bool {
    GLASS.contains(&layer.namespace())
}

/// One kawase pass: `src` filtered into all of `dst`.
fn pass(
    r: &mut GlesRenderer,
    src: &GlesTexture,
    dst: &mut GlesTexture,
    program: &GlesTexProgram,
    offset: f32,
) -> Result<(), GlesError> {
    let src_size = src.size();
    let dst_size = dst.size();
    let out = Rectangle::<i32, Physical>::from_size((dst_size.w, dst_size.h).into());
    let mut target = r.bind(dst)?;
    let mut frame = r.render(&mut target, out.size, Transform::Normal)?;
    frame.clear([0.0, 0.0, 0.0, 0.0].into(), &[out])?;
    frame.render_texture_from_to(
        src,
        Rectangle::from_size(src_size.to_f64()),
        out,
        &[out],
        &[],
        Transform::Normal,
        1.0,
        Some(program),
        &[
            Uniform::new(
                "halfpixel",
                UniformValue::_2f(
                    0.5 / src_size.w.max(1) as f32,
                    0.5 / src_size.h.max(1) as f32,
                ),
            ),
            Uniform::new("offset", UniformValue::_1f(offset)),
        ],
    )?;
    let _ = frame.finish()?;
    Ok(())
}

/// The blurred backdrop, cut to a glass surface's shape.
pub struct BlurElement {
    id: Id,
    commit: CommitCounter,
    tex: GlesTexture,
    src: Rectangle<f64, Buffer>,
    dst: Rectangle<i32, Physical>,
    mask: GlesTexture,
    program: GlesTexProgram,
}

impl Element for BlurElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.src
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.dst
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for BlurElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let tex = self.tex.size().to_f64();
        let crop = (
            (src.loc.x / tex.w) as f32,
            (src.loc.y / tex.h) as f32,
            (src.size.w / tex.w) as f32,
            (src.size.h / tex.h) as f32,
        );
        let mask = self.mask.tex_id();
        frame.with_context(|gl| unsafe {
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, mask);
            gl.ActiveTexture(ffi::TEXTURE0);
        })?;
        let drawn = frame.render_texture_from_to(
            &self.tex,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            1.0,
            Some(&self.program),
            &[
                Uniform::new("mask", UniformValue::_1i(1)),
                Uniform::new("crop", UniformValue::_4f(crop.0, crop.1, crop.2, crop.3)),
                Uniform::new(
                    "mask_flip",
                    UniformValue::_1f(if self.mask.is_y_inverted() { 1.0 } else { 0.0 }),
                ),
                Uniform::new("floor_alpha", UniformValue::_1f(ALPHA_FLOOR)),
            ],
        );
        frame.with_context(|gl| unsafe {
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            gl.ActiveTexture(ffi::TEXTURE0);
        })?;
        drawn
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blur(radius: u8, reduce: bool) -> Blur {
        Blur::new(&nordlys_config::Theme {
            blur: radius,
            reduce_transparency: reduce,
            ..Default::default()
        })
    }

    #[test]
    fn more_radius_means_more_passes_never_fewer_than_two() {
        let p = |r| blur(r, false).passes();
        assert_eq!(p(1).0, 2);
        assert_eq!(p(24).0, 4);
        assert!(p(8).0 <= p(20).0 && p(20).0 <= p(48).0 && p(48).0 <= p(200).0);
        for r in [1, 8, 24, 100, 255] {
            let (_, offset) = p(r);
            assert!((1.0..=6.0).contains(&offset), "radius {r}: offset {offset}");
        }
    }

    #[test]
    fn zero_radius_or_reduced_transparency_turn_it_off() {
        assert!(!blur(0, false).enabled);
        assert!(!blur(24, true).enabled);
        assert!(blur(24, false).enabled || std::env::var("NORDLYS_BLUR").as_deref() == Ok("0"));
    }
}
