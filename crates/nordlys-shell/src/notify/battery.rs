//! The low-battery warning. The daemon runs all session, so it's the one to
//! say so.

use super::store::{Request, Urgency};
use crate::sources::Battery;

#[derive(Debug, Default)]
pub struct Warning {
    /// Said once for this run on battery.
    said: bool,
    /// The note that said it, so it can be replaced or taken down.
    pub id: u32,
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Nothing,
    Warn,
    /// On charge or no longer low: take this note down.
    Clear(u32),
}

impl Warning {
    /// What to do about the battery as it is now. `at` is the percentage to
    /// warn at; 0 is off.
    pub fn check(&mut self, battery: &Battery, at: u8) -> Step {
        let low = at > 0 && !battery.charging && battery.percent <= at;
        match (low, self.said) {
            (true, false) => {
                self.said = true;
                Step::Warn
            }
            (false, true) => {
                self.said = false;
                Step::Clear(self.id)
            }
            _ => Step::Nothing,
        }
    }
}

/// Critical, so it stays up and shows through do not disturb.
pub fn request(percent: u8, replaces: u32) -> Request {
    Request {
        app: "Power".into(),
        replaces,
        summary: "Battery low".into(),
        body: format!("{percent}% left. Plug in soon."),
        urgency: Some(Urgency::Critical),
        timeout: -1,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(percent: u8, charging: bool) -> Battery {
        Battery { percent, charging }
    }

    #[test]
    fn warns_once_each_time_the_battery_runs_low() {
        let mut w = Warning::default();
        assert_eq!(w.check(&at(16, false), 15), Step::Nothing);
        assert_eq!(w.check(&at(15, false), 15), Step::Warn);
        w.id = 7;
        assert_eq!(w.check(&at(14, false), 15), Step::Nothing);
        assert_eq!(w.check(&at(14, true), 15), Step::Clear(7));
        assert_eq!(w.check(&at(14, true), 15), Step::Nothing);
        assert_eq!(w.check(&at(14, false), 15), Step::Warn);
        // Turning the warning off takes it down.
        assert_eq!(w.check(&at(14, false), 0), Step::Clear(7));
        assert_eq!(w.check(&at(1, false), 0), Step::Nothing);
    }
}
