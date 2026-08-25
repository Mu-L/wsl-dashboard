// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use std::os::windows::process::CommandExt;
use std::process::Command;
use tracing::info;
use super::models::{MountParams, MountedDiskRecord};
use super::operations::build_vhd_mount_args;
use crate::config::mount::MountDiskConfig;

fn md5_hex(input: &[u8]) -> String {
    format!("{:x}", md5::compute(input))
}

const CREATE_NO_WINDOW: u32 = 0x08000000;

// Open native file dialog to select VHD/VHDX file
pub fn select_vhd_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("VHD Files", &["vhdx", "vhd"])
        .set_title("Select VHD Image File")
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

// Execute VHD mount (does not require UAC elevation)
pub async fn mount_vhd_disk(params: MountParams) -> Result<MountedDiskRecord, String> {
    let args = build_vhd_mount_args(&params);
    info!("Executing VHD mount command: wsl {}", args.join(" "));

    tokio::task::spawn_blocking(move || {
        let output = Command::new("wsl")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to execute wsl command: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = if !stderr.trim().is_empty() { stderr } else { stdout };
            // Take only the first line of the error message
            let first_line = msg.trim().lines().next().unwrap_or("").to_string();
            return Err(first_line);
        }

        let file_name = std::path::Path::new(&params.disk_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| params.disk_path.clone());

        let safe_name = file_name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>();

        let mount_id = format!("vhd_{}", md5_hex(params.disk_path.as_bytes()));

        let mount_point = if params.bare {
            "/mnt/wsl (bare)".to_string()
        } else if let Some(ref n) = params.name {
            if !n.trim().is_empty() { format!("/mnt/wsl/{}", n) } else { format!("/mnt/wsl/{}", safe_name) }
        } else {
            format!("/mnt/wsl/{}", safe_name)
        };

        let record = MountedDiskRecord {
            mount_id,
            disk_path: params.disk_path.clone(),
            disk_type: params.disk_type.clone(),
            mount_point,
            mount_time: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            is_bare: params.bare,
            is_mounted: true,
            partition: params.partition.clone(),
            filesystem: params.filesystem.clone(),
            mount_options: params.mount_options.clone(),
            name: params.name.clone(),
        };

        let mut cfg = MountDiskConfig::load();
        cfg.add_or_update(record.clone());

        Ok(record)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}
