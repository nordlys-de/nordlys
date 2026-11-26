//! The Stage layout model.
//!
//! The stage holds one window, or two side by side ("two-up"). The newest
//! window always takes the centre; the window it replaces goes to the front
//! of the shelf. Ambient windows (chat, music) live on their own rail, so
//! they leave the stage without joining the shelf.
//!
//! A [`Desk`] holds one stage per workspace. With more than one screen,
//! each screen shows a workspace of its own and the focused screen's is
//! the active one, which the stage commands act on.
//!
//! This crate is pure logic with no Wayland types, so the compositor, the
//! headless test server and the unit tests all share exactly one copy of it.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::Hash;

/// Where a window waits while it is off the stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Home {
    /// The workspace's own shelf (left side on large screens).
    Shelf,
    /// The ambient rail, shared by every workspace (right side).
    Ambient,
}

#[derive(Debug, Clone)]
pub struct Stage<W> {
    main: Option<W>,
    side: Option<W>,
    shelf: VecDeque<W>,
    homes: HashMap<W, Home>,
}

impl<W: Clone + Eq + Hash> Default for Stage<W> {
    fn default() -> Self {
        Self {
            main: None,
            side: None,
            shelf: VecDeque::new(),
            homes: HashMap::new(),
        }
    }
}

impl<W: Clone + Eq + Hash> Stage<W> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The focused window in the centre.
    pub fn main(&self) -> Option<&W> {
        self.main.as_ref()
    }

    /// The second pane when two-up is on.
    pub fn side(&self) -> Option<&W> {
        self.side.as_ref()
    }

    /// Shelf windows, most recent first. Slot 1 is the first item.
    pub fn shelf(&self) -> impl Iterator<Item = &W> {
        self.shelf.iter()
    }

    pub fn is_two_up(&self) -> bool {
        self.side.is_some()
    }

    pub fn contains(&self, w: &W) -> bool {
        self.homes.contains_key(w)
    }

    pub fn home(&self, w: &W) -> Option<Home> {
        self.homes.get(w).copied()
    }

    /// A window was mapped. Like any newest window, it takes the stage.
    pub fn open(&mut self, w: W, home: Home) {
        self.homes.insert(w.clone(), home);
        self.bring(&w);
    }

    /// A window was closed or unmapped. If it was the main window, the side
    /// pane takes over, or else the most recent shelf window comes back.
    pub fn close(&mut self, w: &W) -> bool {
        if self.homes.remove(w).is_none() {
            return false;
        }
        if self.main.as_ref() == Some(w) {
            self.main = self.side.take().or_else(|| self.shelf.pop_front());
        } else if self.side.as_ref() == Some(w) {
            self.side = None;
        } else {
            self.shelf.retain(|x| x != w);
        }
        true
    }

    /// Put `w` in the centre. Returns false if the window is unknown.
    ///
    /// * Already main: nothing happens.
    /// * The side pane: the panes swap.
    /// * Otherwise: `w` becomes main. In two-up the old main moves to the
    ///   side and the old side goes to the shelf; in single mode the old
    ///   main goes to the shelf.
    pub fn bring(&mut self, w: &W) -> bool {
        if !self.homes.contains_key(w) {
            return false;
        }
        if self.main.as_ref() == Some(w) {
            return true;
        }
        if self.side.as_ref() == Some(w) {
            self.swap();
            return true;
        }
        self.shelf.retain(|x| x != w);
        if self.is_two_up() {
            let old_side = self.side.take();
            self.side = self.main.replace(w.clone());
            if let Some(o) = old_side {
                self.park(o);
            }
        } else if let Some(o) = self.main.replace(w.clone()) {
            self.park(o);
        }
        true
    }

    /// `Super K`: bring back the window that was replaced most recently.
    pub fn previous(&mut self) -> bool {
        match self.shelf.front().cloned() {
            Some(w) => self.bring(&w),
            None => false,
        }
    }

    /// `Super J`: bring the oldest shelf window. Pressing it again walks
    /// through the whole shelf.
    pub fn cycle(&mut self) -> bool {
        match self.shelf.back().cloned() {
            Some(w) => self.bring(&w),
            None => false,
        }
    }

    /// `Super Alt 1–4`: bring shelf slot `n` (1-based).
    pub fn slot(&mut self, n: usize) -> bool {
        match n.checked_sub(1).and_then(|i| self.shelf.get(i)).cloned() {
            Some(w) => self.bring(&w),
            None => false,
        }
    }

    /// `Super T`: turn two-up on (the most recent shelf window joins as the
    /// side pane) or off (the side pane goes back to the shelf).
    pub fn toggle_two_up(&mut self) -> bool {
        if let Some(s) = self.side.take() {
            self.park(s);
            true
        } else if self.main.is_some() {
            match self.shelf.pop_front() {
                Some(n) => {
                    self.side = Some(n);
                    true
                }
                None => false,
            }
        } else {
            false
        }
    }

    /// `Super Tab`: swap main and side.
    pub fn swap(&mut self) -> bool {
        if self.side.is_none() {
            return false;
        }
        std::mem::swap(&mut self.main, &mut self.side);
        true
    }

    /// `Super S`: send the main window to the shelf. The side pane, if any,
    /// takes the centre.
    pub fn send_to_shelf(&mut self) -> bool {
        match self.main.take() {
            Some(m) => {
                self.main = self.side.take();
                self.park(m);
                true
            }
            None => false,
        }
    }

    fn park(&mut self, w: W) {
        if self.homes.get(&w) == Some(&Home::Shelf) {
            self.shelf.push_front(w);
        }
    }
}

