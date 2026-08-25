// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use tracing::{error, info};
use slint::{Model, ModelRc, VecModel};
use crate::{AppWindow, AppState, PhysicalDiskInfo, MountedDiskInfo, i18n};
use crate::api::common::wslui_helper_mount;
use crate::app::mount_disk::{
    MountParams, query_physical_disks, get_system_disk_index,
    mount_physical_disk, mount_vhd_disk, select_vhd_file,
    operations::{execute_unmount, validate_filesystem, validate_mount_options},
};
use crate::config::mount::MountDiskConfig;
use crate::wsl::ops::lifecycle::{shutdown_wsl, start_distro};
use crate::wsl::ops::info::{list_running_distros, get_default_distro_name};

// Pending mount params for retry after mount failure
static PENDING_RETRY_MOUNT: OnceLock<tokio::sync::Mutex<Option<MountParams>>> = OnceLock::new();

// Set to `false` to include USB disks in the mount disk dialog (useful for testing)
const FILTER_OUT_USB_DISKS: bool = true;

fn pending_retry_lock() -> &'static tokio::sync::Mutex<Option<MountParams>> {
    PENDING_RETRY_MOUNT.get_or_init(|| tokio::sync::Mutex::new(None))
}

fn show_toast(ah: slint::Weak<AppWindow>, msg: String) {
    crate::ui::handlers::network::utils::show_toast(ah, msg);
}

// Refresh mounted disks list in Slint UI from mount.toml,
// with real state checking against running WSL distro.
pub fn refresh_mounted_disks(app_handle: slint::Weak<AppWindow>, app_state: Arc<Mutex<AppState>>) {
    let ah = app_handle.clone();
    let as_ptr = app_state.clone();

    tokio::spawn(async move {
        let mut cfg = MountDiskConfig::load();

        // 1. Get the WSL executor to check real mount state
        let executor = {
            let state = as_ptr.lock().await;
            state.wsl_dashboard.executor().clone()
        };

        // 2. Get running distros (cross-locale safe)
        let mut running_distros = list_running_distros(&executor).await;

        // 3. If there are mount configurations but no distro is running,
        // auto-start the default Linux to probe mount points (only if enabled)
        let mount_auto_probe = {
            let state = as_ptr.lock().await;
            state.config_manager.get_settings().mount_auto_probe
        };
        if !cfg.mounted_disks.is_empty() && running_distros.is_empty() && mount_auto_probe {
            if let Some(default_distro) = get_default_distro_name(&executor).await {
                info!(
                    "No running WSL distro, starting default '{}' to probe mount points",
                    default_distro
                );
                let _ = start_distro(&executor, &default_distro).await;
                running_distros = list_running_distros(&executor).await;
            }
        }

        // 4. Check actual mount state
        if let Some(distro) = running_distros.first() {
            for record in cfg.mounted_disks.iter_mut() {
                // Bare mounts have no real mount point directory, so skip the test -d check
                // and trust the is_mounted flag set during mount/unmount operations.
                if record.is_bare {
                    continue;
                }
                let result = executor.execute_command(&[
                    "-d", distro, "-u", "root", "-e", "test", "-d", &record.mount_point,
                ]).await;
                let is_actually_mounted = result.success;
                if record.is_mounted != is_actually_mounted {
                    info!(
                        "Mount point '{}' actual state: {} (was: {})",
                        record.mount_point,
                        if is_actually_mounted { "mounted" } else { "unmounted" },
                        if record.is_mounted { "mounted" } else { "unmounted" },
                    );
                    record.is_mounted = is_actually_mounted;
                }
            }
        }

        // 5. Save updated config
        let _ = cfg.save();

        // 6. Render UI (sorted by mount_id ascending)
        let mut ui_records: Vec<MountedDiskInfo> = cfg.mounted_disks.into_iter().map(|rec| {
            MountedDiskInfo {
                mount_id: rec.mount_id.into(),
                disk_path: rec.disk_path.into(),
                disk_type: rec.disk_type.into(),
                mount_point: rec.mount_point.into(),
                mount_time: rec.mount_time.into(),
                name: rec.name.unwrap_or_default().into(),
                is_bare: rec.is_bare,
                is_mounted: rec.is_mounted,
            }
        }).collect();
        ui_records.sort_by(|a, b| a.mount_id.cmp(&b.mount_id));

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = ah.upgrade() {
                app.set_mount_disk_mounted_list(ModelRc::new(VecModel::from(ui_records)));
            }
        });
    });
}

