use super::*;
use std::collections::HashSet;

type Win = u8;

#[test]
fn an_empty_stage_refuses_every_command() {
    let mut s: Stage<Win> = Stage::default();
    assert!(!s.previous());
    assert!(!s.cycle());
    assert!(!s.slot(1));
    assert!(!s.toggle_two_up());
    assert!(!s.swap());
    assert!(!s.send_to_shelf());
    assert!(!s.bring(&1));
    assert!(!s.close(&1));
    assert_eq!((s.main(), s.side(), s.shelf().count()), (None, None, 0));
}

#[test]
fn two_up_needs_a_second_window() {
    let mut s = Stage::new();
    s.open(1, Home::Shelf);
    assert!(!s.toggle_two_up());
    assert!(!s.is_two_up());
    assert!(!s.swap());
}

#[test]
fn bringing_the_main_window_changes_nothing() {
    let mut s = Stage::new();
    s.open(1, Home::Shelf);
    s.open(2, Home::Shelf);
    assert!(s.bring(&2));
    assert_eq!(s.main(), Some(&2));
    assert_eq!(s.shelf().copied().collect::<Vec<_>>(), [1]);
}

#[test]
fn swap_trades_the_panes() {
    let mut s = Stage::new();
    s.open(1, Home::Shelf);
    s.open(2, Home::Shelf);
    s.toggle_two_up();
    assert!(s.swap());
    assert_eq!((s.main(), s.side()), (Some(&1), Some(&2)));
}

#[test]
fn closing_the_side_pane_or_a_shelf_window_leaves_the_rest() {
    let mut s = Stage::new();
    for w in 1..=4 {
        s.open(w, Home::Shelf);
    }
    s.toggle_two_up();
    assert_eq!((s.main(), s.side()), (Some(&4), Some(&3)));
    assert!(s.close(&3));
    assert_eq!((s.main(), s.side()), (Some(&4), None));
    assert!(s.close(&1));
    assert_eq!(s.shelf().copied().collect::<Vec<_>>(), [2]);
}

#[test]
fn closing_main_in_two_up_promotes_the_side_pane() {
    let mut s = Stage::new();
    for w in 1..=3 {
        s.open(w, Home::Shelf);
    }
    s.toggle_two_up();
    s.close(&3);
    assert_eq!((s.main(), s.side()), (Some(&2), None));
}

#[test]
fn an_ambient_side_pane_leaves_without_joining_the_shelf() {
    let mut s = Stage::new();
    s.open(1, Home::Shelf);
    s.open(9, Home::Ambient);
    s.toggle_two_up();
    assert_eq!((s.main(), s.side()), (Some(&9), Some(&1)));
    s.swap();
    assert!(s.toggle_two_up());
    assert_eq!((s.main(), s.side(), s.shelf().count()), (Some(&1), None, 0));
    assert!(s.contains(&9));
    assert_eq!(s.home(&9), Some(Home::Ambient));
    assert_eq!(s.home(&1), Some(Home::Shelf));
    assert_eq!(s.home(&5), None);
}

#[test]
fn a_desk_refuses_unknown_windows() {
    let mut d: Desk<Win> = Desk::default();
    assert_eq!(d.active(), 1);
    assert!(!d.bring(&1));
    assert!(!d.close(&1));
    assert!(!d.move_to(&1, 2));
    assert!(!d.contains(&1));
    assert_eq!(d.workspace_of(&1), None);
    assert_eq!(d.workspaces().count(), 1);
}

#[test]
fn moving_a_window_to_its_own_workspace_changes_nothing() {
    let mut d = Desk::new();
    d.open(1, Home::Shelf);
    assert!(d.move_to(&1, 1));
    assert_eq!(d.stage().main(), Some(&1));
}

#[test]
fn an_ambient_window_keeps_its_home_when_moved() {
    let mut d = Desk::new();
    d.open(9, Home::Ambient);
    assert!(d.move_to(&9, 2));
    d.switch(2);
    assert_eq!(d.stage().home(&9), Some(Home::Ambient));
}

