//! Lucide icons (ISC licence, see assets/LICENSE-lucide), bundled as a
//! 12 KB subset font so the shell needs nothing installed. Icons are text
//! glyphs: they take the colour and size of the text around them.
//!
//! To add one, put its Lucide name in scripts/subset-icons.sh, rerun it,
//! and add the codepoint from the release's codepoints.json here.

// A few are kept ready for pieces still to come (power menu, settings).
#![allow(dead_code)]

use iced::widget::{Svg, Text, svg, text};
use iced::{Color, Font, Length};

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

/// The Lucide pictures the shell draws as SVG: one stroke weight and one
/// size everywhere, the same set the Settings app uses (see `symbol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    Wifi,
    WifiLow,
    WifiZero,
    WifiOff,
    Bluetooth,
    BluetoothOff,
    Bell,
    BellOff,
    BatteryLow,
    BatteryMedium,
    BatteryFull,
    BatteryCharging,
    Clock,
    Trash,
    X,
    ChevronDown,
    ChevronUp,
}

/// A picture `size` pixels square in `color`, recoloured to fit the text around it.
pub fn symbol<'a>(s: Symbol, size: f32, color: Color) -> Svg<'a> {
    svg(svg::Handle::from_memory(symbol_bytes(s)))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_, _| svg::Style { color: Some(color) })
}

/// The SVG source of a symbol.
pub fn symbol_bytes(s: Symbol) -> &'static [u8] {
    match s {
        Symbol::Wifi => include_bytes!("../assets/icons/wifi.svg"),
        Symbol::WifiLow => include_bytes!("../assets/icons/wifi-low.svg"),
        Symbol::WifiZero => include_bytes!("../assets/icons/wifi-zero.svg"),
        Symbol::WifiOff => include_bytes!("../assets/icons/wifi-off.svg"),
        Symbol::Bluetooth => include_bytes!("../assets/icons/bluetooth.svg"),
        Symbol::BluetoothOff => include_bytes!("../assets/icons/bluetooth-off.svg"),
        Symbol::Bell => include_bytes!("../assets/icons/bell.svg"),
        Symbol::BellOff => include_bytes!("../assets/icons/bell-off.svg"),
        Symbol::BatteryLow => include_bytes!("../assets/icons/battery-low.svg"),
        Symbol::BatteryMedium => include_bytes!("../assets/icons/battery-medium.svg"),
        Symbol::BatteryFull => include_bytes!("../assets/icons/battery-full.svg"),
        Symbol::BatteryCharging => include_bytes!("../assets/icons/battery-charging.svg"),
        Symbol::Clock => include_bytes!("../assets/icons/clock.svg"),
        Symbol::Trash => include_bytes!("../assets/icons/trash-2.svg"),
        Symbol::X => include_bytes!("../assets/icons/x.svg"),
        Symbol::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
        Symbol::ChevronUp => include_bytes!("../assets/icons/chevron-up.svg"),
    }
}

/// What an icon is drawn from: a font glyph (the older set, still used for
/// volume and media) or a picture. Both take a size and a colour via `draw`.
#[derive(Debug, Clone, Copy)]
pub enum Face {
    Font(char),
    Picture(Symbol),
}

impl From<char> for Face {
    fn from(glyph: char) -> Self {
        Face::Font(glyph)
    }
}

impl From<Symbol> for Face {
    fn from(s: Symbol) -> Self {
        Face::Picture(s)
    }
}

/// Draw an icon of either kind `size` pixels, in `color`.
pub fn draw<'a, M: 'a>(face: impl Into<Face>, size: f32, color: Color) -> iced::Element<'a, M> {
    match face.into() {
        Face::Font(glyph) => icon(glyph, size).color(color).into(),
        Face::Picture(s) => symbol(s, size, color).into(),
    }
}

/// The battery picture for a charge level.
pub fn battery(percent: u8, charging: bool) -> Symbol {
    match (charging, percent) {
        (true, _) => Symbol::BatteryCharging,
        (false, 0..=25) => Symbol::BatteryLow,
        (false, 26..=65) => Symbol::BatteryMedium,
        _ => Symbol::BatteryFull,
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
    fn battery_pictures() {
        assert_eq!(battery(50, true), Symbol::BatteryCharging);
        assert_eq!(battery(3, false), Symbol::BatteryLow);
        assert_eq!(battery(15, false), Symbol::BatteryLow);
        assert_eq!(battery(40, false), Symbol::BatteryMedium);
        assert_eq!(battery(90, false), Symbol::BatteryFull);
    }

    #[test]
    fn every_symbol_is_an_svg() {
        for s in [
            Symbol::Wifi,
            Symbol::WifiLow,
            Symbol::WifiZero,
            Symbol::WifiOff,
            Symbol::Bluetooth,
            Symbol::BluetoothOff,
            Symbol::Bell,
            Symbol::BellOff,
            Symbol::BatteryLow,
            Symbol::BatteryMedium,
            Symbol::BatteryFull,
            Symbol::BatteryCharging,
            Symbol::Clock,
            Symbol::Trash,
            Symbol::X,
            Symbol::ChevronDown,
            Symbol::ChevronUp,
        ] {
            assert!(symbol_bytes(s).starts_with(b"<svg"), "{s:?} isn't an SVG");
        }
    }

    #[test]
    fn font_is_bundled() {
        // A TrueType file starts with version 1.0.
        assert_eq!(&FONT_BYTES[..4], &[0, 1, 0, 0]);
    }
}
