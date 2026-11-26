//! The on-screen display for volume and brightness: a glass pill near the
//! bottom of the screen with the new level, shown for a moment after a
//! media key.
//!
//! `nordlys-shell osd volume up` changes the volume and shows it. Only the
//! first of a burst of presses opens a surface: it takes
//! `$XDG_RUNTIME_DIR/nordlys-osd.sock`, and later presses just tell it the
//! new level and exit. It closes itself once the keys have been quiet for
//! a moment, so nothing sits on the screen (or catches clicks) otherwise.

use crate::icons::{self, icon};
use crate::ui::Rail;
use iced::alignment::Vertical;
use iced::widget::{container, progress_bar, row, text};
use iced::{Color, Element, Length, Subscription, Task};
use iced_layershell::to_layer_message;
use nordlys_config::Config;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const WIDTH: u32 = 340;
pub const HEIGHT: u32 = 64;
/// Distance from the bottom of the screen.
pub const LIFT: i32 = 96;
/// How long the pill stays after the last press.
const LINGER: Duration = Duration::from_millis(1600);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Volume,
    Microphone,
    Brightness,
}

/// What the pill shows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub kind: Kind,
    /// 0–100 (volume may go to 150).
    pub percent: u8,
    pub muted: bool,
}

/// What a key asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Up,
    Down,
    Mute,
    Show,
}

pub const USAGE: &str = "\
usage: nordlys-shell osd volume|brightness up|down|show
       nordlys-shell osd volume|mic mute

Changes the level by 5% and shows it on screen. Bind it to the media keys.";

/// `["volume", "up"]` → what to change.
pub fn parse(args: &[String]) -> Result<(Kind, Change), String> {
    let kind = match args.first().map(String::as_str) {
        Some("volume") => Kind::Volume,
        Some("mic" | "microphone") => Kind::Microphone,
        Some("brightness") => Kind::Brightness,
        _ => return Err(USAGE.into()),
    };
    let change = match args.get(1).map(String::as_str) {
        Some("up" | "+") => Change::Up,
        Some("down" | "-") => Change::Down,
        Some("mute") => Change::Mute,
        Some("show") | None => Change::Show,
        Some(other) => return Err(format!("unknown change {other:?}\n{USAGE}")),
    };
    match (kind, change) {
        (Kind::Brightness, Change::Mute) => Err("brightness has no mute".into()),
        (Kind::Microphone, Change::Up | Change::Down) => {
            Err("only `mic mute` and `mic show`".into())
        }
        ok => Ok(ok),
    }
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Make the change, then read the level back.
pub fn apply(kind: Kind, change: Change) -> Option<Level> {
    const SINK: &str = "@DEFAULT_AUDIO_SINK@";
    const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";
    match (kind, change) {
        (Kind::Volume, Change::Up) => {
            run("wpctl", &["set-mute", SINK, "0"]);
            run("wpctl", &["set-volume", "-l", "1", SINK, "5%+"])
        }
        (Kind::Volume, Change::Down) => run("wpctl", &["set-volume", SINK, "5%-"]),
        (Kind::Volume, Change::Mute) => run("wpctl", &["set-mute", SINK, "toggle"]),
        (Kind::Microphone, Change::Mute) => run("wpctl", &["set-mute", SOURCE, "toggle"]),
        (Kind::Brightness, Change::Up) => run("brightnessctl", &["-q", "-e4", "-n2", "set", "5%+"]),
        (Kind::Brightness, Change::Down) => {
            run("brightnessctl", &["-q", "-e4", "-n2", "set", "5%-"])
        }
        _ => Some(String::new()),
    };
    read(kind)
}

fn read(kind: Kind) -> Option<Level> {
    match kind {
        // Waited for in full: right after a change wpctl can be slow,
        // and the pill is only worth showing with the new level.
        Kind::Volume | Kind::Microphone => {
            let node = if kind == Kind::Volume {
                "@DEFAULT_AUDIO_SINK@"
            } else {
                "@DEFAULT_AUDIO_SOURCE@"
            };
            let out = run("wpctl", &["get-volume", node])?;
            Some(Level {
                kind,
                percent: wpctl_percent(&out)?,
                muted: out.contains("[MUTED]"),
            })
        }
        Kind::Brightness => Some(Level {
            kind,
            percent: brightness_percent(&run("brightnessctl", &["-m"])?)?,
            muted: false,
        }),
    }
}

/// "Volume: 0.45 [MUTED]" → 45.
fn wpctl_percent(s: &str) -> Option<u8> {
    let v: f32 = s
        .strip_prefix("Volume:")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((v.clamp(0.0, 1.5) * 100.0).round() as u8)
}

/// brightnessctl -m: "intel_backlight,backlight,12000,50%,24000" → 50.
fn brightness_percent(s: &str) -> Option<u8> {
    let line = s.lines().next()?;
    let pct = line.split(',').nth(3)?.trim().trim_end_matches('%');
    pct.parse::<u8>().ok().map(|p| p.min(100))
}

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("nordlys-osd.sock")
}

/// Hand the level to a pill that's already up. False if there is none,
/// or it's on its way out (it doesn't say "ok"): then this press opens
/// a pill of its own.
pub fn tell_running(level: Level) -> bool {
    let Ok(mut s) = UnixStream::connect(socket_path()) else {
        return false;
    };
    let line = serde_json::to_string(&level).unwrap_or_default() + "\n";
    if s.write_all(line.as_bytes()).is_err() {
        return false;
    }
    let _ = s.set_read_timeout(Some(Duration::from_millis(400)));
    let mut ack = String::new();
    BufReader::new(s).read_line(&mut ack).is_ok() && ack.trim() == "ok"
}

