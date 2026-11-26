//! The control protocol between `nordlys-comp` and its clients
//! (`nordlysctl`, `nordlys-shell`, scripts).
//!
//! One JSON object per line over a Unix socket. Each request gets exactly
//! one response line, except `subscribe`: after its answer the connection
//! stays open and gets another `ok` line, with the new state, whenever the
//! stage changes.
//!
//! `{"cmd":"version"}` answers with [`VERSION`]; it changes whenever an
//! existing message changes meaning, so clients can tell before they
//! misread something.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    /// Report the stage without changing it.
    State,
    /// Report the protocol version.
    Version,
    /// Report the stage now and again after every change, on this
    /// connection, until the client hangs up.
    Subscribe,
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
    /// Go to workspace `n` (1-based).
    Workspace {
        n: u32,
    },
    /// Send a window (the main one if no id) to workspace `n`.
    MoveTo {
        n: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Launch an app group by name.
    Group {
        name: String,
    },
    /// Re-read nordlys.toml.
    Reload,
    /// Save the next frame as a PNG at `path` (absolute). Only with a
    /// display; the answer comes before the file is written.
    Screenshot {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Response {
    Ok { state: StageState },
    Version { version: u32 },
    Error { message: String },
}

/// The protocol version `{"cmd":"version"}` reports.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageState {
    pub main: Option<String>,
    pub side: Option<String>,
    /// Most recent first.
    pub shelf: Vec<String>,
    /// The active workspace, 1-based. Main, side and shelf are its own.
    #[serde(default = "first")]
    pub workspace: u32,
    /// Workspaces with windows, and the active one, in order.
    #[serde(default)]
    pub workspaces: Vec<WorkspaceState>,
}

fn first() -> u32 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub n: u32,
    pub windows: u32,
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

/// Where the compositor behind `socket` keeps pictures of the windows
/// waiting on the shelf: `nordlys.sock` → `nordlys.previews/`.
pub fn preview_dir(socket: &Path) -> PathBuf {
    socket.with_extension("previews")
}

/// The picture of window `id` in `dir`. Ids are made of the app's name and
/// a number; anything else is replaced, so an id can't leave the folder.
pub fn preview_path(dir: &Path, id: &str) -> PathBuf {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{safe}.rgba"))
}

/// A preview is `NLYP`, the width and height as little-endian u32s, then
/// RGBA bytes (straight alpha), top row first. Plain pixels, so the
/// compositor can rewrite them several times a second for nothing.
const PREVIEW_MAGIC: &[u8; 4] = b"NLYP";

pub fn encode_preview(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + rgba.len());
    out.extend_from_slice(PREVIEW_MAGIC);
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(rgba);
    out
}

/// (width, height, RGBA) from a preview file's bytes.
pub fn decode_preview(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let rest = bytes.strip_prefix(PREVIEW_MAGIC)?;
    let w = u32::from_le_bytes(rest.get(0..4)?.try_into().ok()?);
    let h = u32::from_le_bytes(rest.get(4..8)?.try_into().ok()?);
    let len = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    let px = rest.get(8..8 + len)?;
    (w > 0 && h > 0).then(|| (w, h, px.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_round_trip_and_reject_junk() {
        let px = vec![1u8, 2, 3, 255, 4, 5, 6, 255];
        let bytes = encode_preview(2, 1, &px);
        assert_eq!(decode_preview(&bytes), Some((2, 1, px)));
        assert_eq!(decode_preview(&bytes[..bytes.len() - 1]), None);
        assert_eq!(decode_preview(b"PNG\0\0\0\0\0\0\0\0\0"), None);
        assert_eq!(decode_preview(&encode_preview(0, 0, &[])), None);
    }

    #[test]
    fn preview_paths_stay_in_their_folder() {
        let dir = preview_dir(Path::new("/run/user/1000/nordlys.sock"));
        assert_eq!(dir, Path::new("/run/user/1000/nordlys.previews"));
        assert_eq!(
            preview_path(&dir, "foot-2"),
            Path::new("/run/user/1000/nordlys.previews/foot-2.rgba")
        );
        assert_eq!(
            preview_path(&dir, "../../etc/x"),
            Path::new("/run/user/1000/nordlys.previews/______etc_x.rgba")
        );
    }

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
            Request::Screenshot {
                path: "/tmp/stage.png".into(),
            },
            Request::Subscribe,
            Request::Version,
            Request::Workspace { n: 3 },
            Request::MoveTo { n: 2, id: None },
            Request::MoveTo {
                n: 2,
                id: Some("foot".into()),
            },
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
                workspace: 1,
                workspaces: vec![WorkspaceState { n: 1, windows: 2 }],
            },
        };
        assert_eq!(from_line::<Response>(&to_line(&resp)).unwrap(), resp);
        assert_eq!(
            to_line(&Response::Version { version: VERSION }),
            "{\"status\":\"version\",\"version\":1}\n"
        );
    }

    #[test]
    fn state_without_workspaces_still_reads() {
        let r: Response =
            from_line(r#"{"status":"ok","state":{"main":null,"side":null,"shelf":[]}}"#).unwrap();
        let Response::Ok { state } = r else { panic!() };
        assert_eq!((state.workspace, state.workspaces.len()), (1, 0));
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
