//! The control protocol between `nordlys-comp` and its clients
//! (`nordlysctl`, `nordlys-shell`, scripts).
//!
//! One JSON object per line over a Unix socket. Each request gets exactly
//! one response line.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    /// Report the stage without changing it.
    State,
    /// Register a window (normally the compositor does this itself).
    Open {
        id: String,
        #[serde(default)]
        ambient: bool,
    },
    Close {
        id: String,
    },
    Bring {
        id: String,
    },
    Previous,
    Cycle,
    TwoUp,
    Swap,
    SendToShelf,
    /// Shelf slot, 1-based.
    Slot {
        n: usize,
    },
    /// Launch an app group by name.
    Group {
        name: String,
    },
    /// Re-read nordlys.toml.
    Reload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Response {
    Ok { state: StageState },
    Error { message: String },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageState {
    pub main: Option<String>,
    pub side: Option<String>,
    /// Most recent first.
    pub shelf: Vec<String>,
}

/// `$NORDLYS_SOCKET`, else `$XDG_RUNTIME_DIR/nordlys.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("NORDLYS_SOCKET") {
        return p.into();
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("nordlys.sock")
}

/// Serialise a message as one protocol line, newline included.
pub fn to_line<T: Serialize>(msg: &T) -> String {
    let mut s = serde_json::to_string(msg).expect("protocol types always serialise");
    s.push('\n');
    s
}

pub fn from_line<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_stable() {
        assert_eq!(to_line(&Request::TwoUp), "{\"cmd\":\"two-up\"}\n");
        assert_eq!(
            to_line(&Request::Bring { id: "helix".into() }),
            "{\"cmd\":\"bring\",\"id\":\"helix\"}\n"
        );
    }

    #[test]
    fn round_trip() {
        let reqs = [
            Request::State,
            Request::Open {
                id: "chat".into(),
                ambient: true,
            },
            Request::Slot { n: 2 },
            Request::Group {
                name: "Nordlys dev".into(),
            },
        ];
        for r in reqs {
            assert_eq!(from_line::<Request>(&to_line(&r)).unwrap(), r);
        }
        let resp = Response::Ok {
            state: StageState {
                main: Some("helix".into()),
                side: None,
                shelf: vec!["term".into()],
            },
        };
        assert_eq!(from_line::<Response>(&to_line(&resp)).unwrap(), resp);
    }

    #[test]
    fn ambient_defaults_to_false() {
        let r: Request = from_line(r#"{"cmd":"open","id":"x"}"#).unwrap();
        assert_eq!(
            r,
            Request::Open {
                id: "x".into(),
                ambient: false
            }
        );
    }
}
