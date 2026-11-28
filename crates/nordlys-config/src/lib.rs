//! `~/.config/nordlys/nordlys.toml` and the palette every component draws
//! from. One file, read by the compositor, the shell and the theme writer.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

mod palette;
pub use palette::{Palette, Rgb};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub theme: Theme,
    pub stage: StageConfig,
    pub bar: BarConfig,
    pub gaps: Gaps,
    #[serde(rename = "group")]
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub mode: Mode,
    /// `#RRGGBB`. Unset means the mode's default accent.
    pub accent: Option<String>,
    /// Blur radius in pixels for glass surfaces.
    pub blur: u8,
    /// Swap glass for solid surfaces (weak GPUs, accessibility).
    pub reduce_transparency: bool,
    pub reduce_motion: bool,
    /// Corner radius in pixels for windows and shell surfaces, 0–24. Every
    /// other rounded corner scales from it (see `Palette::corner`).
    pub roundness: u8,
}

/// The default corner radius, the one the shell was drawn at.
pub const DEFAULT_ROUNDNESS: u8 = 14;

impl Default for Theme {
    fn default() -> Self {
        Self {
            mode: Mode::Dark,
            accent: None,
            blur: 24,
            reduce_transparency: false,
            reduce_motion: false,
            roundness: DEFAULT_ROUNDNESS,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StageConfig {
    pub layout: Layout,
    /// Main pane's share of the stage in two-up.
    pub split: f32,
    /// Redraw rate for windows waiting on the shelf.
    pub shelf_fps: u8,
    /// Show the left shelf rail. Off, the rail isn't started; windows still
    /// wait, and the keys that cycle them still work.
    pub shelf: bool,
}

impl Default for StageConfig {
    fn default() -> Self {
        Self {
            layout: Layout::Auto,
            split: 0.6,
            shelf_fps: 10,
            shelf: true,
        }
    }
}

/// The space between the screen's edges and the panels on them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Gaps {
    /// Pixels from the screen's top edge down to the top bar.
    pub topbar: u8,
    /// Pixels from the screen's left edge to the shelf rail.
    pub shelf: u8,
}

impl Default for Gaps {
    fn default() -> Self {
        Self {
            topbar: 10,
            shelf: 12,
        }
    }
}

/// The top bar's spacing and which of its parts show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BarConfig {
    /// Space in pixels above and below the bar: the gap to the screen's
    /// edge and to whatever sits under it, like the shelf.
    pub gap: u8,
    /// Space in pixels at each end of the bar.
    pub side: u8,
    /// Parts of the bar to leave out.
    pub hidden: Vec<BarItem>,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            gap: 10,
            side: 12,
            hidden: Vec::new(),
        }
    }
}

/// A part of the top bar that can be hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BarItem {
    Apps,
    Workspaces,
    Stage,
    Clock,
    Recording,
    Media,
    Notifications,
    Tray,
    Layout,
    Indicators,
    Battery,
}

impl BarItem {
    pub const ALL: [BarItem; 11] = [
        BarItem::Apps,
        BarItem::Workspaces,
        BarItem::Stage,
        BarItem::Clock,
        BarItem::Recording,
        BarItem::Media,
        BarItem::Notifications,
        BarItem::Tray,
        BarItem::Layout,
        BarItem::Indicators,
        BarItem::Battery,
    ];
}

/// How the shelves are drawn. `Auto` picks by monitor width:
/// over 2400 px ultrawide, 1600–2400 px rails, under 1600 px peek.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    #[default]
    Auto,
    Ultrawide,
    Rails,
    Peek,
}

impl Layout {
    pub fn for_width(self, logical_px: u32) -> Layout {
        match self {
            Layout::Auto if logical_px > 2400 => Layout::Ultrawide,
            Layout::Auto if logical_px >= 1600 => Layout::Rails,
            Layout::Auto => Layout::Peek,
            fixed => fixed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    /// `Super G` then this letter.
    pub key: Option<char>,
    /// App ids in launch order: the first takes the stage, the rest wait on the shelf.
    pub apps: Vec<String>,
    #[serde(default)]
    pub open_in: OpenIn,
    pub dir: Option<String>,
    #[serde(default = "yes")]
    pub reuse_running: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenIn {
    #[default]
    NewWorkspace,
    Current,
}

impl Config {
    pub fn palette(&self) -> Palette {
        let mut p = match self.theme.mode {
            Mode::Dark => Palette::dark(),
            Mode::Light => Palette::light(),
        };
        if let Some(a) = self.theme.accent.as_deref().and_then(Rgb::parse) {
            p.set_accent(a);
        }
        p.roundness = self.theme.roundness;
        p
    }

    pub fn group(&self, name_or_key: &str) -> Option<&Group> {
        let mut chars = name_or_key.chars();
        let key = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        self.groups.iter().find(|g| {
            g.name.eq_ignore_ascii_case(name_or_key)
                || (key.is_some() && g.key.map(|k| k.to_ascii_lowercase()) == key)
        })
    }

    pub fn from_toml(s: &str) -> Result<Self, Error> {
        let cfg: Config = toml::from_str(s).map_err(|e| Error::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), Error> {
        if let Some(a) = &self.theme.accent
            && Rgb::parse(a).is_none()
        {
            return Err(Error::Invalid(format!("theme.accent {a:?} is not #RRGGBB")));
        }
        if !(0.3..=0.8).contains(&self.stage.split) {
            return Err(Error::Invalid(
                "stage.split must be between 0.3 and 0.8".into(),
            ));
        }
        if self.theme.roundness > 24 {
            return Err(Error::Invalid("theme.roundness must be 0 to 24".into()));
        }
        if self.bar.gap > 24 {
            return Err(Error::Invalid("bar.gap must be 0 to 24".into()));
        }
        if self.gaps.topbar > 40 {
            return Err(Error::Invalid("gaps.topbar must be 0 to 40".into()));
        }
        if self.gaps.shelf > 40 {
            return Err(Error::Invalid("gaps.shelf must be 0 to 40".into()));
        }
        if self.bar.side > 40 {
            return Err(Error::Invalid("bar.side must be 0 to 40".into()));
        }
        for g in &self.groups {
            if g.apps.is_empty() {
                return Err(Error::Invalid(format!("group {:?} has no apps", g.name)));
            }
        }
        Ok(())
    }

    /// Load `path`. A missing file is not an error: defaults apply.
    pub fn load(path: &Path) -> Result<Self, Error> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::Io(path.to_owned(), e)),
        }
    }
}

/// `$XDG_CONFIG_HOME/nordlys`, else `~/.config/nordlys`.
impl Group {
    /// `dir` with a leading `~` swapped for the home folder.
    pub fn dir_path(&self) -> Option<PathBuf> {
        let dir = self.dir.as_deref()?;
        let home = || std::env::var_os("HOME").map(PathBuf::from);
        Some(match dir.strip_prefix('~') {
            Some("") => home()?,
            Some(rest) if rest.starts_with('/') => home()?.join(&rest[1..]),
            _ => PathBuf::from(dir),
        })
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("nordlys")
}

pub fn default_path() -> PathBuf {
    config_dir().join("nordlys.toml")
}

#[derive(Debug)]
pub enum Error {
    Io(PathBuf, std::io::Error),
    Parse(String),
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            Error::Parse(e) => write!(f, "nordlys.toml: {e}"),
            Error::Invalid(e) => write!(f, "nordlys.toml: {e}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../config/nordlys.toml");

