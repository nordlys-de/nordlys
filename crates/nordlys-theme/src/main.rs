//! nordlys-theme: one palette, written out for every toolkit.
//!
//! Usage: nordlys-theme [--config PATH] [--out DIR] [--dry-run]
//!
//! `--out` defaults to `$XDG_CONFIG_HOME` (usually ~/.config), so the files
//! land where GTK, qt6ct and foot look for them.

use nordlys_config::{Config, Palette};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod targets;

struct Args {
    config: PathBuf,
    out: PathBuf,
    dry_run: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut config = nordlys_config::default_path();
    let mut out = nordlys_config::config_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut dry_run = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" => config = it.next().ok_or("--config needs a path")?.into(),
            "--out" => out = it.next().ok_or("--out needs a directory")?.into(),
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                println!("usage: nordlys-theme [--config PATH] [--out DIR] [--dry-run]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Args {
        config,
        out,
        dry_run,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("nordlys-theme: {e}");
            return ExitCode::from(2);
        }
    };
    let cfg = match Config::load(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nordlys-theme: {e}");
            return ExitCode::FAILURE;
        }
    };
    let p: Palette = cfg.palette();
    for (rel, body) in targets::all(&p) {
        let path = args.out.join(rel);
        if args.dry_run {
            println!("── {} ──\n{body}", path.display());
            continue;
        }
        if let Err(e) = write(&path, &body) {
            eprintln!("nordlys-theme: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {}", path.display());
    }
    ExitCode::SUCCESS
}

fn write(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, body)
}