/// One stage per workspace, numbered from 1. A window lives on exactly one
/// workspace; the stage commands act on the active one.
///
/// Screens are optional: with none named (headless, or a compositor that
/// keeps screens to itself) there's one implicit screen showing the active
/// workspace. Named, each shows one workspace, no two the same, and the
/// focused one's is the active workspace.
#[derive(Debug, Clone)]
pub struct Desk<W> {
    stages: BTreeMap<u32, Stage<W>>,
    active: u32,
    places: HashMap<W, u32>,
    /// Each screen by name, in order, with the workspace it shows.
    screens: Vec<(String, u32)>,
    /// Index into `screens` of the one with the keyboard.
    focused: usize,
}

impl<W: Clone + Eq + Hash> Default for Desk<W> {
    fn default() -> Self {
        Self {
            stages: BTreeMap::from([(1, Stage::default())]),
            active: 1,
            places: HashMap::new(),
            screens: Vec::new(),
            focused: 0,
        }
    }
}

impl<W: Clone + Eq + Hash> Desk<W> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> u32 {
        self.active
    }

    /// The active workspace's stage.
    pub fn stage(&self) -> &Stage<W> {
        &self.stages[&self.active]
    }

    pub fn stage_mut(&mut self) -> &mut Stage<W> {
        self.stages.entry(self.active).or_default()
    }

    /// Workspaces with windows, plus the ones on screen, in order.
    pub fn workspaces(&self) -> impl Iterator<Item = (u32, &Stage<W>)> {
        self.stages.iter().map(|(n, s)| (*n, s))
    }

    /// Workspace `n`'s stage, if it has one (it's empty otherwise).
    pub fn workspace(&self, n: u32) -> Option<&Stage<W>> {
        self.stages.get(&n)
    }

    /// The screens, in order, each with the workspace it shows.
    pub fn screens(&self) -> impl Iterator<Item = (&str, u32)> {
        self.screens.iter().map(|(name, n)| (name.as_str(), *n))
    }

    /// The screen with the keyboard, when screens are named.
    pub fn focused_screen(&self) -> Option<&str> {
        self.screens
            .get(self.focused)
            .map(|(name, _)| name.as_str())
    }

    /// The screen showing workspace `n`, if any.
    pub fn screen_showing(&self, n: u32) -> Option<&str> {
        self.screens
            .iter()
            .find(|(_, w)| *w == n)
            .map(|(name, _)| name.as_str())
    }

    /// These screens are connected now, in this order. Screens still there
    /// keep their workspace; a new one shows the active workspace if no
    /// screen does, else the lowest one not on screen. A screen that went
    /// away leaves its workspace (and its windows) to be visited again.
    pub fn set_screens(&mut self, names: &[String]) {
        let focused = self.focused_screen().map(str::to_owned);
        let mut next: Vec<(String, u32)> = Vec::new();
        for name in names {
            if next.iter().any(|(n, _)| n == name) {
                continue;
            }
            if let Some(kept) = self.screens.iter().find(|(n, _)| n == name) {
                next.push(kept.clone());
            }
        }
        for name in names {
            if next.iter().any(|(n, _)| n == name) {
                continue;
            }
            let taken = |w: u32| next.iter().any(|(_, n)| *n == w);
            let ws = if taken(self.active) {
                (1..).find(|w| !taken(*w)).expect("a free workspace")
            } else {
                self.active
            };
            next.push((name.clone(), ws));
        }
        // Back in the order given.
        next.sort_by_key(|(n, _)| names.iter().position(|x| x == n));
        self.screens = next;
        self.focused = focused
            .and_then(|f| self.screens.iter().position(|(n, _)| *n == f))
            .unwrap_or(0);
        for (_, n) in &self.screens {
            self.stages.entry(*n).or_default();
        }
        if let Some((_, n)) = self.screens.get(self.focused) {
            self.active = *n;
        }
        self.tidy();
    }

    /// Give the keyboard to screen `name`: its workspace becomes the
    /// active one. False if there's no such screen.
    pub fn focus_screen(&mut self, name: &str) -> bool {
        let Some(i) = self.screens.iter().position(|(n, _)| n == name) else {
            return false;
        };
        self.focused = i;
        self.active = self.screens[i].1;
        self.stages.entry(self.active).or_default();
        self.tidy();
        true
    }

    pub fn contains(&self, w: &W) -> bool {
        self.places.contains_key(w)
    }

    pub fn workspace_of(&self, w: &W) -> Option<u32> {
        self.places.get(w).copied()
    }

    /// Go to workspace `n`. If another screen shows it, that screen gets
    /// the keyboard; otherwise the focused screen shows it. False for 0.
    pub fn switch(&mut self, n: u32) -> bool {
        if n == 0 {
            return false;
        }
        match self.screens.iter().position(|(_, w)| *w == n) {
            Some(i) => self.focused = i,
            None => {
                if let Some(s) = self.screens.get_mut(self.focused) {
                    s.1 = n;
                }
            }
        }
        self.active = n;
        self.stages.entry(n).or_default();
        self.tidy();
        true
    }

    /// A window was mapped on the active workspace.
    pub fn open(&mut self, w: W, home: Home) {
        if let Some(old) = self.places.get(&w).copied()
            && old != self.active
            && let Some(s) = self.stages.get_mut(&old)
        {
            s.close(&w);
        }
        self.places.insert(w.clone(), self.active);
        self.stage_mut().open(w, home);
        self.tidy();
    }

    /// A window went away, from whichever workspace it was on.
    pub fn close(&mut self, w: &W) -> bool {
        let Some(n) = self.places.remove(w) else {
            return false;
        };
        let closed = self.stages.get_mut(&n).is_some_and(|s| s.close(w));
        self.tidy();
        closed
    }

    /// Put `w` on stage, switching to its workspace first.
    pub fn bring(&mut self, w: &W) -> bool {
        match self.places.get(w).copied() {
            Some(n) => {
                self.switch(n);
                self.stage_mut().bring(w)
            }
            None => false,
        }
    }

    /// Send `w` to workspace `n`, where it takes the stage. The active
    /// workspace stays where it is.
    pub fn move_to(&mut self, w: &W, n: u32) -> bool {
        let Some(from) = self.places.get(w).copied() else {
            return false;
        };
        if n == 0 {
            return false;
        }
        if from == n {
            return true;
        }
        let home = self
            .stages
            .get(&from)
            .and_then(|s| s.home(w))
            .unwrap_or(Home::Shelf);
        if let Some(s) = self.stages.get_mut(&from) {
            s.close(w);
        }
        self.stages.entry(n).or_default().open(w.clone(), home);
        self.places.insert(w.clone(), n);
        self.tidy();
        true
    }

    /// Forget empty workspaces other than the active one and those on a
    /// screen. A workspace whose only window is an ambient one off the
    /// stage isn't empty: it keeps that window, so it can be brought back.
    fn tidy(&mut self) {
        let active = self.active;
        let shown: Vec<u32> = self.screens.iter().map(|(_, n)| *n).collect();
        self.stages
            .retain(|n, s| *n == active || shown.contains(n) || !s.homes.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shelf(s: &Stage<&'static str>) -> Vec<&'static str> {
        s.shelf().copied().collect()
    }

    /// helix on stage, shelf: term, docs, files
    fn desk() -> Stage<&'static str> {
        let mut s = Stage::new();
        for w in ["files", "docs", "term", "helix"] {
            s.open(w, Home::Shelf);
        }
        s.open("chat", Home::Ambient);
        s.bring(&"helix");
        s
    }

    #[test]
    fn newest_takes_centre_and_old_goes_to_front_of_shelf() {
        let mut s = desk();
        assert_eq!(s.main(), Some(&"helix"));
        assert_eq!(shelf(&s), ["term", "docs", "files"]);

        s.bring(&"docs");
        assert_eq!(s.main(), Some(&"docs"));
        assert_eq!(shelf(&s), ["helix", "term", "files"]);
    }

    #[test]
    fn ambient_windows_never_join_the_shelf() {
        let mut s = desk();
        s.bring(&"chat");
        assert_eq!(s.main(), Some(&"chat"));
        assert_eq!(shelf(&s), ["helix", "term", "docs", "files"]);

        s.bring(&"term");
        assert_eq!(s.main(), Some(&"term"));
        assert!(!shelf(&s).contains(&"chat"));
    }

    #[test]
    fn two_up_new_main_old_main_to_side_old_side_to_shelf() {
        let mut s = desk();
        assert!(s.toggle_two_up());
        assert_eq!((s.main(), s.side()), (Some(&"helix"), Some(&"term")));

        s.bring(&"files");
        assert_eq!((s.main(), s.side()), (Some(&"files"), Some(&"helix")));
        assert_eq!(shelf(&s), ["term", "docs"]);
    }

    #[test]
    fn bringing_the_side_pane_swaps() {
        let mut s = desk();
        s.toggle_two_up();
        s.bring(&"term");
        assert_eq!((s.main(), s.side()), (Some(&"term"), Some(&"helix")));
    }

    #[test]
    fn two_up_off_returns_side_to_shelf() {
        let mut s = desk();
        s.toggle_two_up();
        s.toggle_two_up();
        assert!(!s.is_two_up());
        assert_eq!(shelf(&s), ["term", "docs", "files"]);
    }

    #[test]
    fn previous_toggles_between_last_two() {
        let mut s = desk();
        s.previous();
        assert_eq!(s.main(), Some(&"term"));
        s.previous();
        assert_eq!(s.main(), Some(&"helix"));
    }

    #[test]
    fn cycle_walks_the_whole_shelf() {
        let mut s = desk();
        let mut seen = vec![];
        for _ in 0..4 {
            s.cycle();
            seen.push(*s.main().unwrap());
        }
        assert_eq!(seen, ["files", "docs", "term", "helix"]);
    }

    #[test]
    fn slot_is_one_based_and_bounds_checked() {
        let mut s = desk();
        assert!(!s.slot(0));
        assert!(!s.slot(9));
        assert!(s.slot(2));
        assert_eq!(s.main(), Some(&"docs"));
    }

    #[test]
    fn send_to_shelf_promotes_side() {
        let mut s = desk();
        s.toggle_two_up();
        s.send_to_shelf();
        assert_eq!((s.main(), s.side()), (Some(&"term"), None));
        assert_eq!(shelf(&s)[0], "helix");
    }

    #[test]
    fn closing_main_brings_back_the_previous_window() {
        let mut s = desk();
        assert!(s.close(&"helix"));
        assert_eq!(s.main(), Some(&"term"));
        assert!(!s.close(&"helix"));
        assert!(!s.bring(&"helix"));
    }

    #[test]
    fn each_workspace_has_its_own_stage() {
        let mut d = Desk::new();
        d.open("helix", Home::Shelf);
        d.open("foot", Home::Shelf);
        assert!(d.switch(2));
        assert_eq!(d.stage().main(), None);
        d.open("firefox", Home::Shelf);
        assert_eq!(d.stage().main(), Some(&"firefox"));
        assert_eq!(d.workspace_of(&"helix"), Some(1));
        let seen: Vec<u32> = d.workspaces().map(|(n, _)| n).collect();
        assert_eq!(seen, [1, 2]);
    }

    #[test]
    fn bring_follows_the_window_to_its_workspace() {
        let mut d = Desk::new();
        d.open("helix", Home::Shelf);
        d.open("foot", Home::Shelf);
        d.switch(3);
        assert!(d.bring(&"helix"));
        assert_eq!(d.active(), 1);
        assert_eq!(d.stage().main(), Some(&"helix"));
        // Workspace 3 was empty and is gone again.
        assert!(d.workspaces().all(|(n, _)| n != 3));
    }

    #[test]
    fn move_to_takes_the_stage_there_and_closing_finds_it() {
        let mut d = Desk::new();
        d.open("helix", Home::Shelf);
        d.open("foot", Home::Shelf);
        assert!(d.move_to(&"foot", 2));
        assert_eq!(d.active(), 1);
        assert_eq!(d.stage().main(), Some(&"helix"));
        assert_eq!(d.workspace_of(&"foot"), Some(2));
        assert!(d.close(&"foot"));
        assert!(!d.contains(&"foot"));
        assert!(d.workspaces().all(|(n, _)| n != 2));
        assert!(!d.switch(0));
        assert!(!d.move_to(&"helix", 0));
    }

    #[test]
    fn reopening_a_window_elsewhere_moves_it() {
        let mut d = Desk::new();
        d.open("helix", Home::Shelf);
        d.switch(2);
        d.open("helix", Home::Shelf);
        assert_eq!(d.workspace_of(&"helix"), Some(2));
        assert!(d.workspaces().all(|(n, _)| n != 1));
    }

    #[test]
    fn an_offstage_ambient_window_keeps_its_workspace() {
        let mut d = Desk::new();
        d.open("chat", Home::Ambient);
        d.open("term", Home::Shelf); // chat leaves the stage, not to the shelf
        d.close(&"term");
        assert_eq!(d.stage().main(), None);
        d.switch(2);
        assert!(d.bring(&"chat"), "chat was lost with its workspace");
        assert_eq!((d.active(), d.stage().main()), (1, Some(&"chat")));
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn each_screen_shows_a_workspace_of_its_own() {
        let mut d: Desk<&str> = Desk::new();
        d.open("helix", Home::Shelf);
        d.set_screens(&names(&["eDP-1", "DP-1"]));
        assert_eq!(d.screens().collect::<Vec<_>>(), [("eDP-1", 1), ("DP-1", 2)]);
        assert_eq!((d.focused_screen(), d.active()), (Some("eDP-1"), 1));

        // The other screen gets the keyboard, and new windows open there.
        assert!(d.focus_screen("DP-1"));
        d.open("firefox", Home::Shelf);
        assert_eq!(d.workspace_of(&"firefox"), Some(2));
        assert_eq!(d.workspace(1).and_then(Stage::main), Some(&"helix"));
        assert!(!d.focus_screen("HDMI-A-1"));
    }

    #[test]
    fn switching_to_a_workspace_on_another_screen_focuses_that_screen() {
        let mut d: Desk<&str> = Desk::new();
        d.set_screens(&names(&["eDP-1", "DP-1"]));
        d.switch(2);
        assert_eq!(d.focused_screen(), Some("DP-1"));
        assert_eq!(d.screens().collect::<Vec<_>>(), [("eDP-1", 1), ("DP-1", 2)]);
        // A workspace no screen shows comes to the focused one.
        d.switch(3);
        assert_eq!(d.screens().collect::<Vec<_>>(), [("eDP-1", 1), ("DP-1", 3)]);
        assert_eq!(d.active(), 3);
    }

    #[test]
    fn an_empty_workspace_on_screen_is_kept() {
        let mut d: Desk<&str> = Desk::new();
        d.set_screens(&names(&["eDP-1", "DP-1"]));
        d.open("helix", Home::Shelf);
        d.close(&"helix");
        assert!(d.workspace(2).is_some(), "DP-1's workspace was dropped");
    }

    #[test]
    fn unplugging_a_screen_keeps_its_windows() {
        let mut d: Desk<&str> = Desk::new();
        d.set_screens(&names(&["eDP-1", "DP-1"]));
        d.focus_screen("DP-1");
        d.open("firefox", Home::Shelf);
        d.set_screens(&names(&["eDP-1"]));
        assert_eq!((d.focused_screen(), d.active()), (Some("eDP-1"), 1));
        assert_eq!(d.workspace_of(&"firefox"), Some(2));
        // Plugged back in, it shows the lowest workspace not on screen.
        d.set_screens(&names(&["eDP-1", "DP-1"]));
        assert_eq!(d.screens().collect::<Vec<_>>(), [("eDP-1", 1), ("DP-1", 2)]);
        assert!(d.bring(&"firefox"));
        assert_eq!(d.focused_screen(), Some("DP-1"));
    }

    #[test]
    fn the_first_screen_shows_the_active_workspace() {
        let mut d: Desk<&str> = Desk::new();
        d.switch(3);
        d.set_screens(&names(&["DP-1"]));
        assert_eq!(d.screens().collect::<Vec<_>>(), [("DP-1", 3)]);
    }
}
