//! What the notification daemon knows: toasts on screen, the history
//! behind them and Do not disturb. Pure logic, so it can be tested without
//! a bus or a screen.

use std::time::{Duration, Instant};

/// How long a normal toast stays when the sender doesn't say.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);
/// At most this many toasts are on screen; older ones go to history.
pub const MAX_SHOWN: usize = 4;
const MAX_HISTORY: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

impl Urgency {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }
}

/// Why a toast went away, as numbered by the notification spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closed {
    Expired = 1,
    Dismissed = 2,
    Called = 3,
}

/// A Notify call, already cleaned up.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub app: String,
    pub replaces: u32,
    pub summary: String,
    pub body: String,
    /// (key, label) pairs, in the order the sender gave them.
    pub actions: Vec<(String, String)>,
    pub urgency: Option<Urgency>,
    /// Milliseconds; -1 means "server default", 0 means "never".
    pub timeout: i32,
    /// Don't keep it in history.
    pub transient: bool,
}

#[derive(Debug, Clone)]
pub struct Note {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    pub received: Instant,
    /// How long it stays on screen each time it's shown; None means until dismissed.
    pub lifetime: Option<Duration>,
    pub deadline: Option<Instant>,
    /// Arrived during Do not disturb and hasn't been shown yet.
    pub waiting: bool,
    pub transient: bool,
}

#[derive(Debug, Default)]
pub struct Store {
    next: u32,
    /// On screen, oldest first.
    pub shown: Vec<Note>,
    /// Gone from screen, oldest first.
    pub history: Vec<Note>,
    pub dnd: bool,
}

impl Store {
    /// Take a notification; returns its id.
    pub fn notify(&mut self, req: Request, now: Instant) -> u32 {
        let id = if req.replaces != 0 && self.take(req.replaces).is_some() {
            req.replaces
        } else {
            self.next = self.next.wrapping_add(1).max(1);
            self.next
        };
        let urgency = req.urgency.unwrap_or(Urgency::Normal);
        let lifetime = match (req.timeout, urgency) {
            (0, _) | (-1, Urgency::Critical) => None,
            (t, _) if t < 0 => Some(DEFAULT_TIMEOUT),
            (t, _) => Some(Duration::from_millis(t as u64)),
        };
        let mut note = Note {
            id,
            app: req.app,
            summary: req.summary,
            body: req.body,
            actions: req.actions,
            urgency,
            received: now,
            lifetime,
            deadline: None,
            waiting: false,
            transient: req.transient,
        };
        if self.dnd && urgency != Urgency::Critical {
            note.waiting = true;
            self.archive(note);
        } else {
            self.show(note, now);
        }
        id
    }

    fn show(&mut self, mut note: Note, now: Instant) {
        note.waiting = false;
        note.deadline = note.lifetime.map(|l| now + l);
        self.shown.push(note);
        while self.shown.len() > MAX_SHOWN {
            let old = self.shown.remove(0);
            self.archive(old);
        }
    }

    fn archive(&mut self, note: Note) {
        if note.transient && !note.waiting {
            return;
        }
        self.history.push(note);
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
    }

    /// Remove a note wherever it is.
    fn take(&mut self, id: u32) -> Option<Note> {
        if let Some(i) = self.shown.iter().position(|n| n.id == id) {
            return Some(self.shown.remove(i));
        }
        let i = self.history.iter().position(|n| n.id == id)?;
        Some(self.history.remove(i))
    }

    /// Take a toast off screen. True if it was on screen, so a
    /// NotificationClosed signal is due.
    pub fn close(&mut self, id: u32, reason: Closed) -> bool {
        let Some(i) = self.shown.iter().position(|n| n.id == id) else {
            if reason == Closed::Called {
                self.history.retain(|n| n.id != id);
            }
            return false;
        };
        let note = self.shown.remove(i);
        if reason != Closed::Called {
            self.archive(note);
        }
        true
    }

    /// Toasts whose time is up, except the one under the pointer.
    pub fn expire(&mut self, now: Instant, hovered: Option<u32>) -> Vec<u32> {
        let due: Vec<u32> = self
            .shown
            .iter()
            .filter(|n| Some(n.id) != hovered && n.deadline.is_some_and(|d| d <= now))
            .map(|n| n.id)
            .collect();
        for &id in &due {
            self.close(id, Closed::Expired);
        }
        due
    }

    /// Give a toast a little more time, e.g. after the pointer leaves it.
    pub fn extend(&mut self, id: u32, now: Instant, by: Duration) {
        if let Some(n) = self.shown.iter_mut().find(|n| n.id == id)
            && let Some(d) = n.deadline.as_mut()
        {
            *d = (*d).max(now + by);
        }
    }

    pub fn set_dnd(&mut self, on: bool, now: Instant) {
        self.dnd = on;
        if !on {
            // Show what waited, newest last.
            let (waiting, rest): (Vec<Note>, Vec<Note>) =
                self.history.drain(..).partition(|n| n.waiting);
            self.history = rest;
            for n in waiting {
                self.show(n, now);
            }
        }
    }

