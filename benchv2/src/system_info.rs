use std::process::Command;

use crate::metrics::SystemInfo;

pub fn collect_system_info() -> SystemInfo {
    let mut system = sysinfo::System::new_all();
    system.refresh_cpu_all();
    system.refresh_memory();

    let total_memory = system.total_memory();
    let available_memory_raw = system.available_memory();
    let used_memory = if available_memory_raw == 0 {
        system.used_memory().min(total_memory)
    } else {
        total_memory.saturating_sub(available_memory_raw)
    };
    let available_memory = total_memory.saturating_sub(used_memory);
    let used_memory_percent = if total_memory > 0 {
        (used_memory as f64 / total_memory as f64) * 100.0
    } else {
        0.0
    };

    let cpu_model = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    SystemInfo {
        hostname,
        machine_model: detect_machine_model(),
        os_name: sysinfo::System::name().unwrap_or_else(|| "unknown".to_string()),
        os_version: sysinfo::System::long_os_version()
            .or_else(sysinfo::System::os_version)
            .unwrap_or_else(|| "unknown".to_string()),
        kernel_version: sysinfo::System::kernel_version().unwrap_or_else(|| "unknown".to_string()),
        architecture: std::env::consts::ARCH.to_string(),
        cpu_model,
        cpu_logical_cores: num_cpus::get(),
        cpu_physical_cores: num_cpus::get_physical(),
        total_memory_bytes: total_memory,
        available_memory_bytes: available_memory,
        used_memory_bytes: used_memory,
        used_memory_percent,
    }
}

fn detect_machine_model() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Some(model) = run_cmd("sysctl", &["-n", "hw.model"]) {
            return model;
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(model) = run_cmd(
            "sh",
            &[
                "-c",
                "cat /sys/devices/virtual/dmi/id/product_name 2>/dev/null",
            ],
        ) {
            return model;
        }
        if let Some(model) = run_cmd("hostnamectl", &["--json=short"]) {
            if !model.is_empty() {
                return model;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(model) =
            run_cmd("cmd", &["/C", "wmic computersystem get model | findstr /V Model"])
        {
            return model;
        }
    }

    "unknown".to_string()
}

fn run_cmd(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
