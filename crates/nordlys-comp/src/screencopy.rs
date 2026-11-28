//! wlr-screencopy: how grim (screenshots, with slurp for a region),
//! wf-recorder and gpu-screen-recorder see the screen. Smithay has no
//! implementation of it, so this is one, for shared-memory buffers.
//!
//! A client asks for a screen (or a region of it); it gets told the size
//! and format to allocate, then hands a buffer over with `copy`. The next
//! frame drawn for that screen is read back into the buffer. `copy` gets
//! a frame drawn now; `copy_with_damage` (what recorders use) waits for
//! the screen to change.

use crate::comp::Comp;
use crate::render::{self, Elem};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Clock, Monotonic, Physical, Rectangle};
use smithay::wayland::shm::{with_buffer_contents, with_buffer_contents_mut};
use std::sync::Mutex;
use std::time::Duration;

/// The one format offered: every wl_shm client can take it.
const FORMAT: wl_shm::Format = wl_shm::Format::Xrgb8888;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Comp, ZwlrScreencopyManagerV1, _>(3, ());
}

/// A copy waiting for its screen's next frame.
pub struct Pending {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    damage: bool,
}

/// What a frame captures: which screen, which part of it (in its pixels),
/// and whether with the pointer. `None` when the screen was gone.
pub struct Frame {
    target: Option<(Output, Rectangle<i32, Physical>)>,
    cursor: bool,
    used: Mutex<bool>,
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for Comp {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for Comp {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_manager_v1::Request;
        let (frame, cursor, output, region) = match request {
            Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor != 0, output, None),
            Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (
                frame,
                overlay_cursor != 0,
                output,
                Some((x, y, width, height)),
            ),
            Request::Destroy => return,
            _ => return,
        };
        let target = Output::from_resource(&output).and_then(|o| {
            let r = area(&o, region)?;
            Some((o, r))
        });
        let frame = data_init.init(
            frame,
            Frame {
                target: target.clone(),
                cursor,
                used: Mutex::new(false),
            },
        );
        let Some((_, r)) = target else {
            frame.failed();
            return;
        };
        frame.buffer(
            FORMAT,
            r.size.w as u32,
            r.size.h as u32,
            r.size.w as u32 * 4,
        );
        if frame.version() >= 3 {
            frame.buffer_done();
        }
    }
}

/// The part of `output` asked for, in its pixels: all of it, or a region
/// given in its logical coordinates, cut to the screen. `None` if empty.
fn area(output: &Output, region: Option<(i32, i32, i32, i32)>) -> Option<Rectangle<i32, Physical>> {
    let full = Rectangle::from_size(render::output_size(output));
    let Some((x, y, w, h)) = region else {
        return Some(full);
    };
    let s = render::scale(output).x;
    let px = |v: i32| (f64::from(v) * s).round() as i32;
    let asked = Rectangle::new((px(x), px(y)).into(), (px(w), px(h)).into());
    asked.intersection(full).filter(|r| !r.is_empty())
}

impl Dispatch<ZwlrScreencopyFrameV1, Frame> for Comp {
    fn request(
        comp: &mut Self,
        _: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &Frame,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use zwlr_screencopy_frame_v1::{Error, Request};
        let (buffer, damage) = match request {
            Request::Copy { buffer } => (buffer, false),
            Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        {
            let mut used = data.used.lock().unwrap_or_else(|p| p.into_inner());
            if *used {
                frame.post_error(Error::AlreadyUsed, "frame already copied");
                return;
            }
            *used = true;
        }
        let Some((_, r)) = &data.target else {
            frame.failed();
            return;
        };
        let fits = with_buffer_contents(&buffer, |_, len, b| {
            b.format == FORMAT
                && b.width == r.size.w
                && b.height == r.size.h
                && b.stride >= r.size.w * 4
                && b.offset >= 0
                && (b.offset as usize) + (b.stride as usize) * (b.height as usize) <= len
        });
        match fits {
            Ok(true) => {}
            Ok(false) => {
                frame.post_error(Error::InvalidBuffer, "wrong buffer size or format");
                return;
            }
            // Not shared memory (dmabuf isn't offered): can't fill it.
            Err(_) => {
                frame.failed();
                return;
            }
        }
        comp.copies.push(Pending {
            frame: frame.clone(),
            buffer,
            damage,
        });
        if !damage {
            comp.request_redraw();
        }
    }
}

/// Fill the copies waiting on `output` from the frame just drawn for it.
/// `elements` are that frame's; `with_cursor` says whether they include
/// the pointer, and they're drawn again when a copy wants it otherwise.
pub fn serve(
    comp: &mut Comp,
    r: &mut GlesRenderer,
    output: &Output,
    elements: &[Elem],
    with_cursor: bool,
) {
    comp.copies.retain(|p| p.frame.is_alive());
    let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut comp.copies)
        .into_iter()
        .partition(|p| target(p).is_some_and(|(o, _)| o == output));
    comp.copies = rest;
    if mine.is_empty() {
        return;
    }
    let size = render::output_size(output);
    let s = render::scale(output).x;
    let ground = comp.ground();
    let mut drawn: [Option<Result<Vec<u8>, String>>; 2] = [None, None];
    let time = Duration::from(Clock::<Monotonic>::new().now());
    for p in mine {
        let Some((_, area)) = target(&p) else {
            continue;
        };
        let cursor = p.frame.data::<Frame>().is_some_and(|f| f.cursor);
        let pixels = drawn[usize::from(cursor)].get_or_insert_with(|| {
            if cursor == with_cursor {
                render::read_back(r, elements, size, s, ground)
            } else {
                let again = render::elements(comp, r, output, cursor);
                render::read_back(r, &again, size, s, ground)
            }
        });
        let filled = match pixels {
            Ok(px) => fill(&p.buffer, px, size.w, area),
            Err(e) => Err(e.clone()),
        };
        match filled {
            Ok(()) => {
                p.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
                if p.damage {
                    p.frame.damage(0, 0, area.size.w as u32, area.size.h as u32);
                }
                let secs = time.as_secs();
                p.frame
                    .ready((secs >> 32) as u32, secs as u32, time.subsec_nanos());
            }
            Err(e) => {
                eprintln!("nordlys-comp: screencopy failed: {e}");
                p.frame.failed();
            }
        }
    }
}

