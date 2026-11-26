//! Time, time zone and locale through systemd's timedatectl and localectl.

use super::{SLOW, key_values, lookup, run, run_for};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DateTime {
    pub timezone: String,
    pub ntp: bool,
    pub can_ntp: bool,
    pub synced: bool,
    pub local_rtc: bool,
    pub zones: Vec<String>,
    pub locale: String,
    pub locales: Vec<String>,
}

pub fn read() -> Result<DateTime, String> {
    let kv = key_values(&run("timedatectl", &["show"])?);
    let zones = run("timedatectl", &["list-timezones"])
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    let locale = run("localectl", &["status"])
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.trim().strip_prefix("System Locale:"))
                .and_then(|v| v.split_whitespace().find_map(|p| p.strip_prefix("LANG=")))
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let locales = run("localectl", &["list-locales"])
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    Ok(DateTime {
        timezone: lookup(&kv, "Timezone").unwrap_or("UTC").to_owned(),
        ntp: lookup(&kv, "NTP") == Some("yes"),
        can_ntp: lookup(&kv, "CanNTP") == Some("yes"),
        synced: lookup(&kv, "NTPSynchronized") == Some("yes"),
        local_rtc: lookup(&kv, "LocalRTC") == Some("yes"),
        zones,
        locale,
        locales,
    })
}

pub fn set_timezone(zone: &str) -> Result<(), String> {
    run_for(SLOW, "timedatectl", &["set-timezone", zone]).map(|_| ())
}

pub fn set_ntp(on: bool) -> Result<(), String> {
    run_for(
        SLOW,
        "timedatectl",
        &["set-ntp", if on { "true" } else { "false" }],
    )
    .map(|_| ())
}

pub fn set_local_rtc(on: bool) -> Result<(), String> {
    run_for(
        SLOW,
        "timedatectl",
        &["set-local-rtc", if on { "1" } else { "0" }],
    )
    .map(|_| ())
}

/// `YYYY-MM-DD HH:MM[:SS]`, checked before asking systemd.
pub fn valid_time(s: &str) -> bool {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").is_ok()
        || chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").is_ok()
}

pub fn set_time(s: &str) -> Result<(), String> {
    if !valid_time(s) {
        return Err("Write the time as 2026-11-25 14:30".into());
    }
    run_for(SLOW, "timedatectl", &["set-time", s]).map(|_| ())
}

pub fn set_locale(lang: &str) -> Result<(), String> {
    run_for(SLOW, "localectl", &["set-locale", &format!("LANG={lang}")]).map(|_| ())
}

#[cfg(test)]
mod tests {
    #[test]
    fn times() {
        assert!(super::valid_time("2026-11-25 14:30"));
        assert!(super::valid_time("2026-11-25 14:30:05"));
        assert!(!super::valid_time("25/11/2026"));
    }
}
