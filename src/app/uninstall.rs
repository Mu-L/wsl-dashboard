// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use tracing::{info, error, warn};
use std::fs;
use std::path::PathBuf;
use crate::app::autostart;
use crate::network::scheduler;
use crate::utils::system;

// Execute software uninstall cleanup logic
// Returns config directory path for final deletion after logging system is shutdown
pub async fn run_uninstall() -> Option<std::path::PathBuf> {
    info!(">>> [START] Executing cleanup process (/clean) <<<");

    // 1. Get config directory and paths
    let home_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let config_dir = home_dir.join(".wsldashboard");
    
    if !config_dir.exists() {
        warn!("Configuration directory {:?} not found, skipping config-based cleanup.", config_dir);
    } else {
        // 2. Clean up port forwarding and firewall rules
        cleanup_network_rules().await;

        // 3. Clean up USB bindings
        cleanup_usb_bindings().await;
    }

    // 4. Delete scheduled tasks
    cleanup_task_scheduler().await;

    // 5. Delete registry autostart entry
    cleanup_registry_autostart().await;

    info!(">>> [FINISH] System-level cleanup completed <<<");
    
    if config_dir.exists() {
        Some(config_dir)
    } else {
        None
    }
}

// Final cleanup logic executed after logging system is shutdown
// Since logging system is already closed, we use println! directly to output to console
pub fn final_cleanup(config_dir: Option<std::path::PathBuf>, delete_all: bool) {
    if let Some(path) = config_dir {
        if delete_all {
            // Give a little time for log file handles to be fully released
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Err(e) = fs::remove_dir_all(&path) {
                eprintln!("Failed to delete configuration directory {:?}: {}", path, e);
            } else {
                println!("Successfully deleted configuration directory.");
            }
        } else {
            println!("System cleanup completed. Configuration directory kept (run with /all to delete).");
        }
    }
}

async fn cleanup_network_rules() {
    info!("Cleaning up port forwarding and firewall rules...");
    
    // Manually load network config to avoid triggering ConfigManager's auto-repair/create logic
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let network_path = home_dir.join(".wsldashboard").join("network.toml");
    
    if !network_path.exists() {
        info!("network.toml not found, skipping port forwarding cleanup.");
        return;
    }

    match fs::read_to_string(&network_path) {
        Ok(content) => {
            if let Ok(config) = toml::from_str::<crate::config::models::NetworkConfig>(&content) {
                for rule in &config.port_proxies {
                    info!("Deleting port proxy: {}:{} -> {}:{}", rule.listen_address, rule.listen_port, rule.distro_name, rule.target_port);
                    
                    // 1. Delete netsh rules
                    let cmd = format!("netsh interface portproxy delete v4tov4 listenaddress={} listenport={}", 
                        rule.listen_address, rule.listen_port);
                    let _ = system::run_invisible_elevated_command(&cmd);

                    // 2. Delete firewall rule (if exists)
                    // The logic here references how rule_name is generated in src/network/port_proxy.rs
                    let rule_name = format!("WSL_Dashboard_{}_{}", rule.distro_name, rule.listen_port);
                    info!("Attempting to delete firewall rule: {}", rule_name);
                    let fw_cmd = format!("netsh advfirewall firewall delete rule name=\"{}\"", rule_name);
                    let _ = system::run_invisible_elevated_command(&fw_cmd);
                }
                info!("Port forwarding and firewall rules cleanup completed.");
            }
        }
        Err(e) => error!("Failed to read network.toml: {}", e),
    }
}

async fn cleanup_usb_bindings() {
    info!("Cleaning up USB bindings...");
    
    let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let settings_path = home_dir.join(".wsldashboard").join("usb.toml");
    
    if !settings_path.exists() {
        info!("usb.toml not found, skipping USB unbinding cleanup.");
        return;
    }

    match fs::read_to_string(&settings_path) {
        Ok(content) => {
            if let Ok(config) = toml::from_str::<crate::config::models::UsbConfigFile>(&content) {
                for device in config.usb.values() {
                    if device.bus_id.is_empty() { continue; }
                    info!("Performing unbind for device {} (BusId: {})", device.vid_pid, device.bus_id);
                    // Execute usbipd unbind --busid <bus_id>
                    let args = vec!["unbind".to_string(), "--busid".to_string(), device.bus_id.clone()];
                    if let Err(e) = system::run_command_with_elevation("usbipd", args) {
                        warn!("Failed to unbind device {} (may be disconnected): {}", device.bus_id, e);
                    } else {
                        info!("Unbind command sent for device {}.", device.bus_id);
                    }
                }
                info!("USB binding cleanup completed.");
            }
        }
        Err(e) => error!("Failed to read usb.toml: {}", e),
    }
}

async fn cleanup_task_scheduler() {
    info!("Deleting scheduled task: {}", scheduler::TASK_NAME);
    let cmd = format!("schtasks /Delete /TN \"{}\" /F", scheduler::TASK_NAME);
    if let Err(e) = system::run_invisible_elevated_command(&cmd) {
        warn!("Failed to delete scheduled task (may not exist): {}", e);
    } else {
        info!("Scheduled task deleted successfully.");
    }

    info!("Cleaning up user task directory: \\WSLDashboard\\UserTasks");
    // Write a temporary powershell script to unregister all tasks in the UserTasks directory
    // and then remove the directory itself.
    // This avoids cmd.exe quote stripping and pipe escaping issues
    let temp_ps1 = std::env::temp_dir().join("wsldashboard_uninstall_tasks.ps1");
    let ps1_content = "\
$tasks = Get-ScheduledTask -TaskPath '\\WSLDashboard\\UserTasks\\' -ErrorAction SilentlyContinue
if ($tasks) { $tasks | Unregister-ScheduledTask -Confirm:$false }
$service = New-Object -ComObject Schedule.Service
$service.Connect()
try {
    $rootFolder = $service.GetFolder('\\')
    $wslFolder = $rootFolder.GetFolder('WSLDashboard')
    try { $wslFolder.DeleteFolder('UserTasks', $null) } catch { }
    try { $rootFolder.DeleteFolder('WSLDashboard', $null) } catch { }
} catch { }
";
    if std::fs::write(&temp_ps1, ps1_content).is_ok() {
        let ps_cmd = format!("powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"", temp_ps1.display());
        let _ = system::run_invisible_elevated_command(&ps_cmd);
        let _ = std::fs::remove_file(&temp_ps1);
    }

    // Note: schtasks /Delete cannot remove a folder that still contains tasks.
    // The PowerShell script above unregisters all tasks and then removes the folder.

    // Delete the task_scheduler.ps1 script file (always delete regardless of CleanupData setting)
    let script_path = scheduler::get_script_path();
    if script_path.exists() {
        info!("Deleting task scheduler script: {:?}", script_path);
        if let Err(e) = fs::remove_file(&script_path) {
            warn!("Failed to delete task scheduler script: {}", e);
        } else {
            info!("Task scheduler script deleted successfully.");
            // Try to remove the scripts directory if it's now empty
            if let Some(parent) = script_path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    } else {
        info!("Task scheduler script not found at {:?}, skipping deletion.", script_path);
    }
}

async fn cleanup_registry_autostart() {
    info!("Deleting registry autostart entry...");
    // set_dashboard_autostart(false, false) handles registry deletion
    if let Err(e) = autostart::set_dashboard_autostart(false, false).await {
        warn!("Failed to delete registry autostart entry: {}", e);
    } else {
        info!("Registry autostart entry deleted successfully.");
    }
}
