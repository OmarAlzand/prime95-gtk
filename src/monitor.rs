// Live CPU temperature and clock speed for the status bar.
// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;

/// CPU package temperature in °C (coretemp/k10temp), else the hottest thermal zone.
pub fn cpu_temp() -> Option<i32> {
    let read = |path: String| fs::read_to_string(path).ok()?.trim().parse::<i32>().ok();
    for hwmon in fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let dir = hwmon.path();
        let name = fs::read_to_string(dir.join("name")).unwrap_or_default();
        if matches!(name.trim(), "coretemp" | "k10temp" | "zenpower") {
            if let Some(t) = read(format!("{}/temp1_input", dir.display())) {
                return Some(t / 1000);
            }
        }
    }
    fs::read_dir("/sys/class/thermal")
        .ok()?
        .flatten()
        .filter_map(|zone| read(format!("{}/temp", zone.path().display())))
        .max()
        .map(|t| t / 1000)
}

/// Average current clock speed across all CPUs, in MHz.
pub fn avg_mhz() -> Option<u32> {
    let info = fs::read_to_string("/proc/cpuinfo").ok()?;
    let speeds: Vec<f64> = info
        .lines()
        .filter(|l| l.starts_with("cpu MHz"))
        .filter_map(|l| l.split(':').nth(1)?.trim().parse().ok())
        .collect();
    (!speeds.is_empty()).then(|| (speeds.iter().sum::<f64>() / speeds.len() as f64) as u32)
}

pub fn cpu_has(flag: &str) -> bool {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| info.lines().find(|l| l.starts_with("flags")).map(|l| l.split_whitespace().any(|f| f == flag)))
        .unwrap_or(false)
}
