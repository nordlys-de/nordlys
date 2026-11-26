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
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            mode: Mode::Dark,
            accent: None,
            blur: 24,
            reduce_transparency: false,
            reduce_motion: false,
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
}

impl Default for StageConfig {
    fn default() -> Self {
        Self {
            layout: Layout::Auto,
            split: 0.6,
            shelf_fps: 10,
        }
    }
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
