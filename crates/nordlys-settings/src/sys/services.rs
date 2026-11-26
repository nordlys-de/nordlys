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
}

impl Unit {
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
        "nordlys-shell@ambient.service" => "Right rail with media and quick settings",
        "nordlys-shell@close-button.service" => "The × that closes the window under the pointer",
        "nordlys-notify.service" => "Notification daemon and toasts",
        "nordlys-idle.service" => "Dims, locks and suspends when you're away",
        "nordlys-wallpaper.service" => "Draws the wallpaper",
        "nordlys-session.target" => "Everything a Nordlys session runs",
        _ => "",
    }
}

pub fn read() -> Result<Services, String> {
    if !super::has("systemctl") {
        return Ok(Services::default());
    }
    let session = parse_units(
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
            })
        })
        .collect()
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
}