    #[test]
    fn example_config_parses() {
        let c = Config::from_toml(EXAMPLE).unwrap();
        assert_eq!(c.groups.len(), 2);
        assert_eq!(c.group("nordlys dev").unwrap().apps[0], "helix");
        assert_eq!(c.group("R").unwrap().name, "Research");
    }

    #[test]
    fn empty_file_means_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn typos_are_rejected() {
        assert!(Config::from_toml("[theme]\nacent = \"#fff\"").is_err());
        assert!(Config::from_toml("[theme]\naccent = \"blue\"").is_err());
    }

    #[test]
    fn look_settings_parse_and_validate() {
        let c = Config::from_toml(
            "[theme]\nroundness = 0\n[stage]\nshelf = false\n\
             [bar]\ngap = 4\nside = 20\nhidden = [\"tray\", \"layout\"]\n",
        )
        .unwrap();
        assert_eq!(c.theme.roundness, 0);
        assert!(!c.stage.shelf);
        assert_eq!((c.bar.gap, c.bar.side), (4, 20));
        assert_eq!(c.bar.hidden, [BarItem::Tray, BarItem::Layout]);
        assert_eq!(c.palette().corner(14.0), 0.0);
        assert!(Config::from_toml("[bar]\nhidden = [\"clok\"]").is_err());
        assert!(Config::from_toml("[theme]\nroundness = 25").is_err());
        assert!(Config::from_toml("[bar]\ngap = 25").is_err());
        assert!(Config::from_toml("[bar]\nside = 41").is_err());
    }

    #[test]
    fn screen_gaps_parse_and_validate() {
        let c = Config::from_toml("[gaps]\ntopbar = 4\nshelf = 30\n").unwrap();
        assert_eq!((c.gaps.topbar, c.gaps.shelf), (4, 30));
        assert_eq!(Config::default().gaps, Gaps::default());
        assert!(Config::from_toml("[gaps]\ntopbar = 41").is_err());
        assert!(Config::from_toml("[gaps]\nshelf = 41").is_err());
        assert!(Config::from_toml("[gaps]\ntop = 4").is_err());
    }

    #[test]
    fn corners_scale_from_the_default() {
        let p = Config::default().palette();
        assert_eq!(p.corner(8.0), 8.0);
        let big = Config::from_toml("[theme]\nroundness = 24").unwrap();
        assert_eq!(big.palette().corner(14.0), 24.0);
    }

    #[test]
    fn auto_layout_follows_width() {
        assert_eq!(Layout::Auto.for_width(3440), Layout::Ultrawide);
        assert_eq!(Layout::Auto.for_width(2560), Layout::Ultrawide);
        assert_eq!(Layout::Auto.for_width(1920), Layout::Rails);
        assert_eq!(Layout::Auto.for_width(1440), Layout::Peek);
        assert_eq!(Layout::Rails.for_width(3440), Layout::Rails);
    }

    #[test]
    fn accent_override_applies() {
        let c = Config::from_toml("[theme]\naccent = \"#F4B183\"").unwrap();
        assert_eq!(c.palette().accent.hex(), "#F4B183");
    }

    #[test]
    fn group_dir_expands_home() {
        let g = |dir: &str| Group {
            name: "x".into(),
            key: None,
            apps: vec![],
            open_in: OpenIn::Current,
            dir: Some(dir.into()),
            reuse_running: true,
        };
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(g("~/code").dir_path(), Some(home.join("code")));
        assert_eq!(g("~").dir_path(), Some(home));
        assert_eq!(g("/srv").dir_path(), Some(PathBuf::from("/srv")));
        assert_eq!(g("~bob/x").dir_path(), Some(PathBuf::from("~bob/x")));
    }
}
