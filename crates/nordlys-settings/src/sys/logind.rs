//! What closing the lid and pressing the power button do.
//!
//! systemd-logind decides this for the whole machine, from files only root may
//! write. So a change goes through pkexec and the user answers the system's
//! password prompt. Nothing typed by anyone reaches the command or the file:
//! the commands are fixed, and the file is built from [`Key`] and [`Choice`]
//! alone. It's piped in, so root never reads a file another process could
//! swap. Removing the file goes back to the system's own settings.

use super::{run, run_for, wait_within};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

const DROP_IN: &str = "/etc/systemd/logind.conf.d/90-nordlys.conf";
/// Long enough to type a password.
const PROMPT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Lid,
    LidPlugged,
    LidDocked,
    PowerButton,
}

impl Key {
    pub const ALL: [Key; 4] = [Key::Lid, Key::LidPlugged, Key::LidDocked, Key::PowerButton];

    fn name(self) -> &'static str {
        match self {
            Key::Lid => "HandleLidSwitch",
            Key::LidPlugged => "HandleLidSwitchExternalPower",
            Key::LidDocked => "HandleLidSwitchDocked",
            Key::PowerButton => "HandlePowerKey",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Suspend,
    Hibernate,
    Lock,
    Ignore,
    PowerOff,
}

impl Choice {
    const ALL: [Choice; 5] = [
        Choice::Suspend,
        Choice::Hibernate,
        Choice::Lock,
        Choice::Ignore,
        Choice::PowerOff,
    ];

    fn word(self) -> &'static str {
        match self {
            Choice::Suspend => "suspend",
            Choice::Hibernate => "hibernate",
            Choice::Lock => "lock",
            Choice::Ignore => "ignore",
            Choice::PowerOff => "poweroff",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Choice::Suspend => "Sleep",
            Choice::Hibernate => "Hibernate",
            Choice::Lock => "Lock",
            Choice::Ignore => "Nothing",
            Choice::PowerOff => "Shut down",
        }
    }

    fn parse(word: &str) -> Option<Choice> {
        Choice::ALL.into_iter().find(|c| c.word() == word)
    }
}

/// What logind does now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Buttons {
    /// logind's own word for each of [`Key::ALL`]; empty when unset.
    words: [String; 4],
    /// This app's file is there, so there's something to undo.
    pub ours: bool,
    pub has_lid: bool,
}

impl Buttons {
    /// What a key does, or None when it's something this app doesn't offer.
    pub fn get(&self, key: Key) -> Option<Choice> {
        let word = &self.words[key as usize];
        // Unset on external power means the same as on battery.
        if word.is_empty() && key == Key::LidPlugged {
            return self.get(Key::Lid);
        }
        Choice::parse(word)
    }

    /// logind's word, for a setting this app doesn't offer.
    pub fn word(&self, key: Key) -> &str {
        &self.words[key as usize]
    }
}

pub fn read() -> Result<Buttons, String> {
    let mut args = vec![
        "get-property",
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    ];
    args.extend(Key::ALL.map(Key::name));
    let words = parse_properties(&run("busctl", &args)?)?;
    Ok(Buttons {
        words,
        ours: std::path::Path::new(DROP_IN).exists(),
        has_lid: std::fs::read_dir("/proc/acpi/button/lid").is_ok_and(|mut d| d.next().is_some()),
    })
}

/// `s "suspend"`, one line for each property asked for.
fn parse_properties(s: &str) -> Result<[String; 4], String> {
    let words: Vec<String> = s
        .lines()
        .filter_map(|l| l.strip_prefix("s \"")?.strip_suffix('"'))
        .map(str::to_owned)
        .collect();
    words
        .try_into()
        .map_err(|_| "logind didn't say what the buttons do".to_owned())
}

/// The settings in this app's file. Anything else in it is dropped on the
/// next save.
fn parse_file(s: &str) -> Vec<(Key, Choice)> {
    s.lines()
        .filter_map(|line| {
            let (name, word) = line.trim().split_once('=')?;
            let key = Key::ALL.into_iter().find(|k| k.name() == name.trim())?;
            Some((key, Choice::parse(word.trim())?))
        })
        .collect()
}

