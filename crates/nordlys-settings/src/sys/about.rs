//! What this machine is: from /proc, /sys, os-release and a few tools.

use super::{key_values, lookup, run};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct About {
    pub os: String,
    pub os_url: String,
    pub kernel: String,
    pub hostname: String,
    pub model: String,
    pub cpu: String,
    pub cores: usize,
    pub gpus: Vec<String>,
    pub memory_total: u64,
    pub memory_used: u64,
    pub swap_total: u64,
    pub disk_total: u64,
    pub disk_used: u64,
    pub uptime_secs: u64,
    pub compositor: String,
    pub session: String,
    pub user: String,
    pub shell: String,
}

pub fn read() -> Result<About, String> {
    let os = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .map(|s| key_values(&s))
        .unwrap_or_default();
    let mem = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |key: &str| -> u64 {
        mem.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.split_whitespace().next()?.parse().ok())
            .unwrap_or(0)
            * 1024
    };
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let cpu = cpuinfo
        .lines()
        .find_map(|l| l.strip_prefix("model name"))
        .map(|v| v.trim_start_matches([' ', '\t', ':']).trim().to_owned())
        .unwrap_or_default();
    let (disk_total, disk_used) = disk("/").unwrap_or((0, 0));
    let read = |p: &str| {
        std::fs::read_to_string(p)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    let vendor = read("/sys/class/dmi/id/sys_vendor");
    let product = read("/sys/class/dmi/id/product_name");
    let model = [vendor, product]
        .into_iter()
        .filter(|s| !s.is_empty() && !s.contains("To be filled"))
        .collect::<Vec<_>>()
        .join(" ");
    let compositor = if super::on_hyprland() {
        run("hyprctl", &["version", "-j"])
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .and_then(|v| {
                v.get("tag")
                    .and_then(|t| t.as_str())
                    .map(|t| format!("Hyprland {t}"))
            })
            .unwrap_or_else(|| "Hyprland".into())
    } else if std::path::Path::new(&nordlys_socket()).exists() {
        "nordlys-comp".into()
    } else {
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "Unknown".into())
    };
    Ok(About {
        os: lookup(&os, "PRETTY_NAME")
            .or(lookup(&os, "NAME"))
            .unwrap_or("Linux")
            .to_owned(),
        os_url: lookup(&os, "HOME_URL").unwrap_or_default().to_owned(),
        kernel: read("/proc/sys/kernel/osrelease"),
        hostname: read("/proc/sys/kernel/hostname"),
        model,
        cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        cpu,
        gpus: gpus(),
        memory_total: kb("MemTotal:"),
        memory_used: kb("MemTotal:").saturating_sub(kb("MemAvailable:")),
        swap_total: kb("SwapTotal:"),
        disk_total,
        disk_used,
        uptime_secs: read("/proc/uptime")
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        compositor,
        session: std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
        user: std::env::var("USER").unwrap_or_default(),
        shell: std::env::var("SHELL").unwrap_or_default(),
    })
}

fn nordlys_socket() -> String {
    format!(
        "{}/nordlys.sock",
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())
    )
}

fn disk(path: &str) -> Option<(u64, u64)> {
    // df is everywhere and saves unsafe statvfs calls.
    let out = run("df", &["-B1", "--output=size,used", path]).ok()?;
    let line = out.lines().nth(1)?;
    let mut f = line.split_whitespace();
    Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
}

fn gpus() -> Vec<String> {
    let Ok(out) = run("lspci", &["-mm"]) else {
        return vec![];
    };
    out.lines()
        .filter(|l| {
            l.contains("VGA") || l.contains("3D controller") || l.contains("Display controller")
        })
        .filter_map(|l| {
            // 00:02.0 "VGA compatible controller" "Intel Corporation" "Device name" …
            let q: Vec<&str> = l.split('"').collect();
            let vendor = q
                .get(3)?
                .replace(" Corporation", "")
                .replace(" [AMD/ATI]", "");
            let device = q.get(5)?;
            Some(format!("{vendor} {device}"))
        })
        .collect()
}

/// 16 GB, 512 MB.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 10.0 || i == 0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// 3 days, 4 hours · 2 hours, 5 minutes · 12 minutes.
pub fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    let unit = |n: u64, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    match (d, h) {
        (0, 0) => unit(m, "minute"),
        (0, _) => format!("{}, {}", unit(h, "hour"), unit(m, "minute")),
        _ => format!("{}, {}", unit(d, "day"), unit(h, "hour")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_durations() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(16 * 1024 * 1024 * 1024), "16 GB");
        assert_eq!(bytes(1536 * 1024 * 1024), "1.5 GB");
        assert_eq!(duration(60), "1 minute");
        assert_eq!(duration(2 * 3600 + 5 * 60), "2 hours, 5 minutes");
        assert_eq!(duration(3 * 86400 + 3600), "3 days, 1 hour");
    }

    #[test]
    fn reads_this_machine() {
        let a = read().unwrap();
        assert!(!a.kernel.is_empty());
        assert!(a.memory_total > 0);
    }
}