// Mount the disk. If there are running distros, mount directly without
// shutdown (keep them running). If no running distros, execute wsl --shutdown
// first to ensure clean VM state.
// On success: toast + refresh.
// On failure: save params for retry and show restart dialog.
async fn run_shutdown_and_mount(
    ah: slint::Weak<AppWindow>,
    as_ptr: Arc<Mutex<AppState>>,
    params: MountParams,
) {
    // 1. Check if there are running distros
    let executor = {
        let state = as_ptr.lock().await;
        state.wsl_dashboard.executor().clone()
    };
    let running_distros = list_running_distros(&executor).await;
    if running_distros.is_empty() {
        // No running distro → shutdown first to ensure clean VM state
        info!("No running WSL distro, executing wsl --shutdown before mount");
        let _ = shutdown_wsl(&executor).await;
    } else {
        info!("Running distro(s) detected, mounting directly without shutdown");
    }

    // 2. Execute mount
    let result = if params.disk_type == "physical" {
        mount_physical_disk(params.clone()).await
    } else {
        mount_vhd_disk(params.clone()).await
    };

    match result {
        Ok(_) => {
            show_toast(ah.clone(), i18n::t("mount_disk.mount_success"));
            refresh_mounted_disks(ah.clone(), as_ptr.clone());
        }
        Err(e) => {
            error!("Mount error: {}", e);
            // Store params for retry
            let mut lock = pending_retry_lock().lock().await;
            *lock = Some(params);
            // Show restart dialog
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = ah.upgrade() {
                    app.set_show_mount_restart_confirm(true);
                }
            });
        }
    }
}