/// A workspace whose only remaining window waits off the stage on the
/// ambient rail must not be forgotten: the window is still there.
#[test]
fn an_ambient_window_survives_its_workspace_emptying() {
    let mut d = Desk::new();
    d.open(9, Home::Ambient); // a chat window ...
    d.open(1, Home::Shelf); // ... replaced by another window
    d.switch(2);
    assert!(d.close(&1));
    assert!(d.contains(&9));
    assert!(d.bring(&9), "the chat window can't be reached any more");
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn shown(d: &Desk<Win>) -> Vec<(String, u32)> {
    d.screens().map(|(s, n)| (s.to_owned(), n)).collect()
}

fn on(list: &[(&str, u32)]) -> Vec<(String, u32)> {
    list.iter().map(|(s, n)| ((*s).to_owned(), *n)).collect()
}

#[test]
fn without_named_screens_nothing_is_focused() {
    let mut d: Desk<Win> = Desk::new();
    assert_eq!(d.focused_screen(), None);
    assert_eq!(d.screen_showing(1), None);
    assert_eq!(d.screens().count(), 0);
    assert!(!d.focus_screen("eDP-1"));
    assert!(d.workspace(1).is_some());
    assert!(d.workspace(7).is_none());
}

#[test]
fn a_screen_named_twice_counts_once() {
    let mut d: Desk<Win> = Desk::new();
    d.set_screens(&names(&["A", "B", "A"]));
    assert_eq!(shown(&d), on(&[("A", 1), ("B", 2)]));
}

#[test]
fn a_new_screen_takes_the_active_workspace_when_it_is_off_screen() {
    let mut d: Desk<Win> = Desk::new();
    d.switch(5);
    d.set_screens(&names(&["A", "B"]));
    assert_eq!(shown(&d), on(&[("A", 5), ("B", 1)]));
    assert_eq!((d.active(), d.focused_screen()), (5, Some("A")));
}

#[test]
fn screens_keep_their_workspace_when_reordered() {
    let mut d: Desk<Win> = Desk::new();
    d.set_screens(&names(&["A", "B"]));
    assert!(d.focus_screen("B"));
    d.set_screens(&names(&["B", "A"]));
    assert_eq!(shown(&d), on(&[("B", 2), ("A", 1)]));
    assert_eq!((d.focused_screen(), d.active()), (Some("B"), 2));
}

#[test]
fn when_the_focused_screen_goes_the_first_one_takes_over() {
    let mut d: Desk<Win> = Desk::new();
    d.set_screens(&names(&["A", "B"]));
    d.focus_screen("B");
    d.open(1, Home::Shelf);
    d.set_screens(&names(&["A"]));
    assert_eq!((d.focused_screen(), d.active()), (Some("A"), 1));
    assert_eq!(d.workspace_of(&1), Some(2), "its window waits on 2");
    // All screens gone: the desk carries on with what it had.
    d.set_screens(&[]);
    assert_eq!((d.focused_screen(), d.active()), (None, 1));
    assert!(d.bring(&1));
    assert_eq!(d.active(), 2);
}

#[test]
fn switching_to_a_workspace_on_another_screen_moves_the_keyboard() {
    let mut d: Desk<Win> = Desk::new();
    d.set_screens(&names(&["A", "B"]));
    assert!(d.switch(2));
    assert_eq!(d.focused_screen(), Some("B"));
    assert_eq!(shown(&d), on(&[("A", 1), ("B", 2)]), "nothing moved");
    assert!(d.switch(9));
    assert_eq!(shown(&d), on(&[("A", 1), ("B", 9)]));
    assert_eq!(d.screen_showing(9), Some("B"));
    assert_eq!(d.screen_showing(2), None);
}

#[test]
fn reopening_a_known_window_moves_it_to_the_active_workspace() {
    let mut d: Desk<Win> = Desk::new();
    d.open(1, Home::Shelf);
    d.switch(2);
    d.open(1, Home::Shelf);
    assert_eq!(d.workspace_of(&1), Some(2));
    d.switch(1);
    assert_eq!(d.stage().main(), None);
}

/// A small fixed-seed generator, so a failing run can be repeated.
struct Dice(u64);

impl Dice {
    fn roll(&mut self, sides: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % sides
    }
}

/// What must hold for any stage, whatever happened to it.
fn check_stage(s: &Stage<Win>, step: &str) {
    assert!(
        s.side().is_none() || s.main().is_some(),
        "a side pane with no main window: {step}"
    );
    let placed: Vec<Win> = s
        .main()
        .into_iter()
        .chain(s.side())
        .chain(s.shelf())
        .copied()
        .collect();
    let unique: HashSet<Win> = placed.iter().copied().collect();
    assert_eq!(unique.len(), placed.len(), "a window shows twice: {step}");
    for w in &placed {
        assert!(s.contains(w), "{w} is placed but unknown: {step}");
    }
    for w in s.shelf() {
        assert_eq!(s.home(w), Some(Home::Shelf), "{w} on the shelf: {step}");
    }
    for (w, home) in &s.homes {
        if *home == Home::Shelf {
            assert!(unique.contains(w), "{w} got lost: {step}");
        }
    }
}

#[test]
fn any_sequence_of_stage_commands_keeps_the_stage_consistent() {
    let mut dice = Dice(0x9E37_79B9_7F4A_7C15);
    for run in 0..300 {
        let mut s: Stage<Win> = Stage::new();
        for turn in 0..200 {
            let w = dice.roll(6) as Win;
            let home = if w >= 4 { Home::Ambient } else { Home::Shelf };
            let command = dice.roll(9);
            match command {
                0 => s.open(w, home),
                1 => {
                    let known = s.contains(&w);
                    assert_eq!(s.close(&w), known);
                    assert!(!s.contains(&w));
                }
                2 => {
                    if s.bring(&w) {
                        assert_eq!(s.main(), Some(&w));
                    }
                }
                3 => {
                    s.previous();
                }
                4 => {
                    s.cycle();
                }
                5 => {
                    s.slot(dice.roll(6) as usize);
                }
                6 => {
                    s.toggle_two_up();
                }
                7 => {
                    s.swap();
                }
                _ => {
                    s.send_to_shelf();
                }
            }
            check_stage(&s, &format!("run {run} turn {turn} command {command}"));
        }
    }
}

/// Windows of both kinds, with screens coming, going and taking focus.
#[test]
fn any_sequence_of_desk_commands_keeps_the_desk_consistent() {
    let mut dice = Dice(0x2545_F491_4F6C_DD1D);
    let all = ["A", "B", "C"];
    for run in 0..300 {
        let mut d: Desk<Win> = Desk::new();
        for turn in 0..200 {
            let w = dice.roll(6) as Win;
            let home = if w >= 4 { Home::Ambient } else { Home::Shelf };
            let n = dice.roll(4) as u32;
            let command = dice.roll(9);
            match command {
                0 => d.open(w, home),
                1 => {
                    let known = d.contains(&w);
                    assert_eq!(d.close(&w), known);
                }
                2 => {
                    let known = d.contains(&w);
                    assert_eq!(d.bring(&w), known);
                }
                3 => {
                    assert_eq!(d.switch(n), n != 0);
                }
                4 => {
                    let wanted = d.contains(&w) && n != 0;
                    assert_eq!(d.move_to(&w, n), wanted);
                }
                5 => {
                    d.stage_mut().toggle_two_up();
                }
                6 => {
                    d.stage_mut().send_to_shelf();
                }
                7 => {
                    let some: Vec<&str> =
                        all.iter().copied().filter(|_| dice.roll(2) == 0).collect();
                    d.set_screens(&names(&some));
                    assert_eq!(d.screens().count(), some.len());
                }
                _ => {
                    d.focus_screen(all[dice.roll(3) as usize]);
                }
            }
            let step = format!("run {run} turn {turn} command {command}");
            let mut seen = HashSet::new();
            let on_screen: Vec<u32> = d.screens().map(|(_, n)| n).collect();
            let distinct: HashSet<u32> = on_screen.iter().copied().collect();
            assert_eq!(distinct.len(), on_screen.len(), "shown twice: {step}");
            if let Some(focused) = d.focused_screen() {
                assert_eq!(d.screen_showing(d.active()), Some(focused), "{step}");
            }
            for (n, s) in d.workspaces() {
                check_stage(s, &step);
                assert!(
                    n == d.active() || on_screen.contains(&n) || !s.homes.is_empty(),
                    "an empty workspace lingers: {step}"
                );
                for w in s.homes.keys() {
                    assert!(seen.insert(*w), "{w} is on two workspaces: {step}");
                    assert_eq!(d.workspace_of(w), Some(n), "{step}");
                }
            }
            assert_eq!(seen.len(), d.places.len(), "a window got lost: {step}");
            assert!(d.workspaces().any(|(n, _)| n == d.active()), "{step}");
        }
    }
}
