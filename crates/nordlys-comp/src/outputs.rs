//! wlr-output-management: how wlr-randr, kanshi, nwg-displays and
//! wdisplays see the screens and arrange them. Smithay has no
//! implementation of it, so this is one.
//!
//! Each client gets a head per screen, with its modes, and the state it's
//! in. A client asks for a change by building a configuration against the
//! last state it was sent (the serial) and applying it; the backend then
//! switches modes, moves screens and changes their scale. Turning a screen
//! off, rotating it and fractional scales aren't supported yet, so a
//! configuration asking for them fails and nothing changes.

use crate::comp::Comp;
use smithay::output::{Mode, Output, Scale};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use smithay::reexports::wayland_server::protocol::wl_output::Transform;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use smithay::utils::{Logical, Point};
use std::sync::{Arc, Mutex};

/// Version 3: version 4 only adds adaptive sync, which isn't offered.
const VERSION: u32 = 3;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Comp, ZwlrOutputManagerV1, _>(VERSION, ());
}

/// What the screens are set to, as last told to the clients.
#[derive(Default)]
pub struct Managers {
    list: Vec<Manager>,
    serial: u32,
    last: Vec<(Output, Shown)>,
}

struct Manager {
    res: ZwlrOutputManagerV1,
    heads: Vec<Head>,
}

struct Head {
    output: Output,
    res: ZwlrOutputHeadV1,
    modes: Vec<(Mode, ZwlrOutputModeV1)>,
    shown: Shown,
}

/// The part of a screen's state that changes.
#[derive(Clone, PartialEq, Debug)]
struct Shown {
    mode: Option<Mode>,
    position: Point<i32, Logical>,
    scale: f64,
}

fn shown(o: &Output) -> Shown {
    Shown {
        mode: o.current_mode(),
        position: o.current_location(),
        scale: o.current_scale().fractional_scale(),
    }
}

/// One screen as a configuration wants it.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub output: Output,
    pub mode: Mode,
    pub position: Point<i32, Logical>,
    pub scale: i32,
}

/// What a client set on one head of a configuration.
#[derive(Default)]
pub struct HeadChange {
    mode: Option<Mode>,
    custom: Option<(i32, i32, i32)>,
    position: Option<Point<i32, Logical>>,
    transform: Option<Transform>,
    scale: Option<f64>,
}

/// Each head a configuration names, with what to set: `None` to turn it
/// off.
type Named = (Output, Option<Arc<Mutex<HeadChange>>>);

pub struct Config {
    serial: u32,
    /// Each head the configuration names: `None` to turn it off.
    heads: Mutex<Vec<Named>>,
    used: Mutex<bool>,
}

/// Tell every client what changed about the screens: ones that came, went
/// or changed mode, place or scale.
pub fn refresh(comp: &mut Comp) {
    let now: Vec<(Output, Shown)> = comp.outputs.iter().map(|o| (o.clone(), shown(o))).collect();
    if now == comp.output_managers.last {
        return;
    }
    comp.output_managers.last = now;
    comp.output_managers.serial = comp.output_managers.serial.wrapping_add(1);
    let serial = comp.output_managers.serial;
    let outputs = comp.outputs.clone();
    let dh = comp.dh.clone();
    for m in &mut comp.output_managers.list {
        m.heads.retain(|h| {
            // A screen whose modes changed is announced again, afresh.
            let same = h.modes.iter().map(|(x, _)| *x).eq(h.output.modes());
            let keep = outputs.contains(&h.output) && same;
            if !keep {
                for (_, mode) in &h.modes {
                    mode.finished();
                }
                h.res.finished();
            }
            keep && h.res.is_alive()
        });
        for h in &mut m.heads {
            let now = shown(&h.output);
            if now != h.shown {
                send_state(h, &now);
                h.shown = now;
            }
        }
        for o in &outputs {
            if !m.heads.iter().any(|h| h.output == *o)
                && let Some(h) = new_head(&dh, &m.res, o)
            {
                m.heads.push(h);
            }
        }
        m.res.done(serial);
    }
}