fn target(p: &Pending) -> Option<&(Output, Rectangle<i32, Physical>)> {
    p.frame.data::<Frame>()?.target.as_ref()
}

/// Copy `area` of an RGBA frame `width` pixels wide into an XRGB buffer
/// (B, G, R, X in memory).
fn fill(
    buffer: &WlBuffer,
    rgba: &[u8],
    width: i32,
    area: &Rectangle<i32, Physical>,
) -> Result<(), String> {
    with_buffer_contents_mut(buffer, |ptr, len, b| {
        // SAFETY: smithay hands over the pool's mapping, `len` bytes long;
        // the client may write it meanwhile, which only garbles its copy.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        copy_rows(dst, &b, rgba, width, area)
    })
    .map_err(|e| format!("{e:?}"))?
}

fn copy_rows(
    dst: &mut [u8],
    b: &smithay::wayland::shm::BufferData,
    rgba: &[u8],
    width: i32,
    area: &Rectangle<i32, Physical>,
) -> Result<(), String> {
    let (w, h) = (area.size.w as usize, area.size.h as usize);
    let (x0, y0, src_w) = (area.loc.x as usize, area.loc.y as usize, width as usize);
    if rgba.len() < (y0 + h) * src_w * 4 || x0 + w > src_w {
        return Err("short read from the GPU".into());
    }
    for row in 0..h {
        let s = ((y0 + row) * src_w + x0) * 4;
        let d = b.offset as usize + row * b.stride as usize;
        let (src, out) = (
            &rgba[s..s + w * 4],
            dst.get_mut(d..d + w * 4).ok_or("buffer too small")?,
        );
        for (o, i) in out
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(src.as_chunks::<4>().0)
        {
            *o = [i[2], i[1], i[0], 0xff];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::wayland::shm::BufferData;

    #[test]
    fn copies_a_region_into_xrgb_with_its_stride() {
        // A 3x2 frame, each pixel's red value its index.
        let rgba: Vec<u8> = (0..6u8).flat_map(|i| [i, 10, 20, 255]).collect();
        let b = BufferData {
            offset: 4,
            width: 2,
            height: 1,
            stride: 12,
            format: FORMAT,
        };
        let mut dst = vec![0u8; 16];
        let area = Rectangle::new((1, 1).into(), (2, 1).into());
        copy_rows(&mut dst, &b, &rgba, 3, &area).unwrap();
        assert_eq!(&dst[4..12], &[20, 10, 4, 255, 20, 10, 5, 255]);
        assert_eq!(&dst[..4], &[0; 4]);
    }

    #[test]
    fn a_region_past_the_frame_is_refused() {
        let rgba = vec![0u8; 3 * 2 * 4];
        let b = BufferData {
            offset: 0,
            width: 2,
            height: 2,
            stride: 8,
            format: FORMAT,
        };
        let mut dst = vec![0u8; 16];
        let area = Rectangle::new((2, 1).into(), (2, 2).into());
        assert!(copy_rows(&mut dst, &b, &rgba, 3, &area).is_err());
    }
}
