// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use crate::config::ConfigManager;
use crate::utils::logging::LoggingSystem;

// Handle immediate commands (like /version, /help) that don't require complex environment initialization
pub fn handle_immediate_commands(args: &[String]) -> Option<i32> {
    // 0. Quick check for version or help (before any initialization for clean output)
    if args.iter().any(|a| a == "/version" || a == "-v" || a == "--version") {
        crate::utils::system::attach_console();
        println!("v{}", env!("CARGO_PKG_VERSION"));
        return Some(0);
    }

    if args.iter().any(|a| a == "/help" || a == "-h" || a == "--help" || a == "/?") {
        crate::utils::system::attach_console();
        println!("");
        println!("WSL Dashboard v{} ({})", env!("CARGO_PKG_VERSION"), crate::app::PROJECT_WEBSITE);
        println!("Usage: wsldashboard.exe [options]");
        println!("");
        println!("Options:");
        println!("  /initialize   Initialize Task Scheduler tasks and helper scripts (Requires UAC)");
        println!("  /clean [/all] Clean up system-level configurations (Uninstall mode)");
        println!("                Append /all to also delete the ~/.wsldashboard directory");
        println!("  /silent       Start the application minimized to the system tray");
        println!("  /scheduler    Auto-start distros,USB auto-connect,port forwarding (Internal use)");
        println!("  /version, -v  Show version information");
        println!("  /help, -h     Show this help message");
        println!("");
        return Some(0);
    }
    
    None
}

// Handle complex commands that require context (like /scheduler, /clean, /initialize)
pub async fn handle_context_commands(
    args: &[String], 
    config_manager: &ConfigManager,
    logging_system: LoggingSystem,
) -> Option<i32> {
    // 4. Initial check for network sync command
    if let Some(pos) = args.iter().position(|a| a == "/scheduler") {
        let exit_code = crate::app::scheduler::run_scheduler_task(args, pos, config_manager).await;
        
        // Ensure logs are flushed before exit
        drop(logging_system);
        return Some(exit_code);
    }

    // 4.7 Check for clean command
    if args.iter().any(|a| a == "/clean") {
        crate::utils::system::attach_console();

        let delete_all = args.iter().any(|a| a == "/all");
        let config_dir = crate::app::uninstall::run_uninstall().await;
        
        // Ensure logs are flushed before exit
        drop(logging_system);

        // Final step: handle directory deletion or persistence
        crate::app::uninstall::final_cleanup(config_dir, delete_all);
        
        return Some(0);
    }

    // 4.8 Check for initialize command
    if args.iter().any(|a| a == "/initialize") {
        crate::utils::system::attach_console();

        crate::app::initialize::run_initialize().await;

        // Ensure logs are flushed before exit
        drop(logging_system);

        return Some(0);
    }
    
    None
}
