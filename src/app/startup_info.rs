// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use tracing::info;

// Collect and log system version information asynchronously at startup.
// This runs in a background tokio task and does not block the startup flow.
pub async fn log_startup_system_info() {
    use tokio::process::Command;
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let app_version = env!("CARGO_PKG_VERSION");

    // Run wsl --version to get both WSL and Windows version info
    let wsl_output = async {
        let mut cmd = Command::new("wsl.exe");
        cmd.arg("--version")
            .env("WSL_UTF8", "1")
            .creation_flags(CREATE_NO_WINDOW)
            .kill_on_drop(true);
        cmd.output().await.ok()
    }
    .await;

    // --- Parse WSL version ---
    let wsl_version = wsl_output
        .as_ref()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // Find the line containing "WSL version:" but not other components
            stdout.lines().find_map(|line| {
                let lower = line.to_lowercase();
                if lower.contains("wsl")
                    && !lower.contains("wslg")
                    && !lower.contains("kernel")
                    && !lower.contains("windows")
                    && !lower.contains("direct3d")
                    && !lower.contains("dxcore")
                    && !lower.contains("msrdc")
                {
                    // Extract version number (e.g. "2.4.8.0" from "WSL version: 2.4.8.0")
                    line.split(|c: char| !c.is_ascii_digit() && c != '.')
                        .find(|token| {
                            !token.is_empty()
                                && token.contains('.')
                                && token.chars().all(|c| c.is_ascii_digit() || c == '.')
                        })
                        .map(|v| v.to_string())
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "N/A".to_string());

    // --- Parse Windows version ---
    let windows_version = match wsl_output
        .as_ref()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.lines().find_map(|line| {
                let lower = line.to_lowercase();
                if lower.contains("windows version") {
                    // Extract "10.0.26100.1" from "Windows version: 10.0.26100.1"
                    line.split(':').nth(1).map(|s| s.trim().to_string())
                } else {
                    None
                }
            })
        }) {
        Some(v) => v,
        None => {
            // Fallback: run ver command via cmd.exe
            let output = std::process::Command::new("cmd.exe")
                .args(["/c", "ver"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .ok();
            match output {
                Some(o) if o.status.success() => {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if !s.is_empty() { s } else { "N/A".to_string() }
                }
                _ => "N/A".to_string(),
            }
        }
    };

    info!(
        "[STARTUP] App: v{} | WSL: {} | Windows: {}",
        app_version, wsl_version, windows_version
    );
}