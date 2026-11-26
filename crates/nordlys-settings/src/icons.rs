//! Lucide icons (ISC licence, see assets/LICENSE-lucide) as SVG, tinted to
//! the text colour around them. To add one, drop its SVG in assets/icons
//! (from lucide.dev, 24×24, stroke) and list it here.

use iced::widget::{Svg, svg};
use iced::{Color, Length};
use std::sync::OnceLock;

macro_rules! icons {
    ($($name:ident => $file:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(dead_code)]
        pub enum Icon { $($name),* }

        const FILES: &[&[u8]] = &[$(include_bytes!(concat!("../assets/icons/", $file, ".svg"))),*];
    };
}

icons! {
    AppWindow => "app-window",
    BatteryCharging => "battery-charging",
    BatteryFull => "battery-full",
    BellOff => "bell-off",
    Bell => "bell",
    BluetoothOff => "bluetooth-off",
    Bluetooth => "bluetooth",
    Check => "check",
    ChevronRight => "chevron-right",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    Clock => "clock",
    Columns2 => "columns-2",
    Cpu => "cpu",
    ExternalLink => "external-link",
    EyeOff => "eye-off",
    Eye => "eye",
    FileText => "file-text",
    Folder => "folder",
    Gauge => "gauge",
    Globe => "globe",
    HardDrive => "hard-drive",
    History => "history",
    Image => "image",
    Info => "info",
    Keyboard => "keyboard",
    Layers => "layers",
    LayoutDashboard => "layout-dashboard",
    Leaf => "leaf",
    Link => "link",
    Lock => "lock",
    LogOut => "log-out",
    MemoryStick => "memory-stick",
    MicOff => "mic-off",
    Mic => "mic",
    Monitor => "monitor",
    MoonStar => "moon-star",
    Moon => "moon",
    Mouse => "mouse",
    Package => "package",
    Palette => "palette",
    Pencil => "pencil",
    Play => "play",
    Plus => "plus",
    Power => "power",
    RefreshCw => "refresh-cw",
    Rocket => "rocket",
    RotateCcw => "rotate-ccw",
    ScanSearch => "scan-search",
    Search => "search",
    Server => "server",
    Shield => "shield",
    SlidersHorizontal => "sliders-horizontal",
    Square => "square",
    Sun => "sun",
    Terminal => "terminal",
    Trash2 => "trash-2",
    Unlink => "unlink",
    User => "user",
    Volume2 => "volume-2",
    VolumeX => "volume-x",
    WifiHigh => "wifi-high",
    WifiLow => "wifi-low",
    WifiOff => "wifi-off",
    Wifi => "wifi",
    WifiZero => "wifi-zero",
    Wrench => "wrench",
    X => "x",
    Zap => "zap",
}

fn handle(i: Icon) -> svg::Handle {
    static HANDLES: OnceLock<Vec<svg::Handle>> = OnceLock::new();
    HANDLES.get_or_init(|| FILES.iter().map(|b| svg::Handle::from_memory(*b)).collect())[i as usize]
        .clone()
}

/// An icon `size` pixels square in `color`.
pub fn icon<'a>(i: Icon, size: f32, color: Color) -> Svg<'a> {
    svg(handle(i))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_, _| svg::Style { color: Some(color) })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_icon_is_an_svg() {
        for f in super::FILES {
            assert!(f.starts_with(b"<svg"), "an icon file isn't SVG");
        }
    }
}
