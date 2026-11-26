//! One module per page. Each keeps its own state, reads what it shows when
//! it's opened (and every few seconds while it's open, if that changes on
//! its own), and applies changes as they're made: there's no Apply button.

pub mod about;
pub mod advanced;
pub mod appearance;
pub mod bluetooth;
pub mod datetime;
pub mod default_apps;
pub mod displays;
pub mod groups;
pub mod keyboard;
pub mod lock;
pub mod mouse;
pub mod network;
pub mod notifications;
pub mod power;
pub mod services;
pub mod sound;
pub mod startup;
pub mod windows;

use crate::app::Ctx;
use crate::store::V;
use crate::ui::El;
use iced::Task;

pub trait PageState: Default {
    type Msg: Clone + std::fmt::Debug + Send + 'static;

    /// The page came into view.
    fn open(&mut self, _ctx: &mut Ctx) -> Task<Self::Msg> {
        Task::none()
    }

    /// Every few seconds while in view.
    fn tick(&mut self, _ctx: &mut Ctx) -> Task<Self::Msg> {
        Task::none()
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut Ctx) -> Task<Self::Msg>;

    fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Self::Msg>;

    /// A question over the whole window, when the page has one open.
    fn dialog<'a>(&'a self, _ctx: &'a Ctx) -> Option<El<'a, Self::Msg>> {
        None
    }
}

/// Set a Hyprland option and say if Hyprland refused it.
pub fn hypr_set(ctx: &mut Ctx, key: &str, v: V) {
    if let Err(e) = ctx.hypr.set(key, v) {
        ctx.fail(e);
    }
}

/// Set a nordlys.toml key and say if it was refused.
pub fn toml_set(ctx: &mut Ctx, table: &str, key: &str, v: impl Into<toml_edit::Value>) {
    match ctx.toml.set(table, key, v) {
        Ok(()) => ctx.restyle(),
        Err(e) => ctx.fail(e),
    }
}

/// A background read that's folded: one at a time, the latest wins.
#[derive(Debug, Default)]
pub struct Reads {
    busy: bool,
}

impl Reads {
    pub fn start(&mut self) -> bool {
        !std::mem::replace(&mut self.busy, true)
    }
    pub fn done(&mut self) {
        self.busy = false;
    }
}
