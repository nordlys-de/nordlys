//! Which graphics driver the always-running pieces load.
//!
//! The Vulkan loader opens every installed driver, and on a laptop with an
//! Intel and an NVIDIA GPU, NVIDIA's costs each shell process some 50 MB
//! of memory (and video memory) to draw a bar. So where there's an Intel
//! GPU, those pieces load only Intel's driver.
//!
//! The variables are read by this process alone: [`restore`] takes them
//! out again for the apps it starts. `NORDLYS_SHELL_GPU=auto` turns all
//! of this off, and so does setting any of [`CHOSEN`] yourself.

use std::path::Path;
use std::process::Command;

/// Turns this off when set to `auto`.
const SWITCH: &str = "NORDLYS_SHELL_GPU";
/// Names what we set, so a supervisor's children know it too.
const MARKER: &str = "NORDLYS_GPU_SET";
/// Someone who sets one of these has made their own choice.
const CHOSEN: [&str; 8] = [
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_LOADER_DRIVERS_SELECT",
    "VK_LOADER_DRIVERS_DISABLE",
    "WGPU_BACKEND",
    "WGPU_ADAPTER_NAME",
    "WGPU_POWER_PREF",
    "ICED_BACKEND",
];
const ICD_DIR: &str = "/usr/share/vulkan/icd.d";
/// Current Intel GPUs, then the driver for the Haswell and Broadwell ones.
/// Distributions differ on the architecture suffix.
const ICDS: [&str; 4] = [
    "intel_icd.json",
    "intel_icd.x86_64.json",
    "intel_hasvk_icd.json",
    "intel_hasvk_icd.x86_64.json",
];

/// Keep this process to Intel's driver, if that's possible and wanted.
///
/// # Safety
///
/// Changes the environment, so no other thread may be running yet.
pub unsafe fn prefer_intel() {
    let set = plan(
        |k| std::env::var(k).ok(),
        &intel_icds(Path::new(ICD_DIR)),
        has_intel_gpu(Path::new("/sys/class/drm")),
    );
    if set.is_empty() {
        return;
    }
    let names: Vec<&str> = set.iter().map(|(k, _)| *k).collect();
    // SAFETY: the caller has no other threads.
    unsafe {
        std::env::set_var(MARKER, names.join(","));
        for (key, value) in &set {
            std::env::set_var(key, value);
        }
    }
}

/// Give a command we start the environment we were given.
pub fn restore(cmd: &mut Command) {
    let marker = std::env::var(MARKER).ok();
    for key in added(marker.as_deref()) {
        cmd.env_remove(key);
    }
}

/// The variables to set.
fn plan(
    var: impl Fn(&str) -> Option<String>,
    icds: &[String],
    intel: bool,
) -> Vec<(&'static str, String)> {
    if !intel
        || icds.is_empty()
        || var(SWITCH).as_deref() == Some("auto")
        || CHOSEN.iter().any(|k| var(k).is_some())
    {
        return vec![];
    }
    vec![
        ("VK_DRIVER_FILES", icds.join(":")),
        // Without this wgpu also tries OpenGL, which loads NVIDIA's again.
        ("WGPU_BACKEND", "vulkan".to_owned()),
    ]
}

/// The variables named by the marker, and the marker. Only ever ones we
/// set, whatever the marker says.
fn added(marker: Option<&str>) -> Vec<&str> {
    let Some(marker) = marker else {
        return vec![];
    };
    marker
        .split(',')
        .filter(|k| CHOSEN.contains(k))
        .chain([MARKER])
        .collect()
}

fn intel_icds(dir: &Path) -> Vec<String> {
    ICDS.iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn has_intel_gpu(drm: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return false;
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        .any(|e| {
            std::fs::read_to_string(e.path().join("device/vendor"))
                .is_ok_and(|v| v.trim() == "0x8086")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icds() -> Vec<String> {
        vec!["/a.json".into(), "/b.json".into()]
    }

    #[test]
    fn keeps_to_intel_when_there_is_one() {
        assert_eq!(
            plan(|_| None, &icds(), true),
            [
                ("VK_DRIVER_FILES", "/a.json:/b.json".to_owned()),
                ("WGPU_BACKEND", "vulkan".to_owned()),
            ]
        );
    }

    #[test]
    fn leaves_other_machines_alone() {
        assert!(plan(|_| None, &icds(), false).is_empty());
        assert!(plan(|_| None, &[], true).is_empty());
    }

    #[test]
    fn leaves_the_users_choice_alone() {
        for chosen in CHOSEN {
            let var = |k: &str| (k == chosen).then(|| "x".to_owned());
            assert!(plan(var, &icds(), true).is_empty(), "{chosen}");
        }
        let off = |k: &str| (k == SWITCH).then(|| "auto".to_owned());
        assert!(plan(off, &icds(), true).is_empty());
    }

    #[test]
    fn takes_back_only_what_it_set() {
        assert!(added(None).is_empty());
        assert_eq!(
            added(Some("VK_DRIVER_FILES,WGPU_BACKEND,PATH")),
            ["VK_DRIVER_FILES", "WGPU_BACKEND", MARKER]
        );
    }
}
