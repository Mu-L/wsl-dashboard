// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::core::HSTRING;

pub fn get_disk_free_space(path: &str) -> u64 {
    let mut free_bytes_available: u64 = 0;
    let mut total_number_of_bytes: u64 = 0;
    let mut total_number_of_free_bytes: u64 = 0;

    let path_hstring = HSTRING::from(path);
    unsafe {
        if GetDiskFreeSpaceExW(
            &path_hstring,
            Some(&mut free_bytes_available),
            Some(&mut total_number_of_bytes),
            Some(&mut total_number_of_free_bytes),
        ).is_ok() {
            free_bytes_available
        } else {
            0
        }
    }
}

pub fn get_c_drive_free_space() -> u64 {
    get_disk_free_space("C:\\")
}

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    use std::io::Write;

    let mut cmd = Command::new("clip");
    cmd.stdin(Stdio::piped());
    
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn()
        .map_err(|e| format!("Failed to spawn clip.exe: {}", e))?;

    let mut stdin = child.stdin.take().ok_or("Failed to open stdin for clip.exe")?;
    stdin.write_all(text.as_bytes()).map_err(|e| format!("Failed to write to clip.exe: {}", e))?;
    drop(stdin);

    let status = child.wait().map_err(|e| format!("Failed to wait for clip.exe: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("clip.exe exited with status: {}", status))
    }
}

// Execute a command with UAC elevation using ShellExecuteExW
pub fn run_command_with_elevation(program_name: &str, args: Vec<String>) -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NOASYNC};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{WaitForSingleObject, GetExitCodeProcess, INFINITE};
    use tracing::{debug, error};

    let args_str = args.join(" ");
    let program = HSTRING::from(program_name);
    let parameters = HSTRING::from(&args_str);
    let verb = HSTRING::from("runas");
    
    debug!("Executing elevated command: {} {}", program_name, args_str);

    let sys_dir = HSTRING::from("C:\\Windows\\System32");
    
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(program.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        lpDirectory: PCWSTR(sys_dir.as_ptr()),
        nShow: SW_HIDE.0 as i32,
        ..Default::default()
    };

    unsafe {
        match ShellExecuteExW(&mut sei) {
            Ok(()) => {
                // Wait for the elevated process to finish
                if !sei.hProcess.is_invalid() {
                    WaitForSingleObject(sei.hProcess, INFINITE);

                    let mut exit_code: u32 = 0;
                    if GetExitCodeProcess(sei.hProcess, &mut exit_code).is_ok() {
                        if exit_code != 0 {
                            let _ = CloseHandle(sei.hProcess);
                            let msg = if exit_code == 4294967295 {
                                format!("Command exited with code {} (0x{:X}) - WSL not ready or disk in use", exit_code, exit_code)
                            } else {
                                format!("Command exited with code {} (0x{:X})", exit_code, exit_code)
                            };
                            error!("Elevated command failed: {}", msg);
                            return Err(msg);
                        }
                    }

                    let _ = CloseHandle(sei.hProcess);
                }
                Ok(())
            }
            Err(e) => {
                let msg = format!("UAC elevation failed or was denied: {}", e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }
}

// Execute a command completely invisibly with elevation.
pub fn run_invisible_elevated_commands(commands: Vec<String>) -> Result<(), String> {
    use tracing::info;
    use tracing::debug;
    use tracing::error;
    
    if commands.is_empty() { return Ok(()); }
    
    // Join commands with ' & '
    let combined = commands.join(" & ");
    
    // Create a unique temp file to capture stdout/stderr for diagnostic purposes
    let temp_dir = std::env::temp_dir();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_file = temp_dir.join(format!("wsldashboard_elevated_{}.log", timestamp));
    let temp_path = temp_file.to_string_lossy().to_string();
    
    // Redirect stdout and stderr to the temp file for diagnostic capture.
    // NOTE: do NOT wrap the whole command in extra quotes here. The redirect target
    // is already quoted, and an additional outer `" "` pair would nest with it and
    // confuse cmd.exe's parser (e.g. `\\.\PHYSICALDRIVE1` device paths).
    let combined_with_redirect = format!("({}) > \"{}\" 2>&1", combined, temp_path);
    
    info!("Requesting invisible elevated execution for {} commands via cmd.exe", commands.len());
    debug!("Full elevated command: cmd /c {}", combined_with_redirect);
    
    let result = run_command_with_elevation("cmd.exe", vec!["/c".to_string(), combined_with_redirect]);
    
    // Read the captured output file if it exists
    let output_content = std::fs::read_to_string(&temp_file).unwrap_or_default();
    let _ = std::fs::remove_file(&temp_file);
    
    if result.is_err() {
        if !output_content.trim().is_empty() {
            error!("Elevated command stderr/stdout output:\n{}", output_content.trim());
        }
    }
    
    result
}

pub fn run_invisible_elevated_command(command: &str) -> Result<(), String> {
    run_invisible_elevated_commands(vec![command.to_string()])
}

// Asynchronously clean up legacy VBS startup script (shell:startup)
pub fn cleanup_legacy_vbs_startup() {
    std::thread::spawn(|| {
        use tracing::{info, warn, error};

        // Get the current user's AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup
        let home_dir = match dirs::home_dir() {
            Some(path) => path,
            None => return,
        };
        
        let vbs_path = home_dir.join("AppData")
            .join("Roaming")
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join("wsl-dashboard.vbs");

        if vbs_path.exists() {
            info!("Legacy startup VBS found: {:?}. Attempting cleanup...", vbs_path);
            
            // Attempt to delete the file with a 3-second timeout constraint (prevents permanent blockage by antivirus)
            let path_to_del = vbs_path.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            
            std::thread::spawn(move || {
                let res = std::fs::remove_file(&path_to_del);
                let _ = tx.send(res);
            });

            match rx.recv_timeout(std::time::Duration::from_secs(3)) {
                Ok(Ok(_)) => info!("Successfully removed legacy VBS startup script."),
                Ok(Err(e)) => error!("Failed to remove legacy VBS script: {}", e),
                Err(_) => warn!("Cleanup of legacy VBS script timed out (possible antivirus interference)."),
            }
        }
    });
}

// Try to attach to parent process console (used to display log output when running in command line)
pub fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

pub fn is_sparse_file(path: &str) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let attributes = metadata.file_attributes();
            // FILE_ATTRIBUTE_SPARSE_FILE = 0x200
            return (attributes & 0x200) != 0;
        }
    }
    false
}
