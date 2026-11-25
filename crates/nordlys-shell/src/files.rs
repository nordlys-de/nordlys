//! A small, bounded index of files under $HOME for `/` searches.
//!
//! It walks a few levels deep, skips hidden folders and build output, and
//! stops at a fixed count, so it finishes quickly even on a large home.

use std::path::{Path, PathBuf};

const MAX_DEPTH: u8 = 4;
const MAX_ENTRIES: usize = 40_000;
const SKIP: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "venv",
    "__pycache__",
    "snap",
];

pub fn index_home() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return vec![];
    };
    let mut out = Vec::new();
    walk(&home, 0, &mut out);
    out
}

fn walk(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            if depth < MAX_DEPTH && !SKIP.contains(&name.as_ref()) {
                walk(&path, depth + 1, out);
            }
        } else if ft.is_file() {
            out.push(path);
        }
    }
}

/// "~/code/nordlys" for display.
pub fn pretty_dir(path: &Path) -> String {
    let dir = path.parent().unwrap_or(path);
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if dir.starts_with(&home) => {
            let rest = dir.strip_prefix(&home).unwrap_or(dir);
            if rest.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~/{}", rest.display())
            }
        }
        _ => dir.display().to_string(),
    }
}