pub fn setup(app: &AppWindow, app_handle: slint::Weak<AppWindow>, app_state: Arc<Mutex<AppState>>) {
    // Refresh list callback
    let ah = app_handle.clone();
    let as_ptr = app_state.clone();
    app.on_refresh_mount_disk_list(move || {
        refresh_mounted_disks(ah.clone(), as_ptr.clone());
    });

    // Open Mount Disk dialog callback
    let ah = app_handle.clone();
    app.on_open_mount_disk_dialog(move || {
        show_toast(ah.clone(), i18n::t("mount_disk.querying_data"));
        let ah2 = ah.clone();
        slint::spawn_local(async move {
            let raw_disks_result = query_physical_disks().await;
            let system_disk_index = get_system_disk_index().await.unwrap_or(u32::MAX);

            let physical_info_list: Vec<PhysicalDiskInfo> = match raw_disks_result {
                Ok(disks) => disks.into_iter()
                    .filter(|d| (!FILTER_OUT_USB_DISKS || d.interface_type.to_uppercase() != "USB") && d.index != system_disk_index)
                    .map(|d| {
                        let size_gb = (d.size as f64) / (1024.0 * 1024.0 * 1024.0);
                        let size_str = format!("{:.2} GB", size_gb);
                        let display_text = format!("{} ({}) - {}", d.model.trim(), size_str, d.device_id.trim());
                        let is_usb = d.interface_type.to_uppercase().contains("USB");
                        PhysicalDiskInfo {
                            device_id: d.device_id.into(),
                            model: d.model.into(),
                            size: size_str.into(),
                            index: d.index as i32,
                            interface_type: d.interface_type.into(),
                            is_usb,
                            display_text: display_text.into(),
                        }
                    }).collect(),
                Err(e) => {
                    error!("Failed to query physical disks: {}", e);
                    Vec::new()
                }
            };

            let cfg = MountDiskConfig::load();
            let mut mounted_info_list: Vec<MountedDiskInfo> = cfg.mounted_disks.into_iter().map(|rec| {
                MountedDiskInfo {
                    mount_id: rec.mount_id.into(),
                    disk_path: rec.disk_path.into(),
                    disk_type: rec.disk_type.into(),
                    mount_point: rec.mount_point.into(),
                    mount_time: rec.mount_time.into(),
                    name: rec.name.unwrap_or_default().into(),
                    is_bare: rec.is_bare,
                    is_mounted: rec.is_mounted,
                }
            }).collect();
            mounted_info_list.sort_by(|a, b| a.mount_id.cmp(&b.mount_id));

            let physical_texts: Vec<slint::SharedString> = physical_info_list.iter().map(|d| d.display_text.clone()).collect();

            let physical_disks_model = ModelRc::new(VecModel::from(physical_info_list));

            if let Some(app) = ah2.upgrade() {
                app.set_mount_disk_physical_disks(physical_disks_model.clone());
                app.set_mount_disk_physical_disk_texts(ModelRc::new(VecModel::from(physical_texts)));
                app.set_mount_disk_mounted_list(ModelRc::new(VecModel::from(mounted_info_list)));
                app.set_mount_disk_dialog_selected_disk_idx(0);
                app.set_mount_disk_dialog_vhd_path("".into());
                app.set_mount_disk_dialog_name("".into());
                app.set_mount_disk_dialog_partition("".into());
                app.set_mount_disk_dialog_filesystem("".into());
                app.set_mount_disk_dialog_options("".into());
                app.set_mount_disk_dialog_bare(false);
                app.set_mount_disk_dialog_tab(0);
                app.set_task_status_visible(false);
                app.set_show_mount_disk_dialog(true);
            }
        }).unwrap();

        // Fetch mount help URL asynchronously (non-blocking)
        let ah = ah.clone();
        tokio::spawn(async move {
            let mount_help_url = wslui_helper_mount().mount_disk.url;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = ah.upgrade() {
                    app.set_mount_disk_help_url(mount_help_url.into());
                }
            });
        });
    });

    // Close Mount Disk dialog callback
    let ah = app_handle.clone();
    app.on_close_mount_disk_dialog(move || {
        if let Some(app) = ah.upgrade() {
            app.set_show_mount_disk_dialog(false);
        }
    });

    // Select VHD file dialog callback
    let ah = app_handle.clone();
    app.on_mount_disk_select_vhd(move || {
        let ah = ah.clone();
        tokio::spawn(async move {
            if let Some(path) = select_vhd_file() {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = ah.upgrade() {
                        app.set_mount_disk_dialog_vhd_path(path.into());
                    }
                });
            }
        });
    });

    // Confirm Mount callback (dialog mount)
    let ah = app_handle.clone();
    let as_ptr = app_state.clone();
    app.on_mount_disk_confirm_mount(move || {
        let ah = ah.clone();
        let as_ptr = as_ptr.clone();
        slint::spawn_local(async move {
            if let Some(app) = ah.upgrade() {
                let tab = app.get_mount_disk_dialog_tab();
                let bare = app.get_mount_disk_dialog_bare();
                let name = app.get_mount_disk_dialog_name().to_string();
                let partition = app.get_mount_disk_dialog_partition().to_string();
                let filesystem = app.get_mount_disk_dialog_filesystem().to_string();
                let mount_options = app.get_mount_disk_dialog_options().to_string();

                // For VHD tab, check file selection first
                if tab == 0 {
                    let vhd_path = app.get_mount_disk_dialog_vhd_path().to_string();
                    if vhd_path.trim().is_empty() {
                        show_toast(ah.clone(), i18n::t("mount_disk.vhd_path"));
                        return;
                    }
                }

                // Validate name (required when not bare mode)
                if !bare {
                    let name = name.trim();
                    if name.is_empty() {
                        show_toast(ah.clone(), i18n::t("dialog.name_required"));
                        return;
                    }
                    let is_valid_chars = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
                    if !is_valid_chars || name.len() > 25 {
                        show_toast(ah.clone(), i18n::t("dialog.install_name_invalid"));
                        return;
                    }
                }

                // Check if name already exists in config
                if !bare {
                    let name = name.trim();
                    if !name.is_empty() {
                        let cfg = MountDiskConfig::load();
                        let name_exists = cfg.mounted_disks.iter().any(|r| r.name.as_deref() == Some(name));
                        if name_exists {
                            let dup_name = name.to_string();
                            show_toast(ah.clone(), i18n::tr("mount_disk.error_name_duplicate", &[dup_name]));
                            return;
                        }
                    }
                }

                if !filesystem.trim().is_empty() && !validate_filesystem(&filesystem) {
                    show_toast(ah.clone(), i18n::t("mount_disk.error_invalid_filesystem"));
                    return;
                }

                // Validate partition number if provided
                if !partition.trim().is_empty() {
                    let partition_str = partition.trim();
                    match partition_str.parse::<u32>() {
                        Ok(num) if num >= 1 && num <= 9999 => {}
                        _ => {
                            show_toast(ah.clone(), i18n::t("mount_disk.error_invalid_partition"));
                            return;
                        }
                    }
                }

                // Validate mount options format if provided
                if !mount_options.trim().is_empty() && !validate_mount_options(&mount_options) {
                    show_toast(ah.clone(), i18n::t("mount_disk.error_invalid_options"));
                    return;
                }

                // Pre-check: disk already mounted
                let disk_path_to_check = if tab == 1 {
                    let disk_idx = app.get_mount_disk_dialog_selected_disk_idx() as usize;
                    let physical_disks = app.get_mount_disk_physical_disks();
                    if disk_idx < physical_disks.row_count() {
                        physical_disks.row_data(disk_idx).map(|d| d.device_id.to_string())
                    } else {
                        None
                    }
                } else {
                    let vhd_path = app.get_mount_disk_dialog_vhd_path().to_string();
                    if vhd_path.trim().is_empty() { None } else { Some(vhd_path) }
                };

                if let Some(ref check_path) = disk_path_to_check {
                    let cfg = MountDiskConfig::load();
                    let already_mounted = cfg.mounted_disks.iter().any(|r| r.is_mounted && r.disk_path == *check_path);
                    if already_mounted {
                        show_toast(ah.clone(), i18n::t("mount_disk.error_already_mounted"));
                        return;
                    }
                }

                // Build MountParams based on active tab
                let params = if tab == 1 {
                    // Physical Disk
                    let disk_idx = app.get_mount_disk_dialog_selected_disk_idx() as usize;
                    let physical_disks = app.get_mount_disk_physical_disks();
                    if disk_idx >= physical_disks.row_count() {
                        show_toast(ah.clone(), i18n::t("mount_disk.no_disks_found"));
                        return;
                    }
                    match physical_disks.row_data(disk_idx) {
                        Some(disk_info) => MountParams {
                            disk_path: disk_info.device_id.to_string(),
                            disk_type: "physical".to_string(),
                            partition: if partition.trim().is_empty() { None } else { Some(partition) },
                            filesystem: if filesystem.trim().is_empty() { None } else { Some(filesystem) },
                            mount_options: if mount_options.trim().is_empty() { None } else { Some(mount_options) },
                            name: if name.trim().is_empty() { None } else { Some(name) },
                            bare,
                        },
                        None => {
                            show_toast(ah.clone(), i18n::t("mount_disk.no_disks_found"));
                            return;
                        }
                    }
                } else {
                    // VHD
                    MountParams {
                        disk_path: app.get_mount_disk_dialog_vhd_path().to_string(),
                        disk_type: "vhd".to_string(),
                        partition: if partition.trim().is_empty() { None } else { Some(partition) },
                        filesystem: if filesystem.trim().is_empty() { None } else { Some(filesystem) },
                        mount_options: if mount_options.trim().is_empty() { None } else { Some(mount_options) },
                        name: if name.trim().is_empty() { None } else { Some(name) },
                        bare,
                    }
                };

                info!("Mounting disk: {:?}", params);

                // Close the dialog and proceed directly (wsl --shutdown is always safe)
                app.set_show_mount_disk_dialog(false);
                run_shutdown_and_mount(ah.clone(), as_ptr.clone(), params).await;
            }
        }).unwrap();
    });

    // Open Mount Help URL callback
    let ah = app_handle.clone();
    app.on_mount_disk_open_help_url(move || {
        let ah = ah.clone();
        slint::spawn_local(async move {
            if let Some(app) = ah.upgrade() {
                let url = app.get_mount_disk_help_url().to_string();
                if !url.is_empty() {
                    let _ = open::that(url);
                }
            }
        }).unwrap();
    });

    // Confirm mount restart (wsl --shutdown + retry)
    let ah = app_handle.clone();
    let as_ptr = app_state.clone();
    app.on_confirm_mount_restart(move || {
        let ah = ah.clone();
        let as_ptr = as_ptr.clone();
        tokio::spawn(async move {
            let params = {
                let mut lock = pending_retry_lock().lock().await;
                lock.take()
            };
            if let Some(params) = params {
                info!("Retrying mount after wsl --shutdown");
                let executor = {
                    let state = as_ptr.lock().await;
                    state.wsl_dashboard.executor().clone()
                };
                let _ = shutdown_wsl(&executor).await;
                let result = if params.disk_type == "physical" {
                    mount_physical_disk(params.clone()).await
                } else {
                    mount_vhd_disk(params.clone()).await
                };
                match result {
                    Ok(_) => {
                        show_toast(ah.clone(), i18n::t("mount_disk.mount_success"));
                        refresh_mounted_disks(ah.clone(), as_ptr.clone());
                    }
                    Err(e) => {
                        error!("Mount retry also failed: {}", e);
                        show_toast(ah.clone(), i18n::t("mount_disk.error_restart_wsl"));
                    }
                }
            } else {
                error!("No pending retry mount found");
            }
        });
    });

    // Cancel mount restart
    app.on_cancel_mount_restart(move || {
        tokio::spawn(async move {
            let mut lock = pending_retry_lock().lock().await;
            lock.take();
        });
    });

    // Unmount from Card List (only unmount, keep config, mark as unmounted)
    let ah = app_handle.clone();
    let as_ptr = app_state.clone();
    app.on_mount_disk_unmount_from_list(move |disk_path| {
        let ah = ah.clone();
        let as_ptr = as_ptr.clone();
        let path = disk_path.to_string();
        slint::spawn_local(async move {
            let cfg = MountDiskConfig::load();
            let is_physical = cfg.mounted_disks.iter().any(|item| {
                (item.disk_path == path || item.mount_id == path) && item.disk_type == "physical"
            });

            match execute_unmount(&path, is_physical) {
                Ok(_) => {
                    let mut cfg = MountDiskConfig::load();
                    cfg.set_mount_status(&path, false);
                    show_toast(ah.clone(), i18n::t("mount_disk.unmount_success"));
                    refresh_mounted_disks(ah.clone(), as_ptr.clone());
                }
                Err(_e) => {
                    // If unmount fails, still mark as unmounted since the disk might already be detached
                    let mut cfg = MountDiskConfig::load();
                    cfg.set_mount_status(&path, false);
                    show_toast(ah.clone(), i18n::t("mount_disk.error_unmount_failed"));
                    refresh_mounted_disks(ah.clone(), as_ptr.clone());
                }
            }
        }).unwrap();
    });

    // Delete from Card List (unmount + remove from config permanently)
    let ah = app_handle.clone();
    let as_ptr = app_state.clone();
    app.on_mount_disk_delete_from_list(move |disk_path| {
        let ah = ah.clone();
        let as_ptr = as_ptr.clone();
        let path = disk_path.to_string();
        slint::spawn_local(async move {
            let cfg = MountDiskConfig::load();
            let is_physical = cfg.mounted_disks.iter().any(|item| {
                (item.disk_path == path || item.mount_id == path) && item.disk_type == "physical"
            });

            let _ = execute_unmount(&path, is_physical);
            let mut cfg = MountDiskConfig::load();
            cfg.remove(&path);
            show_toast(ah.clone(), i18n::t("mount_disk.unmount_success"));
            refresh_mounted_disks(ah.clone(), as_ptr.clone());
        }).unwrap();
    });

    // Mount from Card List (remount using stored params)
    let ah = app_handle.clone();
    let as_ptr = app_state.clone();
    app.on_mount_disk_mount_from_list(move |disk_path| {
        let ah = ah.clone();
        let as_ptr = as_ptr.clone();
        let path = disk_path.to_string();
        slint::spawn_local(async move {
            let cfg = MountDiskConfig::load();
            let record = cfg.mounted_disks.iter().find(|item| {
                item.disk_path == path || item.mount_id == path
            }).cloned();

            match record {
                Some(rec) => {
                    // Pre-check: disk already mounted
                    if rec.is_mounted {
                        show_toast(ah.clone(), i18n::t("mount_disk.error_already_mounted"));
                        return;
                    }

                    let params = MountParams {
                        disk_path: rec.disk_path.clone(),
                        disk_type: rec.disk_type.clone(),
                        partition: rec.partition.clone(),
                        filesystem: rec.filesystem.clone(),
                        mount_options: rec.mount_options.clone(),
                        bare: rec.is_bare,
                        name: rec.name.clone(),
                    };

                    // Proceed directly (wsl --shutdown is always safe)
                    run_shutdown_and_mount(ah.clone(), as_ptr.clone(), params).await;
                }
                None => {
                    show_toast(ah.clone(), i18n::t("mount_disk.no_disks_found"));
                }
            }
        }).unwrap();
    });
}