/// Announce `output` to the client of `manager`, with everything about it.
fn new_head(dh: &DisplayHandle, manager: &ZwlrOutputManagerV1, output: &Output) -> Option<Head> {
    let client = manager.client()?;
    let v = manager.version();
    let res = client
        .create_resource::<ZwlrOutputHeadV1, _, Comp>(dh, v, output.downgrade())
        .ok()?;
    manager.head(&res);
    res.name(output.name());
    let p = output.physical_properties();
    res.description(format!("{} {} ({})", p.make, p.model, output.name()));
    if p.size.w > 0 && p.size.h > 0 {
        res.physical_size(p.size.w, p.size.h);
    }
    let preferred = output.preferred_mode();
    let mut modes = Vec::new();
    for m in output.modes() {
        let Ok(mode) = client.create_resource::<ZwlrOutputModeV1, _, Comp>(dh, v, m) else {
            continue;
        };
        res.mode(&mode);
        mode.size(m.size.w, m.size.h);
        if m.refresh > 0 {
            mode.refresh(m.refresh);
        }
        if preferred == Some(m) {
            mode.preferred();
        }
        modes.push((m, mode));
    }
    res.enabled(1);
    if v >= 2 {
        res.make(p.make.clone());
        res.model(p.model.clone());
    }
    let mut head = Head {
        output: output.clone(),
        res,
        modes,
        shown: shown(output),
    };
    let now = head.shown.clone();
    send_state(&mut head, &now);
    Some(head)
}

fn send_state(h: &mut Head, s: &Shown) {
    if let Some(m) = s.mode
        && let Some((_, res)) = h.modes.iter().find(|(x, _)| *x == m)
    {
        h.res.current_mode(res);
    }
    h.res.position(s.position.x, s.position.y);
    h.res.transform(Transform::Normal);
    h.res.scale(s.scale);
}

/// What `heads` asks for, checked against what can be done: a change for
/// every screen, or why it can't be.
fn plan(outputs: &[Output], heads: &[Named]) -> Result<Vec<Change>, String> {
    let mut changes = Vec::new();
    for (o, h) in heads {
        if !outputs.contains(o) {
            return Err(format!("{} is gone", o.name()));
        }
        let Some(h) = h else {
            return Err(format!("turning {} off isn't supported yet", o.name()));
        };
        let h = h.lock().unwrap_or_else(|p| p.into_inner());
        if h.transform.is_some_and(|t| t != Transform::Normal) {
            return Err("rotating a screen isn't supported yet".into());
        }
        let mode = match (h.mode, h.custom) {
            (Some(m), _) => m,
            (None, Some((w, hh, r))) => o
                .modes()
                .into_iter()
                .find(|m| m.size == (w, hh).into() && (r == 0 || (m.refresh - r).abs() <= 500))
                .ok_or_else(|| format!("{} has no {w}x{hh} mode", o.name()))?,
            (None, None) => o.current_mode().ok_or("the screen has no mode")?,
        };
        let scale = h
            .scale
            .unwrap_or_else(|| o.current_scale().fractional_scale());
        if scale.fract() != 0.0 || !(1.0..=4.0).contains(&scale) {
            return Err(format!("scale {scale}: only 1, 2, 3 or 4 for now"));
        }
        changes.push(Change {
            output: o.clone(),
            mode,
            position: h.position.unwrap_or_else(|| o.current_location()),
            scale: scale as i32,
        });
    }
    Ok(changes)
}

