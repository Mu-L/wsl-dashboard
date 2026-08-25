// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use tracing::info;
use super::models::MountParams;

// Allowed filesystem types whitelist
pub const ALLOWED_FILESYSTEMS: &[&str] = &["ext4", "xfs", "btrfs", "vfat", "ntfs"];

// Known simple mount options (no value)
const KNOWN_MOUNT_OPTIONS: &[&str] = &[
    "ro", "rw",
    "atime", "noatime", "relatime", "nodiratime", "strictatime", "lazytime", "nolazytime",
    "exec", "noexec",
    "suid", "nosuid",
    "dev", "nodev",
    "sync", "async", "dirsync",
    "auto", "noauto",
    "user", "nouser", "owner", "group",
    "defaults", "remount", "bind", "rbind", "move",
    "nofail", "_netdev",
    "discard", "nodiscard",
    "barrier", "nobarrier",
    "iversion", "noiversion",
    "mand", "nomand",
    "compress",
];

// Known key prefixes for key=value mount options
const KNOWN_MOUNT_OPTION_PREFIXES: &[&str] = &[
    "errors=",
    "data=",
    "compress=", "compress-force=",
    "subvol=", "subvolid=",
    "device=",
    "nfsvers=", "vers=", "port=", "rsize=", "wsize=",
    "uid=", "gid=", "umask=", "fmask=", "dmask=",
    "iocharset=", "codepage=",
    "sec=",
    "x-systemd.", "x-gvfs-",
];

// Validate mount options against a known whitelist.
// Each comma-separated option must be a known option or key=value prefix.
pub fn validate_mount_options(opts: &str) -> bool {
    if opts.trim().is_empty() {
        return true;
    }
    opts.split(',')
        .all(|opt| {
            let opt = opt.trim().to_lowercase();
            if opt.is_empty() {
                return false;
            }
            // Check exact match against known options
            if KNOWN_MOUNT_OPTIONS.contains(&opt.as_str()) {
                return true;
            }
            // Check prefix match for key=value options
            KNOWN_MOUNT_OPTION_PREFIXES.iter().any(|prefix| opt.starts_with(prefix))
        })
}

// Validate filesystem type string if provided
pub fn validate_filesystem(fs: &str) -> bool {
    let fs_lower = fs.to_lowercase();
    ALLOWED_FILESYSTEMS.contains(&fs_lower.as_str())
}

// Build wsl --mount command line string for physical disk
pub fn build_physical_mount_cmd(params: &MountParams) -> String {
    let mut cmd = format!("wsl --mount {}", params.disk_path);

    if params.bare {
        cmd.push_str(" --bare");
    } else {
        if let Some(ref name) = params.name {
            if !name.trim().is_empty() {
                cmd.push_str(&format!(" --name {}", name.trim()));
            }
        }
        if let Some(ref part) = params.partition {
            if !part.trim().is_empty() {
                cmd.push_str(&format!(" --partition {}", part.trim()));
            }
        }
        if let Some(ref fs) = params.filesystem {
            if !fs.trim().is_empty() {
                cmd.push_str(&format!(" --type {}", fs.trim()));
            }
        }
        if let Some(ref opts) = params.mount_options {
            if !opts.trim().is_empty() {
                cmd.push_str(&format!(" --options {}", opts.trim()));
            }
        }
    }

    cmd
}

// Build wsl --mount --vhd command args for VHD image
pub fn build_vhd_mount_args(params: &MountParams) -> Vec<String> {
    let mut args = vec![
        "--mount".to_string(),
        params.disk_path.clone(),
        "--vhd".to_string(),
    ];

    if params.bare {
        args.push("--bare".to_string());
    } else {
        if let Some(ref name) = params.name {
            if !name.trim().is_empty() {
                args.push("--name".to_string());
                args.push(name.trim().to_string());
            }
        }
        if let Some(ref part) = params.partition {
            if !part.trim().is_empty() {
                args.push("--partition".to_string());
                args.push(part.trim().to_string());
            }
        }
        if let Some(ref fs) = params.filesystem {
            if !fs.trim().is_empty() {
                args.push("--type".to_string());
                args.push(fs.trim().to_string());
            }
        }
        if let Some(ref opts) = params.mount_options {
            if !opts.trim().is_empty() {
                args.push("--options".to_string());
                args.push(opts.trim().to_string());
            }
        }
    }

    args
}

// Execute unmount command via wsl --unmount <disk_path>
pub fn execute_unmount(disk_path: &str, is_physical: bool) -> Result<(), String> {
    let mut clean_path = disk_path.replace('/', "\\");
    if clean_path.starts_with(r"\\?\") {
        clean_path = clean_path.trim_start_matches(r"\\?\").to_string();
    }

    let is_physical_drive = is_physical || clean_path.to_uppercase().contains("PHYSICALDRIVE");

    info!("Unmounting disk (physical: {}): {}", is_physical_drive, clean_path);

    // VHD/VHDX disks also need administrator elevation for detach, same as physical disks
        let cmd = format!("wsl --unmount {}", clean_path);
        info!("Executing wsl --unmount command: {}", cmd);
        crate::utils::system::run_invisible_elevated_command(&cmd)
}
