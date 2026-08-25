// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

pub mod constants;
pub mod state;
pub mod window;
pub mod updater;
pub mod autostart;
pub mod tray;
pub mod single_instance;
pub mod tasks;
pub mod uninstall;
pub mod initialize;
pub mod scheduler;
pub mod task_scheduler;
pub mod scheduler_task;
pub mod popup;
pub mod runner;
pub mod cli;
pub mod launcher;
pub mod lifecycle;
pub mod mount_disk;
pub mod startup_info;

pub use constants::*;
pub use state::{AppState, VSCodeExtensionData};
