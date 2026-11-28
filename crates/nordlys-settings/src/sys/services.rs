//! The session's services (systemd user units) and the system's failed ones.

use super::{SLOW, run, run_for};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Services {
    pub systemd: bool,
    pub session: Vec<Unit>,
    pub failed_system: Vec<Unit>,
    pub failed_user: Vec<Unit>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Unit {
    pub name: String,
    pub description: String,
    pub active: String,
    pub sub: String,
    /// Masked: systemd won't start it, so it stays off at the next login.
    pub masked: bool,
}

impl Unit {
    /// Can "Start at login" be turned off for it? The session's own
    /// services can. They have no `[Install]` section (the session target
    /// starts them), so off means masked, not disabled.
    pub fn optional(&self) -> bool {
        self.name.starts_with("nordlys-") && self.name.ends_with(".service")
    }
    pub fn running(&self) -> bool {
        self.active == "active"
    }
    pub fn failed(&self) -> bool {
        self.active == "failed"
    }
    /// `nordlys-shell@bar.service` → `Bar`.
    pub fn short(&self) -> String {
        let n = self
            .name
            .trim_end_matches(".service")
            .trim_end_matches(".target");
        let n = n.strip_prefix("nordlys-shell@").unwrap_or(n);
        let n = n.strip_prefix("nordlys-").unwrap_or(n);
        let mut c = n.chars();
        match c.next() {
            Some(f) => f
                .to_uppercase()
                .chain(c)
                .collect::<String>()
                .replace('-', " "),
            None => String::new(),
        }
    }
}

/// What each piece of the session is for.
pub fn purpose(name: &str) -> &'static str {
    match name {
        "nordlys-shell@bar.service" => "Top bar with workspaces, clock and battery",
        "nordlys-shell@shelf.service" => "Left rail with the windows waiting on the shelf",
        "nordlys-shell@close-button.service" => "The × that closes the window under the pointer",
        "nordlys-notify.service" => "Notification daemon and toasts",
        "nordlys-idle.service" => "Dims, locks and suspends when you're away",
        "nordlys-wallpaper.service" => "Draws the wallpaper",
        "nordlys-clipboard.service" => "Keeps clipboard history for Super V",
        "nordlys-session.target" => "Everything a Nordlys session runs",
        _ => "",
    }
}

pub fn read() -> Result<Services, String> {
    if !super::has("systemctl") {
        return Ok(Services::default());
    }
    let mut session = parse_units(
        &run(
            "systemctl",
            &[
                "--user",
                "list-units",
                "--all",
                "--plain",
                "--no-legend",
                "--no-pager",
                "nordlys-*",
            ],
        )
        .unwrap_or_default(),
    );
    // A masked unit can drop out of list-units, which would leave no way to
    // turn it back on; the unit files still name it.
    let masked = parse_masked(
        &run(
            "systemctl",
            &[
                "--user",
                "list-unit-files",
                "--state=masked,masked-runtime",
                "--plain",
                "--no-legend",
                "--no-pager",
                "nordlys-*",
            ],
        )
        .unwrap_or_default(),
    );
    mark_masked(&mut session, &masked);
    let failed = |user: bool| {
        let mut args = vec![
            "list-units",
            "--failed",
            "--plain",
            "--no-legend",
            "--no-pager",
        ];
        if user {
            args.insert(0, "--user");
        }
        parse_units(&run("systemctl", &args).unwrap_or_default())
    };
    Ok(Services {
        systemd: true,
        session,
        failed_system: failed(false),
        failed_user: failed(true)
            .into_iter()
            .filter(|u| !u.name.starts_with("nordlys-"))
            .collect(),
    })
}

/// `name.service loaded active running Description words`
fn parse_units(s: &str) -> Vec<Unit> {
    s.lines()
        .filter_map(|l| {
            let mut f = l.trim_start_matches(['●', '*', ' ']).split_whitespace();
            let name = f.next()?.to_owned();
            let _load = f.next()?;
            let active = f.next()?.to_owned();
            let sub = f.next()?.to_owned();
            let description = f.collect::<Vec<_>>().join(" ");
            Some(Unit {
                name,
                description,
                active,
                sub,
                masked: false,
            })
        })
        .collect()
}

/// `name.service masked enabled`, one unit file a line.
fn parse_masked(s: &str) -> Vec<String> {
    s.lines()
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect()
}

/// Flag the masked units, adding any that list-units left out.
fn mark_masked(session: &mut Vec<Unit>, masked: &[String]) {
    for name in masked {
        match session.iter_mut().find(|u| &u.name == name) {
            Some(u) => u.masked = true,
            None => session.push(Unit {
                name: name.clone(),
                active: "inactive".into(),
                sub: "dead".into(),
                masked: true,
                ..Unit::default()
            }),
        }
    }
    session.sort_by(|a, b| a.name.cmp(&b.name));
}

pub fn control(user: bool, verb: &str, unit: &str) -> Result<(), String> {
    let mut args = vec![verb, unit];
    if user {
        args.insert(0, "--user");
    }
    run_for(SLOW, "systemctl", &args).map(|_| ())
}

/// The last lines a unit logged.
pub fn log(user: bool, unit: &str) -> Result<String, String> {
    let mut args = vec!["-u", unit, "-n", "40", "--no-pager", "-o", "short-iso"];
    if user {
        args.insert(0, "--user");
    }
    run("journalctl", &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_lines() {
        let s = "nordlys-shell@bar.service loaded active running Nordlys bar\n\
                 ● nordlys-idle.service loaded failed failed Nordlys idle (hypridle)\n";
        let u = parse_units(s);
        assert_eq!(u.len(), 2);
        assert_eq!(u[0].short(), "Bar");
        assert!(u[0].running());
        assert_eq!(u[1].name, "nordlys-idle.service");
        assert!(u[1].failed());
        assert_eq!(u[1].description, "Nordlys idle (hypridle)");
        assert_eq!(u[1].short(), "Idle");
    }

    #[test]
    fn masked_units_stay_listed() {
        let mut u =
            parse_units("nordlys-idle.service loaded inactive dead Nordlys idle (hypridle)\n");
        let masked = parse_masked(
            "nordlys-idle.service masked enabled\nnordlys-clipboard.service masked enabled\n",
        );
        mark_masked(&mut u, &masked);
        assert_eq!(u.len(), 2);
        assert_eq!(u[0].name, "nordlys-clipboard.service");
        assert!(u.iter().all(|u| u.masked && u.optional()));
        let target = Unit {
            name: "nordlys-session.target".into(),
            ..Unit::default()
        };
        assert!(!target.optional());
    }
}