    /// Bring the most recent history entry back on screen.
    pub fn show_last(&mut self, now: Instant) -> bool {
        match self.history.pop() {
            Some(n) => {
                self.show(n, now);
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// (waiting, in history, Do not disturb)
    pub fn counts(&self) -> (u32, u32, bool) {
        let waiting = self.history.iter().filter(|n| n.waiting).count() as u32;
        (waiting, self.history.len() as u32, self.dnd)
    }

    /// Newest first, on screen and in history.
    pub fn recent(&self, n: usize) -> Vec<&Note> {
        let mut all: Vec<&Note> = self.shown.iter().chain(self.history.iter()).collect();
        all.sort_by_key(|n| std::cmp::Reverse(n.id));
        all.truncate(n);
        all
    }
}

/// Senders often use a little HTML even when not asked to. Keep the text.
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(summary: &str) -> Request {
        Request {
            app: "test".into(),
            summary: summary.into(),
            timeout: -1,
            ..Request::default()
        }
    }

    #[test]
    fn ids_and_replacing() {
        let mut s = Store::default();
        let t = Instant::now();
        let a = s.notify(req("a"), t);
        let b = s.notify(req("b"), t);
        assert_eq!((a, b), (1, 2));
        let again = s.notify(
            Request {
                replaces: a,
                ..req("a2")
            },
            t,
        );
        assert_eq!(again, a);
        assert_eq!(s.shown.len(), 2);
        assert_eq!(s.shown.last().unwrap().summary, "a2");
        // Replacing an id that isn't there gets a fresh one.
        let c = s.notify(
            Request {
                replaces: 99,
                ..req("c")
            },
            t,
        );
        assert_eq!(c, 3);
    }

    #[test]
    fn timeouts_and_expiry() {
        let mut s = Store::default();
        let t = Instant::now();
        let normal = s.notify(req("normal"), t);
        let sticky = s.notify(
            Request {
                timeout: 0,
                ..req("sticky")
            },
            t,
        );
        let critical = s.notify(
            Request {
                urgency: Some(Urgency::Critical),
                ..req("critical")
            },
            t,
        );
        let later = t + DEFAULT_TIMEOUT + Duration::from_millis(1);
        // Hovering holds it.
        assert!(s.expire(later, Some(normal)).is_empty());
        assert_eq!(s.expire(later, None), vec![normal]);
        assert_eq!(
            s.shown.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![sticky, critical]
        );
        assert_eq!(s.history.len(), 1);
    }

    #[test]
    fn close_reasons() {
        let mut s = Store::default();
        let t = Instant::now();
        let a = s.notify(req("a"), t);
        let b = s.notify(req("b"), t);
        assert!(s.close(a, Closed::Dismissed));
        assert!(s.close(b, Closed::Called));
        assert!(!s.close(b, Closed::Called));
        // Dismissed ones stay in history; ones the app closed don't.
        assert_eq!(s.history.iter().map(|n| n.id).collect::<Vec<_>>(), vec![a]);
    }

    #[test]
    fn do_not_disturb_holds_then_shows() {
        let mut s = Store::default();
        let t = Instant::now();
        s.set_dnd(true, t);
        s.notify(req("quiet"), t);
        s.notify(
            Request {
                urgency: Some(Urgency::Critical),
                ..req("loud")
            },
            t,
        );
        assert_eq!(s.shown.len(), 1);
        assert_eq!(s.counts(), (1, 1, true));
        s.set_dnd(false, t);
        assert_eq!(s.shown.len(), 2);
        assert_eq!(s.counts(), (0, 0, false));
    }

    #[test]
    fn overflow_history_and_recall() {
        let mut s = Store::default();
        let t = Instant::now();
        for i in 0..6 {
            s.notify(req(&format!("n{i}")), t);
        }
        assert_eq!(s.shown.len(), MAX_SHOWN);
        assert_eq!(s.history.len(), 2);
        assert_eq!(
            s.recent(3)
                .iter()
                .map(|n| n.summary.as_str())
                .collect::<Vec<_>>(),
            vec!["n5", "n4", "n3"]
        );
        let first = s.shown[0].id;
        s.close(first, Closed::Dismissed);
        assert!(s.show_last(t));
        assert_eq!(s.shown.last().unwrap().id, first);
        s.clear();
        assert_eq!(s.counts(), (0, 0, false));
    }

    #[test]
    fn transient_skips_history() {
        let mut s = Store::default();
        let t = Instant::now();
        let id = s.notify(
            Request {
                transient: true,
                ..req("volume")
            },
            t,
        );
        s.close(id, Closed::Dismissed);
        assert!(s.history.is_empty());
    }

    #[test]
    fn markup_is_stripped() {
        assert_eq!(plain("<b>Build</b> &amp; test"), "Build & test");
        assert_eq!(
            plain("a &lt;tag&gt; <a href=\"x\">link</a>"),
            "a <tag> link"
        );
    }
}
