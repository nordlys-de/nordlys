//! The Stage layout model.
//!
//! The stage holds one window, or two side by side ("two-up"). The newest
//! window always takes the centre; the window it replaces goes to the front
//! of the shelf. Ambient windows (chat, music) live on their own rail, so
//! they leave the stage without joining the shelf.
//!
//! This crate is pure logic with no Wayland types, so the compositor, the
//! headless test server and the unit tests all share exactly one copy of it.

use std::collections::{HashMap, VecDeque};
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
}
