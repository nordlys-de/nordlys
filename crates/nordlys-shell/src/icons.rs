//! Lucide icons (ISC licence, see assets/LICENSE-lucide), bundled as a
//! 12 KB subset font so the shell needs nothing installed. Icons are text
//! glyphs: they take the colour and size of the text around them.
//!
//! To add one, put its Lucide name in scripts/subset-icons.sh, rerun it,
//! and add the codepoint from the release's codepoints.json here.

// A few are kept ready for pieces still to come (power menu, settings).
#![allow(dead_code)]

use iced::Font;
use iced::widget::{Text, text};

pub const FONT_BYTES: &[u8] = include_bytes!("../assets/lucide-subset.ttf");
pub const FONT: Font = Font::with_name("lucide");

pub const WIFI: char = '\u{e1ae}';
pub const WIFI_OFF: char = '\u{e1af}';
pub const BLUETOOTH: char = '\u{e05c}';
pub const BLUETOOTH_OFF: char = '\u{e1b9}';
pub const BELL: char = '\u{e059}';
pub const BELL_OFF: char = '\u{e05a}';
pub const MOON: char = '\u{e11e}';
pub const MUSIC: char = '\u{e122}';
pub const PLAY: char = '\u{e13c}';
pub const PAUSE: char = '\u{e12e}';
pub const SKIP_FORWARD: char = '\u{e160}';
pub const SLIDERS: char = '\u{e29a}';
pub const VOLUME: char = '\u{e1a9}';
pub const VOLUME_LOW: char = '\u{e1aa}';
pub const VOLUME_HIGH: char = '\u{e1ab}';
pub const VOLUME_MUTED: char = '\u{e1ac}';
pub const CLOSE: char = '\u{e1b2}';
pub const BATTERY: char = '\u{e053}';
pub const BATTERY_LOW: char = '\u{e056}';
pub const BATTERY_MEDIUM: char = '\u{e057}';
pub const BATTERY_FULL: char = '\u{e055}';
pub const BATTERY_CHARGING: char = '\u{e054}';
pub const TWO_UP: char = '\u{e098}';
pub const GRID: char = '\u{e0ff}';
pub const SEARCH: char = '\u{e151}';
pub const TERMINAL: char = '\u{e181}';
pub const FOLDER: char = '\u{e0d7}';
pub const FILE: char = '\u{e0c0}';
pub const WINDOW: char = '\u{e426}';
pub const POWER: char = '\u{e140}';
pub const LOCK: char = '\u{e10b}';
pub const SETTINGS: char = '\u{e154}';
pub const HISTORY: char = '\u{e1f5}';
pub const MONITOR: char = '\u{e11d}';
pub const REFRESH: char = '\u{e145}';
pub const SHELF: char = '\u{e12a}';
pub const SWAP: char = '\u{e24a}';
pub const EYE: char = '\u{e0ba}';
pub const EYE_OFF: char = '\u{e0bb}';
pub const ARROW_RIGHT: char = '\u{e049}';
pub const RESTART: char = '\u{e148}';
pub const CAPS_LOCK: char = '\u{e1e4}';

/// One icon at the given size; set its colour with `.color()`.
pub fn icon<'a>(glyph: char, size: impl Into<iced::Pixels>) -> Text<'a> {
    text(glyph.to_string())
        .font(FONT)
        .size(size)
        .line_height(1.0)
}

/// The battery glyph for a charge level.
pub fn battery(percent: u8, charging: bool) -> char {
    match (charging, percent) {
        (true, _) => BATTERY_CHARGING,
        (false, 0..=5) => BATTERY,
        (false, 6..=25) => BATTERY_LOW,
        (false, 26..=65) => BATTERY_MEDIUM,
        _ => BATTERY_FULL,
    }
}

/// The volume glyph for a level and mute state.
pub fn volume(level: f32, muted: bool) -> char {
    match (muted, level) {
        (true, _) => VOLUME_MUTED,
        (false, l) if l <= 0.0 => VOLUME,
        (false, l) if l < 0.5 => VOLUME_LOW,
        _ => VOLUME_HIGH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_glyphs() {
        assert_eq!(volume(0.8, true), VOLUME_MUTED);
        assert_eq!(volume(0.0, false), VOLUME);
        assert_eq!(volume(0.3, false), VOLUME_LOW);
        assert_eq!(volume(0.9, false), VOLUME_HIGH);
    }

    #[test]
    fn battery_glyphs() {
        assert_eq!(battery(50, true), BATTERY_CHARGING);
        assert_eq!(battery(3, false), BATTERY);
        assert_eq!(battery(15, false), BATTERY_LOW);
        assert_eq!(battery(40, false), BATTERY_MEDIUM);
        assert_eq!(battery(90, false), BATTERY_FULL);
    }

    #[test]
    fn font_is_bundled() {
        // A TrueType file starts with version 1.0.
        assert_eq!(&FONT_BYTES[..4], &[0, 1, 0, 0]);
    }
}
