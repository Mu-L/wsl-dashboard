// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use crate::wsl::models::{WslCommandResult, WslDistro, WslInformation};
pub use crate::wsl::executor::WslCommandExecutor;
use crate::config::ConfigManager;

impl WslCommandExecutor {
    // Get WSL subsystem list
    pub async fn list_distros(&self) -> WslCommandResult<Vec<WslDistro>> {
        crate::wsl::ops::info::list_distros(self).await
    }

    // Start specified WSL subsystem
    pub async fn start_distro(&self, distro_name: &str) -> WslCommandResult<String> {
        crate::wsl::ops::lifecycle::start_distro(self, distro_name).await
    }

    // Stop specified WSL subsystem
    pub async fn stop_distro(&self, distro_name: &str) -> WslCommandResult<String> {
        crate::wsl::ops::lifecycle::stop_distro(self, distro_name).await
    }

    // Shutdown WSL
    pub async fn shutdown_wsl(&self) -> WslCommandResult<String> {
        crate::wsl::ops::lifecycle::shutdown_wsl(self).await
    }

    // Delete specified WSL subsystem
    pub async fn delete_distro(&self, config_manager: &ConfigManager, distro_name: &str) -> WslCommandResult<String> {
        crate::wsl::ops::lifecycle::delete_distro(self, config_manager, distro_name).await
    }

    // Move specified WSL subsystem
    pub async fn move_distro(&self, distro_name: &str, new_path: &str) -> WslCommandResult<String> {
        crate::wsl::ops::lifecycle::move_distro(self, distro_name, new_path).await
    }
    
    // Export specified WSL subsystem
    pub async fn export_distro(&self, distro_name: &str, file_path: &str) -> WslCommandResult<String> {
        crate::wsl::ops::transfer::export_distro(self, distro_name, file_path).await
    }
    
    // Import WSL subsystem
    pub async fn import_distro(&self, distro_name: &str, install_location: &str, file_path: &str) -> WslCommandResult<String> {
        crate::wsl::ops::transfer::import_distro(self, distro_name, install_location, file_path).await
    }
    
    // Probe for optimal download source
    pub async fn detect_fastest_source(&self) -> bool {
        crate::wsl::ops::info::detect_fastest_source(self).await
    }

    // Get list of installable WSL subsystems
    pub async fn list_available_distros(&self) -> WslCommandResult<String> {
        crate::wsl::ops::info::list_available_distros(self).await
    }

    // Get list of running WSL distros (cross-locale safe)
    pub async fn list_running_distros(&self) -> Vec<String> {
        crate::wsl::ops::info::list_running_distros(self).await
    }

    // Get default WSL distro name (cross-locale safe)
    pub async fn get_default_distro_name(&self) -> Option<String> {
        crate::wsl::ops::info::get_default_distro_name(self).await
    }

    // Open distribution's folder
    pub async fn open_distro_folder(&self, distro_name: &str) -> WslCommandResult<String> {
        crate::wsl::ops::ui::open_distro_folder(self, distro_name).await
    }

    // Open VS Code in distribution
    pub async fn open_distro_vscode(&self, distro_name: &str, working_dir: &str) -> WslCommandResult<String> {
        crate::wsl::ops::ui::open_distro_vscode(self, distro_name, working_dir).await
    }

    // Open terminal in distribution
    pub async fn open_distro_terminal(&self, distro_name: &str, working_dir: &str, proxy_exports: Option<Vec<(String, String)>>) -> WslCommandResult<String> {
        crate::wsl::ops::ui::open_distro_terminal(self, distro_name, working_dir, proxy_exports).await
    }

    // Open specified path in distribution
    pub async fn open_distro_folder_path(&self, distro_name: &str, sub_path: &str) -> WslCommandResult<String> {
        crate::wsl::ops::ui::open_distro_folder_path(self, distro_name, sub_path).await
    }

    // Get information of distribution
    pub async fn get_distro_information(&self, distro_name: &str) -> WslCommandResult<WslInformation> {
        crate::wsl::ops::info::get_distro_information(self, distro_name).await
    }

    #[allow(dead_code)]
    pub async fn get_distro_install_location(&self, distro_name: &str) -> WslCommandResult<String> {
        crate::wsl::ops::info::get_distro_install_location(self, distro_name).await
    }
}