static LISTENER: OnceLock<UnixListener> = OnceLock::new();
/// Set once the pill has decided to close; later presses get no "ok".
static CLOSING: AtomicBool = AtomicBool::new(false);

/// Become the pill: take the socket. Err if it can't be taken.
pub fn claim() -> Result<(), String> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path); // a stale socket; tell_running failed
    let l = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    LISTENER
        .set(l)
        .map_err(|_| "the OSD socket is already taken".to_owned())
}

/// Levels other presses send, as they arrive.
fn updates() -> Subscription<Level> {
    Subscription::run(|| {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        if let Some(l) = LISTENER.get().and_then(|l| l.try_clone().ok()) {
            std::thread::spawn(move || {
                for stream in l.incoming().flatten() {
                    let Ok(mut out) = stream.try_clone() else {
                        continue;
                    };
                    for line in BufReader::new(stream).lines().map_while(Result::ok) {
                        let Ok(level) = serde_json::from_str::<Level>(&line) else {
                            continue;
                        };
                        if CLOSING.load(Ordering::SeqCst) || tx.unbounded_send(level).is_err() {
                            return;
                        }
                        let _ = out.write_all(b"ok\n");
                    }
                }
            });
        }
        rx
    })
}

pub struct Osd {
    level: Level,
    /// The last change shown. None until the surface is up and ticking,
    /// so a slow start doesn't eat the time on screen.
    shown_at: Option<Instant>,
    rail: Rail,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Level(Level),
    Tick,
}

impl Osd {
    pub fn new(config: &Config, level: Level) -> (Self, Task<Message>) {
        (
            Self {
                level,
                shown_at: None,
                rail: Rail::of(&config.palette()),
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Level(l) => {
                self.level = l;
                self.shown_at = Some(Instant::now());
            }
            Message::Tick => match self.shown_at {
                None => self.shown_at = Some(Instant::now()),
                Some(t) if t.elapsed() >= LINGER => {
                    // Give the name up first, so the next press opens a new pill.
                    CLOSING.store(true, Ordering::SeqCst);
                    let _ = std::fs::remove_file(socket_path());
                    return iced::exit();
                }
                Some(_) => {}
            },
            _ => {}
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            updates().map(Message::Level),
            iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick),
        ])
    }

    pub fn view(&self) -> Element<'_, Message> {
        let r = self.rail;
        let l = self.level;
        let glyph = match l.kind {
            Kind::Volume if l.muted || l.percent == 0 => icons::VOLUME_MUTED,
            Kind::Volume if l.percent < 34 => icons::VOLUME,
            Kind::Volume if l.percent < 67 => icons::VOLUME_LOW,
            Kind::Volume => icons::VOLUME_HIGH,
            Kind::Microphone if l.muted => icons::VOLUME_MUTED,
            Kind::Microphone => icons::MUSIC,
            Kind::Brightness => icons::MONITOR,
        };
        let tone = if l.muted { r.dim } else { r.accent };
        let label = if l.muted {
            "Muted".to_owned()
        } else {
            format!("{}%", l.percent)
        };
        let bar = progress_bar(0.0..=100.0, l.percent.min(100) as f32)
            .girth(6)
            .style(move |_| progress_bar::Style {
                background: r.chip.into(),
                bar: tone.into(),
                border: iced::Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
            });
        let body = row![
            icon(glyph, 20).color(tone),
            container(bar).width(Length::Fill),
            text(label).size(14).color(r.text).width(52),
        ]
        .spacing(14)
        .align_y(Vertical::Center);
        container(body)
            .padding([0, 20])
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(r.panel_style())
            .into()
    }

    pub fn style(&self, _theme: &iced::Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: self.rail.text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_what_the_keys_send() {
        assert_eq!(parse(&args("volume up")), Ok((Kind::Volume, Change::Up)));
        assert_eq!(
            parse(&args("brightness -")),
            Ok((Kind::Brightness, Change::Down))
        );
        assert_eq!(
            parse(&args("mic mute")),
            Ok((Kind::Microphone, Change::Mute))
        );
        assert_eq!(parse(&args("volume")), Ok((Kind::Volume, Change::Show)));
        assert!(parse(&args("brightness mute")).is_err());
        assert!(parse(&args("mic up")).is_err());
        assert!(parse(&args("loudness up")).is_err());
    }

    #[test]
    fn reads_wpctl_and_brightnessctl() {
        assert_eq!(wpctl_percent("Volume: 0.45\n"), Some(45));
        assert_eq!(wpctl_percent("Volume: 1.20 [MUTED]"), Some(120));
        assert_eq!(wpctl_percent("nonsense"), None);
        assert_eq!(
            brightness_percent("intel_backlight,backlight,12000,50%,24000\n"),
            Some(50)
        );
        assert_eq!(brightness_percent("x,y,1"), None);
    }

    #[test]
    fn levels_travel_as_json_lines() {
        let l = Level {
            kind: Kind::Brightness,
            percent: 40,
            muted: false,
        };
        let line = serde_json::to_string(&l).unwrap();
        assert_eq!(line, r#"{"kind":"brightness","percent":40,"muted":false}"#);
        assert_eq!(serde_json::from_str::<Level>(&line).unwrap(), l);
    }
}