fn render(settings: &[(Key, Choice)]) -> String {
    let mut s = String::from(
        "# Written by Nordlys Settings. Delete this file to go back to the system's settings.\n\
         [Login]\n",
    );
    for (key, choice) in settings {
        s.push_str(&format!("{}={}\n", key.name(), choice.word()));
    }
    s
}

fn with(mut settings: Vec<(Key, Choice)>, key: Key, choice: Choice) -> Vec<(Key, Choice)> {
    match settings.iter_mut().find(|(k, _)| *k == key) {
        Some(s) => s.1 = choice,
        None => settings.push((key, choice)),
    }
    settings
}

/// Save one setting. True when it applies now, false when it waits for a
/// restart.
pub fn set(key: Key, choice: Choice) -> Result<bool, String> {
    let current = match std::fs::read_to_string(DROP_IN) {
        Ok(s) => parse_file(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => return Err(format!("{DROP_IN}: {e}")),
    };
    as_root(
        &[
            "/usr/bin/install",
            "-D",
            "-m",
            "0644",
            "/dev/stdin",
            DROP_IN,
        ],
        &render(&with(current, key, choice)),
    )?;
    Ok(reload())
}

/// Remove this app's file.
pub fn reset() -> Result<bool, String> {
    as_root(&["/usr/bin/rm", "-f", DROP_IN], "")?;
    Ok(reload())
}

/// Have logind read its settings again, which leaves sessions alone. systemd
/// asks for permission itself.
fn reload() -> bool {
    run_for(PROMPT, "systemctl", &["reload", "systemd-logind.service"]).is_ok()
}

/// Run a fixed command as root through the desktop's password prompt.
fn as_root(command: &[&str], input: &str) -> Result<(), String> {
    let mut child = Command::new("pkexec")
        // No hidden prompt on a terminal behind the window.
        .arg("--disable-internal-agent")
        .args(command)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "pkexec (polkit) is not installed".to_owned(),
            _ => format!("pkexec: {e}"),
        })?;
    if let Some(mut pipe) = child.stdin.take() {
        // A refused prompt can close the pipe first; the exit status says why.
        let _ = pipe.write_all(input.as_bytes());
    }
    let Some((status, mut child)) = wait_within(child, PROMPT) else {
        return Err("Not changed: the password prompt timed out".into());
    };
    if status.success() {
        return Ok(());
    }
    let mut said = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut said);
    }
    Err(refusal(status.code(), said.trim()))
}

fn refusal(code: Option<i32>, said: &str) -> String {
    match code {
        Some(126) => "Not changed: the password prompt was closed".into(),
        Some(127) => {
            "Not changed: permission wasn't given (a polkit agent has to be running)".into()
        }
        _ => format!(
            "Not changed: {}",
            said.lines().last().unwrap_or("the command failed")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_logind_says() {
        let w = parse_properties("s \"suspend\"\ns \"\"\ns \"ignore\"\ns \"reboot\"").unwrap();
        let b = Buttons {
            words: w,
            ..Buttons::default()
        };
        assert_eq!(b.get(Key::Lid), Some(Choice::Suspend));
        assert_eq!(b.get(Key::LidPlugged), Some(Choice::Suspend));
        assert_eq!(b.get(Key::LidDocked), Some(Choice::Ignore));
        assert_eq!(b.get(Key::PowerButton), None);
        assert_eq!(b.word(Key::PowerButton), "reboot");
        assert!(parse_properties("s \"suspend\"").is_err());
    }

    #[test]
    fn the_file_keeps_only_what_was_set() {
        let first = render(&with(vec![], Key::Lid, Choice::Lock));
        assert_eq!(parse_file(&first), [(Key::Lid, Choice::Lock)]);
        assert!(first.contains("[Login]\nHandleLidSwitch=lock\n"));
        let second = with(parse_file(&first), Key::PowerButton, Choice::Suspend);
        let third = with(second, Key::Lid, Choice::Ignore);
        assert_eq!(
            parse_file(&render(&third)),
            [
                (Key::Lid, Choice::Ignore),
                (Key::PowerButton, Choice::Suspend)
            ]
        );
        // Only known keys and words come back out of a file.
        assert!(parse_file("ExecStart=/bin/sh\nHandleLidSwitch=$(reboot)\n").is_empty());
    }
}
