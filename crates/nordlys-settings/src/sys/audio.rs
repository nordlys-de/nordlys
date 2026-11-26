//! Sound through PipeWire's `wpctl`.

use super::run;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Audio {
    pub available: bool,
    pub sinks: Vec<Node>,
    pub sources: Vec<Node>,
    /// Apps playing or recording right now.
    pub streams: Vec<Node>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub id: u32,
    pub name: String,
    pub default: bool,
    pub volume: f32,
    pub muted: bool,
    /// Streams only: true for recording.
    pub input: bool,
}

impl Audio {
    pub fn default_sink(&self) -> Option<&Node> {
        self.sinks.iter().find(|n| n.default).or(self.sinks.first())
    }
    pub fn default_source(&self) -> Option<&Node> {
        self.sources
            .iter()
            .find(|n| n.default)
            .or(self.sources.first())
    }
}

pub fn read() -> Result<Audio, String> {
    let status = run("wpctl", &["status"])?;
    let mut audio = parse_status(&status);
    audio.available = true;
    // Mute shows only in get-volume; ask for every node at once.
    std::thread::scope(|s| {
        let all: Vec<&mut Node> = audio
            .sinks
            .iter_mut()
            .chain(audio.sources.iter_mut())
            .chain(audio.streams.iter_mut())
            .collect();
        let handles: Vec<_> = all
            .into_iter()
            .map(|n| {
                s.spawn(move || {
                    if let Ok(v) = run("wpctl", &["get-volume", &n.id.to_string()])
                        && let Some((level, muted)) = parse_volume(&v)
                    {
                        n.volume = level;
                        n.muted = muted;
                    }
                })
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
    });
    // Which way does each stream go?
    for st in &mut audio.streams {
        if let Ok(info) = run("wpctl", &["inspect", &st.id.to_string()]) {
            st.input = info.contains("Stream/Input/Audio");
            if let Some(app) = info.lines().find_map(|l| {
                l.trim()
                    .trim_start_matches('*')
                    .trim()
                    .strip_prefix("application.name = ")
            }) {
                st.name = app.trim_matches('"').to_owned();
            }
        }
    }
    Ok(audio)
}

/// The Audio part of `wpctl status`.
fn parse_status(s: &str) -> Audio {
    #[derive(PartialEq)]
    enum Part {
        None,
        Sinks,
        Sources,
        Streams,
    }
    let mut audio = Audio::default();
    let mut in_audio = false;
    let mut part = Part::None;
    let mut stream_indent = None;
    for raw in s.lines() {
        if !raw.starts_with(' ') && !raw.is_empty() {
            in_audio = raw.trim() == "Audio";
            part = Part::None;
            continue;
        }
        if !in_audio {
            continue;
        }
        let line: String = raw
            .chars()
            .map(|c| {
                if matches!(c, '│' | '├' | '└' | '─') {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let t = line.trim();
        match t {
            "Sinks:" => {
                part = Part::Sinks;
                continue;
            }
            "Sources:" => {
                part = Part::Sources;
                continue;
            }
            "Streams:" => {
                part = Part::Streams;
                stream_indent = None;
                continue;
            }
            t if t.ends_with(':') => {
                part = Part::None;
                continue;
            }
            _ => {}
        }
        let default = t.starts_with('*');
        let t = t.trim_start_matches('*').trim();
        let Some((id, rest)) = t.split_once(". ") else {
            continue;
        };
        let Ok(id) = id.trim().parse::<u32>() else {
            continue;
        };
        let (name, vol) = match rest.rsplit_once("[vol:") {
            Some((n, v)) => (n.trim(), v.trim_end_matches(']').trim()),
            None => (rest.trim(), ""),
        };
        let mut node = Node {
            id,
            name: name.to_owned(),
            default,
            volume: vol
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            muted: vol.contains("MUTED"),
            input: false,
        };
        match part {
            Part::Sinks => audio.sinks.push(node),
            Part::Sources => audio.sources.push(node),
            Part::Streams => {
                // Streams list their ports underneath, indented further.
                let indent = line.len() - line.trim_start().len();
                let top = *stream_indent.get_or_insert(indent);
                if indent == top {
                    node.name = node.name.split("  ").next().unwrap_or("").trim().to_owned();
                    audio.streams.push(node);
                }
            }
            Part::None => {}
        }
    }
    audio
}

/// "Volume: 0.37" or "Volume: 0.37 [MUTED]".
fn parse_volume(s: &str) -> Option<(f32, bool)> {
    let rest = s.strip_prefix("Volume:")?.trim();
    let level = rest.split_whitespace().next()?.parse::<f32>().ok()?;
    Some((level, rest.contains("MUTED")))
}

pub fn set_volume(id: u32, level: f32) -> Result<(), String> {
    let v = format!("{:.2}", level.clamp(0.0, 1.5));
    run("wpctl", &["set-volume", &id.to_string(), &v]).map(|_| ())
}

pub fn set_mute(id: u32, muted: bool) -> Result<(), String> {
    run(
        "wpctl",
        &["set-mute", &id.to_string(), if muted { "1" } else { "0" }],
    )
    .map(|_| ())
}

pub fn set_default(id: u32) -> Result<(), String> {
    run("wpctl", &["set-default", &id.to_string()]).map(|_| ())
}

/// A short click on the default output, to check it's the right one.
pub fn test_sound() -> Result<(), String> {
    for f in [
        "/usr/share/sounds/freedesktop/stereo/audio-volume-change.oga",
        "/usr/share/sounds/freedesktop/stereo/bell.oga",
        "/usr/share/sounds/freedesktop/stereo/complete.oga",
    ] {
        if std::path::Path::new(f).is_file() {
            return super::spawn("pw-play", &[f]);
        }
    }
    Err("No test sound found (install sound-theme-freedesktop)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "PipeWire 'pipewire-0' [1.6.11, me@box, cookie:1]
 └─ Clients:
        32. WirePlumber                         [1.6.11, me@box, pid:813]

Audio
 ├─ Devices:
 │      48. TU116 High Definition Audio Controller [alsa]
 │
 ├─ Sinks:
 │      53. Built-in Audio Analog Stereo        [vol: 0.56]
 │  *   58. HDMI Digital Stereo (HDMI) [vol: 0.37 MUTED]
 │
 ├─ Sources:
 │  *   54. Built-in Audio Analog Stereo        [vol: 1.00]
 │
 ├─ Filters:
 │
 └─ Streams:
        70. Firefox
             71. output_FL       > Built-in Audio:playback_FL	[active]
             72. output_FR       > Built-in Audio:playback_FR	[active]

Video
 ├─ Sinks:
 │
 ├─ Sources:
 │  *   60. Integrated Camera (V4L2)
";

    #[test]
    fn status_tree() {
        let a = parse_status(STATUS);
        assert_eq!(a.sinks.len(), 2);
        assert_eq!(a.sinks[1].id, 58);
        assert!(a.sinks[1].default && a.sinks[1].muted);
        assert_eq!(a.sinks[1].name, "HDMI Digital Stereo (HDMI)");
        assert!((a.sinks[0].volume - 0.56).abs() < 1e-6);
        assert_eq!(a.sources.len(), 1, "the camera is video, not sound");
        assert_eq!(a.streams.len(), 1);
        assert_eq!(a.streams[0].name, "Firefox");
        assert_eq!(a.default_sink().unwrap().id, 58);
    }

    #[test]
    fn volume_line() {
        assert_eq!(parse_volume("Volume: 0.40"), Some((0.4, false)));
        assert_eq!(parse_volume("Volume: 0.40 [MUTED]"), Some((0.4, true)));
        assert_eq!(parse_volume("nope"), None);
    }
}
