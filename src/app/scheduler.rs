// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use tracing::{info, error, debug};
use crate::config;
use crate::network;
use crate::app::WSL_INIT_SCRIPT;

// Execute network sync and USB auto-attach tasks triggered by Task Scheduler (/scheduler)
// Returns: exit code (0 = success, 1 = at least one error occurred)
pub async fn run_scheduler_task(args: &[String], pos: usize, config_manager: &config::ConfigManager) -> i32 {
    crate::utils::system::attach_console();

    info!(">>> [START] Network sync command detected via /scheduler <<<");
    let mut error_count = 0;

    // 4.0 Cleanup legacy startup scripts (vbs) asynchronously with timeout protection
    crate::utils::system::cleanup_legacy_vbs_startup();
    
    // 4.1 Load all necessary configurations
    let instances_path = config::ConfigManager::get_instances_path();
    let container = config::instances::load_instances(&instances_path);
    let net_config = config_manager.get_network_config();
    let rules = &net_config.port_proxies;
    
    // 4.2 Identify target distros for synchronization and auto-start
    let mut target_distros: std::collections::HashSet<String> = std::collections::HashSet::new();
    if pos + 1 < args.len() && !args[pos + 1].starts_with('/') {
        target_distros.insert(args[pos + 1].clone());
    } else {
        // Source 1: Distros with port proxy rules
        for r in rules {
            target_distros.insert(r.distro_name.clone());
        }

        // Source 2: Distros with auto-startup enabled
        for (name, inst) in &container.instances {
            if inst.auto_startup {
                target_distros.insert(name.clone());
            }
        }

        // Source 3: Distros with USB boot-attach configurations
        let usb_config = config_manager.get_usb_config();
        for device in usb_config.usb.values() {
            if device.boot_attach && !device.distribution.is_empty() {
                target_distros.insert(device.distribution.clone());
            }
        }
    }

    // 4.3 Background startup for target distros (if configured for auto-startup)
    let mut distros_spawned = 0;
    for name in &target_distros {
        if let Some(inst_config) = container.instances.get(name) {
            if inst_config.auto_startup {
                info!("Distro '{}' marked for auto-start. Spawning background init script...", name);
                #[cfg(windows)]
                {
                    use std::process::Command;
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    
                    let _ = Command::new("wsl")
                        .args(&["-d", name, "-u", "root", WSL_INIT_SCRIPT, "start"])
                        .creation_flags(CREATE_NO_WINDOW)
                        .spawn();
                    debug!("Executed command: wsl -d {} -u root {} start", name, WSL_INIT_SCRIPT);
                    distros_spawned += 1;
                }
            }
        }
    }
    
    // 4.4 Wait for network interfaces to stabilize if any distros were spawned
    if distros_spawned > 0 {
        info!("Waiting 5 seconds for {} spawned distros to stabilize network interfaces...", distros_spawned);
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    } else {
        info!("No new distros were spawned. Proceeding with existing running instances.");
    }
    
    // 4.5 Execute synchronization for each target distro
    for name in target_distros {
        let distro_rules: Vec<_> = rules.iter().filter(|r| r.distro_name == name).cloned().collect();
        if distro_rules.is_empty() {
            info!("No rules found for distro '{}', skipping.", name);
            continue;
        }
        
        info!(">>> Executing elevated sync for '{}' (Total Rules: {})", name, distro_rules.len());
        debug!("Elevated sync command: sync_port_proxies(distro=\"{}\", rules={})", name, distro_rules.len());
        // sync_port_proxies has internal 10-retry logic for IP fetching
        if let Err(e) = network::port_proxy::sync_port_proxies(&name, &distro_rules) {
            error!("Sync FAILED for '{}': {}", name, e);
            error_count += 1;
        } else {
            info!("Sync SUCCESS for '{}'.", name);
        }
    }
    
    // 4.6 Auto-attach USB devices if configured (boot-attach)
    info!(">>> [START] USB boot-attach synchronization <<<");
    let boot_devices = config_manager.get_usb_boot_attach_devices();
    
    if !boot_devices.is_empty() {
         // Check if any WSL 2 instance is running (required for usbipd attach)
        debug!("Checking running WSL instances via: wsl -l -v");
        let is_any_running = {
            let mut cmd = std::process::Command::new("wsl");
            cmd.args(["-l", "-v"]);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            cmd.env("WSL_UTF8", "1");
            
            match cmd.output() {
                Ok(out) => {
                    let stdout = crate::wsl::decoder::decode_output(&out.stdout);
                    stdout.lines()
                        .skip(1) // Skip header
                        .any(|line| {
                            let lower = line.to_lowercase();
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            // Must be Running AND Version 2
                            lower.contains("running") && parts.iter().any(|&p| p == "2")
                        })
                }
                Err(_) => false,
            }
        };
        
        if is_any_running {
            info!("Found running WSL 2 instance. Processing {} boot-attach device(s)...", boot_devices.len());
            
            // Build a single batch of USB attach commands to avoid multiple UAC prompts.
            // Each device triggers a separate UAC consent dialog via ShellExecuteExW(runas),
            // so batching all commands into one elevation ensures all devices are processed.
            let mut batch_commands: Vec<String> = Vec::with_capacity(boot_devices.len());
            for device in &boot_devices {
                info!("Boot-attaching USB device: BusId={}, VidPid={}, TargetDistro='{}'", 
                    device.bus_id, device.vid_pid, device.distribution);
                
                let force_flag = if device.force_bind { " --force" } else { "" };
                let auto_flag = if device.auto_attach { " --auto-attach" } else { "" };
                let distro_part = if device.distribution.is_empty() {
                    String::from("--wsl")
                } else {
                    format!("--wsl \"{}\"", device.distribution)
                };
                
                let cmd = format!(
                    "usbipd bind --busid {bus_id}{force} & usbipd attach {distro} --busid {bus_id}{auto}",
                    bus_id = device.bus_id,
                    force = force_flag,
                    distro = distro_part,
                    auto = auto_flag,
                );
                debug!("Queued USB attach command: {}", cmd);
                batch_commands.push(cmd);
            }
            
            // Execute all commands in a single UAC elevation
            match crate::utils::system::run_invisible_elevated_commands(batch_commands) {
                Ok(_) => {
                    for device in &boot_devices {
                        info!("SUCCESS: USB device {} attached.", device.bus_id);
                    }
                }
                Err(e) => {
                    error!("FAILED to batch attach USB devices: {}", e);
                    error_count += 1;
                }
            }
        } else {
            info!("No running WSL 2 instance found. Skipping USB boot-attach.");
        }
    } else {
        info!("No USB devices configured for boot-attach.");
    }
    info!(">>> [FINISH] USB boot-attach synchronization completed. <<<");
    
    info!(">>> [FINISH] All scheduled network tasks completed. <<<");

    // Return exit code based on whether any errors occurred
    if error_count > 0 {
        error!("Scheduler task completed with {} error(s).", error_count);
        1
    } else {
        0
    }
}
