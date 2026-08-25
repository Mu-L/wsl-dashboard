// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use std::os::windows::process::CommandExt;
use std::process::Command;
use tracing::info;
use super::models::{MountParams, MountedDiskRecord, PhysicalDiskRaw};
use super::operations::build_physical_mount_cmd;
use crate::config::mount::MountDiskConfig;

fn md5_hex(input: &[u8]) -> String {
    format!("{:x}", md5::compute(input))
}

const CREATE_NO_WINDOW: u32 = 0x08000000;

// Query the index of the Windows system disk (boot disk) via PowerShell
// The system disk cannot be mounted with wsl --mount, so it should be filtered out.
pub async fn get_system_disk_index() -> Result<u32, String> {
    tokio::task::spawn_blocking(|| {
        let ps_cmd = "Get-CimInstance -ClassName Win32_DiskPartition | Where-Object { $_.BootPartition -eq $true } | Select-Object -First 1 -ExpandProperty DiskIndex";
        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", ps_cmd])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("PowerShell query failed: {}", e))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("PowerShell returned error: {}", err.trim()));
        }

        let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if result.is_empty() {
            return Err("No boot disk found".to_string());
        }

        result.parse::<u32>()
            .map_err(|e| format!("Failed to parse disk index: {}", e))
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

// Query list of physical disks via PowerShell Get-CimInstance
pub async fn query_physical_disks() -> Result<Vec<PhysicalDiskRaw>, String> {
    tokio::task::spawn_blocking(|| {
        let ps_cmd = "Get-CimInstance -ClassName Win32_DiskDrive | Select-Object DeviceID, Model, Size, Index, InterfaceType, MediaType | ConvertTo-Json";
        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", ps_cmd])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("PowerShell query failed: {}", e))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("PowerShell returned error: {}", err.trim()));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        if json_str.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Handle single object vs array JSON response from PowerShell
        if json_str.trim().starts_with('[') {
            serde_json::from_str::<Vec<PhysicalDiskRaw>>(&json_str)
                .map_err(|e| format!("Failed to parse physical disks JSON array: {}", e))
        } else {
            serde_json::from_str::<PhysicalDiskRaw>(&json_str)
                .map(move |item| vec![item])
                .map_err(|e| format!("Failed to parse physical disk JSON object: {}", e))
        }
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
}

// Execute physical disk mount (requires Administrator privileges / UAC elevation)
pub async fn mount_physical_disk(params: MountParams) -> Result<MountedDiskRecord, String> {
    let cmd = build_physical_mount_cmd(&params);
    info!("Executing physical disk mount command: {}", cmd);

    tokio::task::spawn_blocking(move || {
        crate::utils::system::run_invisible_elevated_command(&cmd)?;

        let mount_id = format!("phys_{}", md5_hex(params.disk_path.as_bytes()));

        let mount_point = if params.bare {
            "/mnt/wsl (bare)".to_string()
        } else if let Some(ref n) = params.name {
            format!("/mnt/wsl/{}", n)
        } else {
            // WSL mounts the disk under /mnt/wsl/<DISK>p<partition> when
            // --partition is given, otherwise /mnt/wsl/<DISK>.
            let disk_name = params.disk_path.replace(r"\\.\", "");
            match params.partition {
                Some(ref p) if !p.trim().is_empty() => {
                    format!("/mnt/wsl/{}p{}", disk_name, p.trim())
                }
                _ => format!("/mnt/wsl/{}", disk_name),
            }
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
