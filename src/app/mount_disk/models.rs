// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

// Raw physical disk data retrieved from PowerShell Win32_DiskDrive
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalDiskRaw {
    #[serde(rename = "DeviceID", default)]
    pub device_id: String,
    #[serde(rename = "Model", default)]
    pub model: String,
    #[serde(rename = "Size", default)]
    pub size: u64,
    #[serde(rename = "Index", default)]
    pub index: u32,
    #[serde(rename = "InterfaceType", default)]
    pub interface_type: String,
    #[serde(rename = "MediaType", default)]
    pub media_type: String,
}

// Parameters passed for mounting a disk
#[derive(Debug, Clone, Default)]
pub struct MountParams {
    pub disk_path: String,
    pub disk_type: String, // "physical" or "vhd"
    pub partition: Option<String>,
    pub filesystem: Option<String>,
    pub mount_options: Option<String>,
    pub name: Option<String>,
    pub bare: bool,
}

// Persisted record of a mounted disk in mount.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MountedDiskRecord {
    pub mount_id: String,
    pub disk_path: String,
    pub disk_type: String, // "physical" or "vhd"
    pub mount_point: String,
    pub mount_time: String,
    pub is_bare: bool,
    #[serde(default = "default_mounted_true")]
    pub is_mounted: bool,
    #[serde(default)]
    pub partition: Option<String>,
    #[serde(default)]
    pub filesystem: Option<String>,
    #[serde(default)]
    pub mount_options: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

fn default_mounted_true() -> bool {
    true
}