impl GlobalDispatch<ZwlrOutputManagerV1, ()> for Comp {
    fn bind(
        comp: &mut Self,
        dh: &DisplayHandle,
        _: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let res = data_init.init(resource, ());
        let mut m = Manager {
            res: res.clone(),
            heads: Vec::new(),
        };
        for o in &comp.outputs {
            if let Some(h) = new_head(dh, &res, o) {
                m.heads.push(h);
            }
        }
        res.done(comp.output_managers.serial);
        comp.output_managers.list.push(m);
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for Comp {
    fn request(
        comp: &mut Self,
        _: &Client,
        manager: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_manager_v1::Request;
        match request {
            Request::CreateConfiguration { id, serial } => {
                data_init.init(
                    id,
                    Config {
                        serial,
                        heads: Mutex::new(Vec::new()),
                        used: Mutex::new(false),
                    },
                );
            }
            Request::Stop => {
                comp.output_managers.list.retain(|m| m.res != *manager);
                manager.finished();
            }
            _ => {}
        }
    }

    fn destroyed(
        comp: &mut Self,
        _: smithay::reexports::wayland_server::backend::ClientId,
        manager: &ZwlrOutputManagerV1,
        _: &(),
    ) {
        comp.output_managers.list.retain(|m| m.res != *manager);
    }
}

impl Dispatch<ZwlrOutputHeadV1, smithay::output::WeakOutput> for Comp {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwlrOutputHeadV1,
        _: zwlr_output_head_v1::Request,
        _: &smithay::output::WeakOutput,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // Only `release`, a destructor.
    }
}

impl Dispatch<ZwlrOutputModeV1, Mode> for Comp {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwlrOutputModeV1,
        _: zwlr_output_mode_v1::Request,
        _: &Mode,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // Only `release`, a destructor.
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, Config> for Comp {
    fn request(
        comp: &mut Self,
        _: &Client,
        config: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        data: &Config,
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_configuration_v1::{Error, Request};
        let head_output = |h: &ZwlrOutputHeadV1| {
            h.data::<smithay::output::WeakOutput>()
                .and_then(|w| w.upgrade())
        };
        let mut heads = data.heads.lock().unwrap_or_else(|p| p.into_inner());
        match request {
            Request::EnableHead { id, head } => {
                let change = Arc::new(Mutex::new(HeadChange::default()));
                data_init.init(id, change.clone());
                let Some(o) = head_output(&head) else {
                    // The screen went: the serial is stale, so applying
                    // this gets `cancelled`.
                    return;
                };
                if heads.iter().any(|(x, _)| *x == o) {
                    config.post_error(Error::AlreadyConfiguredHead, "head configured twice");
                    return;
                }
                heads.push((o, Some(change)));
            }
            Request::DisableHead { head } => {
                let Some(o) = head_output(&head) else {
                    return;
                };
                if heads.iter().any(|(x, _)| *x == o) {
                    config.post_error(Error::AlreadyConfiguredHead, "head configured twice");
                    return;
                }
                heads.push((o, None));
            }
            Request::Apply | Request::Test => {
                let apply = matches!(request, Request::Apply);
                {
                    let mut used = data.used.lock().unwrap_or_else(|p| p.into_inner());
                    if *used {
                        config.post_error(Error::AlreadyUsed, "configuration already used");
                        return;
                    }
                    *used = true;
                }
                if data.serial != comp.output_managers.serial {
                    config.cancelled();
                    return;
                }
                if comp
                    .outputs
                    .iter()
                    .any(|o| !heads.iter().any(|(x, _)| x == o))
                {
                    config.post_error(Error::UnconfiguredHead, "every head must be configured");
                    return;
                }
                let result = plan(&comp.outputs, &heads).and_then(|changes| {
                    if apply {
                        comp.configure_outputs(&changes)
                    } else {
                        Ok(())
                    }
                });
                match result {
                    Ok(()) => config.succeeded(),
                    Err(e) => {
                        eprintln!("nordlys-comp: screen configuration refused: {e}");
                        config.failed();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, Arc<Mutex<HeadChange>>> for Comp {
    fn request(
        _: &mut Self,
        _: &Client,
        res: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        data: &Arc<Mutex<HeadChange>>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_configuration_head_v1::{Error, Request};
        let mut h = data.lock().unwrap_or_else(|p| p.into_inner());
        let twice = || res.post_error(Error::AlreadySet, "set twice");
        match request {
            Request::SetMode { mode } => {
                if h.mode.is_some() || h.custom.is_some() {
                    return twice();
                }
                match mode.data::<Mode>() {
                    Some(m) => h.mode = Some(*m),
                    None => res.post_error(Error::InvalidMode, "unknown mode"),
                }
            }
            Request::SetCustomMode {
                width,
                height,
                refresh,
            } => {
                if h.mode.is_some() || h.custom.is_some() {
                    return twice();
                }
                if width <= 0 || height <= 0 || refresh < 0 {
                    return res.post_error(Error::InvalidCustomMode, "bad custom mode");
                }
                h.custom = Some((width, height, refresh));
            }
            Request::SetPosition { x, y } => {
                if h.position.is_some() {
                    return twice();
                }
                h.position = Some((x, y).into());
            }
            Request::SetTransform { transform } => {
                if h.transform.is_some() {
                    return twice();
                }
                match transform {
                    WEnum::Value(t) => h.transform = Some(t),
                    WEnum::Unknown(_) => {
                        res.post_error(Error::InvalidTransform, "unknown transform")
                    }
                }
            }
            Request::SetScale { scale } => {
                if h.scale.is_some() {
                    return twice();
                }
                if scale <= 0.0 {
                    return res.post_error(Error::InvalidScale, "scale must be positive");
                }
                h.scale = Some(scale);
            }
            _ => {}
        }
    }
}

/// The scale the backend should set for a change.
pub fn scale_of(c: &Change) -> Scale {
    Scale::Integer(c.scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::output::{PhysicalProperties, Subpixel};

    fn screen(name: &str) -> Output {
        let o = Output::new(
            name.into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Test".into(),
                model: name.into(),
            },
        );
        let a = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        let b = Mode {
            size: (1280, 720).into(),
            refresh: 59_940,
        };
        o.add_mode(a);
        o.add_mode(b);
        o.change_current_state(Some(a), None, Some(Scale::Integer(1)), Some((0, 0).into()));
        o
    }

    fn change(f: impl FnOnce(&mut HeadChange)) -> Option<Arc<Mutex<HeadChange>>> {
        let mut h = HeadChange::default();
        f(&mut h);
        Some(Arc::new(Mutex::new(h)))
    }

    #[test]
    fn unset_parts_keep_what_the_screen_has() {
        let o = screen("DP-1");
        let got = plan(
            std::slice::from_ref(&o),
            &[(o.clone(), change(|h| h.position = Some((100, 50).into())))],
        )
        .unwrap();
        assert_eq!(got[0].mode, o.current_mode().unwrap());
        assert_eq!(got[0].position, (100, 50).into());
        assert_eq!(got[0].scale, 1);
    }

    #[test]
    fn a_custom_mode_matches_one_the_screen_has() {
        let o = screen("DP-1");
        let got = plan(
            std::slice::from_ref(&o),
            &[(o.clone(), change(|h| h.custom = Some((1280, 720, 60_000))))],
        )
        .unwrap();
        assert_eq!(got[0].mode.size, (1280, 720).into());
        assert!(
            plan(
                std::slice::from_ref(&o),
                &[(o.clone(), change(|h| h.custom = Some((800, 600, 0))))]
            )
            .is_err()
        );
    }

    #[test]
    fn what_isnt_supported_is_refused() {
        let o = screen("DP-1");
        let os = [o.clone()];
        assert!(plan(&os, &[(o.clone(), None)]).is_err());
        assert!(plan(&os, &[(o.clone(), change(|h| h.scale = Some(1.5)))]).is_err());
        assert!(
            plan(
                &os,
                &[(o.clone(), change(|h| h.transform = Some(Transform::_90)))]
            )
            .is_err()
        );
        assert_eq!(
            plan(&os, &[(o, change(|h| h.scale = Some(2.0)))]).unwrap()[0].scale,
            2
        );
    }
}
