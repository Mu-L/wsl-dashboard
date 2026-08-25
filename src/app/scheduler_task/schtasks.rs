// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use std::process::Command;
use std::os::windows::process::CommandExt;
use tracing::{info, warn};

use super::models::*;

// MD5 hash implementation using the standard `md5` crate
fn md5_hex(input: &[u8]) -> String {
    format!("{:x}", md5::compute(input))
}

const CREATE_NO_WINDOW: u32 = 0x08000000;
const TASK_PREFIX: &str = r"\WSLDashboard\UserTasks\";

// Convert month number to schtasks month name
fn month_name(m: i32) -> String {
    match m {
        1 => "JAN", 2 => "FEB", 3 => "MAR", 4 => "APR",
        5 => "MAY", 6 => "JUN", 7 => "JUL", 8 => "AUG",
        9 => "SEP", 10 => "OCT", 11 => "NOV", 12 => "DEC",
        _ => "JAN",
    }.to_string()
}

// Query all scheduled tasks under WSLDashboard
pub fn query_tasks() -> Result<Vec<SchedulerTask>, String> {
    let output = Command::new("schtasks")
        .args(["/Query", "/TN", r"\WSLDashboard\UserTasks", "/FO", "CSV", "/V"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let stdout = match output {
        Ok(ref out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).to_string()
        }
        // Folder may not exist yet (no tasks created); fall back to querying all tasks
        _ => return query_all_tasks(),
    };

    let raw_records = parse_csv_records(&stdout)?;
    build_tasks(raw_records)
}

// Fallback: query all tasks and filter by prefix
fn query_all_tasks() -> Result<Vec<SchedulerTask>, String> {
    let output = Command::new("schtasks")
        .args(["/Query", "/FO", "CSV", "/V"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to execute schtasks: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("schtasks query failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let raw_records = parse_csv_records(&stdout)?;
    build_tasks(raw_records)
}

// Parse CSV into a list of raw records
fn parse_csv_records(csv: &str) -> Result<Vec<RawSchtasksRecord>, String> {
    let mut records = Vec::new();
    let lines: Vec<&str> = csv.lines().collect();

    for line in lines.iter().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let fields = parse_csv_line(line);
        if fields.len() < 9 {
            continue;
        }

        let task_name = fields[1].trim_matches('"').to_string();
        if !task_name.starts_with(TASK_PREFIX) {
            continue;
        }

        let next_run = fields.get(2).map(|s| s.to_string()).unwrap_or_default();
        let status = fields.get(3).map(|s| s.to_string()).unwrap_or_default();
        let last_run = fields.get(5).map(|s| s.to_string()).unwrap_or_default();
        let command = fields.get(8).map(|s| s.to_string()).unwrap_or_default();

        let task_id = task_name.trim_start_matches(TASK_PREFIX).to_string();

        records.push(RawSchtasksRecord {
            task_name,
            task_id,
            status,
            last_run,
            next_run,
            command,
        });
    }

    Ok(records)
}

// Parse a CSV line, properly handling RFC 4180 quoting ("" → ")
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '"' if in_quotes && i + 1 < chars.len() && chars[i + 1] == '"' => {
                // Escaped quote "" within a quoted field → literal "
                current.push('"');
                i += 1; // skip the second "
            }
            '"' => {
                // Toggle quoting (this is the opening/closing quote, not added to value)
                in_quotes = !in_quotes;
            }
            ',' if !in_quotes => {
                fields.push(current.clone());
                current.clear();
            }
            c => {
                current.push(c);
            }
        }
        i += 1;
    }
    fields.push(current);
    fields
}

// Build task list from raw records (each record maps to an independent task, no longer grouped/merged)
fn build_tasks(records: Vec<RawSchtasksRecord>) -> Result<Vec<SchedulerTask>, String> {
    let mut tasks = Vec::new();

    for record in records {
        let (admin_mode, cron_expr, author, xml_enabled) = query_task_xml_info(&record.task_name);

        // <Enabled> from XML is the authoritative, locale-independent source for
        // enabled/disabled state. schtasks /XML does NOT emit a <State> element;
        // the enabled flag lives under <Settings><Enabled>true|false</Enabled>.
        // Default to true when the element is missing (e.g., XML query failed).
        let enabled = xml_enabled.unwrap_or(true);
        let is_disabled = !enabled;

        // "Running" is the only runtime state we surface on the card. The CSV
        // /V "Status" column is locale-dependent, but it is the only readily
        // available source for running state without an extra PowerShell call.
        let csv_status = record.status.trim();
        let is_running = csv_status.eq_ignore_ascii_case("Running");

        let status = if is_running {
            "Running".to_string()
        } else if is_disabled {
            "Disabled".to_string()
        } else {
            "Ready".to_string()
        };

        tasks.push(SchedulerTask {
            task_id: record.task_id.clone(),
            display_name: record.task_id,
            cron_expr,
            command: record.command,
            arguments: String::new(),
            enabled,
            status,
            last_run: record.last_run,
            next_run: record.next_run,
            trigger_summary: String::new(),
            admin_mode,
            author,

        });
    }

    tasks.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    Ok(tasks)
}

// Write cron expression to task's Description field (via PowerShell Set-ScheduledTask)
fn set_task_description(task_name: &str, cron_expr: &str) {
    let (task_path, task_short_name) = match task_name.rfind('\\') {
        Some(pos) => (&task_name[..=pos], &task_name[pos + 1..]),
        None => ("", task_name),
    };
    let description = format!("cron:{}", cron_expr);
    let ps_cmd = format!(
        "$t = Get-ScheduledTask -TaskPath '{}' -TaskName '{}'; if($t){{ $t.Description = '{}'; $t | Set-ScheduledTask | Out-Null }}",
        task_path.replace('\'', "''"),
        task_short_name.replace('\'', "''"),
        description.replace('\'', "''")
    );
    match Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(out) if out.status.success() => {
            info!("Set description for task '{}' -> {}", task_name, cron_expr);
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            warn!("Failed to set task description: {}", stderr);
        }
        Err(e) => {
            warn!("Failed to execute powershell for description: {}", e);
        }
    }
}

// Build a PowerShell command string to set task description (cron expression)
fn build_description_command(task_name: &str, cron_expr: &str) -> String {
    let (task_path, task_short_name) = match task_name.rfind('\\') {
        Some(pos) => (&task_name[..=pos], &task_name[pos + 1..]),
        None => ("", task_name),
    };
    let description = format!("cron:{}", cron_expr);
    let ps_cmd = format!(
        "$t = Get-ScheduledTask -TaskPath '{}' -TaskName '{}'; if($t){{ $t.Description = '{}'; $t | Set-ScheduledTask | Out-Null }}",
        task_path.replace('\'', "''"),
        task_short_name.replace('\'', "''"),
        description.replace('\'', "''")
    );
    format!("powershell -NoProfile -Command \"{}\"", ps_cmd.replace('"', "\\\""))
}

// Query task XML to extract RunLevel, cron expression, Author, and enabled state.
// Returns (admin_mode, cron_expr, author, enabled) where `enabled` is None if the
// <Enabled> element could not be located in the XML.
fn query_task_xml_info(task_name: &str) -> (bool, String, String, Option<bool>) {
    let output = Command::new("schtasks")
        .args(["/Query", "/TN", task_name, "/XML"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let xml = String::from_utf8_lossy(&out.stdout);
            let admin_mode = xml.contains("<RunLevel>HighestAvailable</RunLevel>");
            let cron = extract_cron_from_xml(&xml);
            let author = extract_author_from_xml(&xml);
            let enabled = extract_enabled_from_xml(&xml);
            (admin_mode, cron, author, enabled)
        }
        _ => (false, String::new(), String::new(), None),
    }
}

// Extract cron expression from task XML Description field (format: cron:M H dom mon dow)
fn extract_cron_from_xml(xml: &str) -> String {
    if let Some(start) = xml.find("<Description>") {
        let content_start = start + "<Description>".len();
        if let Some(end) = xml[content_start..].find("</Description>") {
            let desc = xml[content_start..content_start + end].trim();
            if let Some(cron) = desc.strip_prefix("cron:") {
                return cron.trim().to_string();
            }
        }
    }
    String::new()
}

// Extract Author (creator) from task XML
fn extract_author_from_xml(xml: &str) -> String {
    if let Some(start) = xml.find("<Author>") {
        let content_start = start + "<Author>".len();
        if let Some(end) = xml[content_start..].find("</Author>") {
            return xml[content_start..content_start + end].trim().to_string();
        }
    }
    String::new()
}

// Extract <Enabled> value from task XML Settings.
// schtasks /XML does NOT emit a <State> element; the runtime enabled/disabled
// flag is stored under <Settings><Enabled>true|false</Enabled>. This value is
// locale-independent and authoritative for the enabled/disabled distinction.
// Returns None if the element is not present.
fn extract_enabled_from_xml(xml: &str) -> Option<bool> {
    let start = xml.find("<Enabled>")?;
    let content_start = start + "<Enabled>".len();
    let end = xml[content_start..].find("</Enabled>")?;
    let val = xml[content_start..content_start + end].trim().to_ascii_lowercase();
    match val.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

// Strip known command wrappers to recover the original command path.
// e.g. `powershell.exe -ExecutionPolicy Bypass -File "D:\script.ps1"` → `D:\script.ps1`
// e.g. `python.exe "D:\script.py"` → `D:\script.py`
pub fn unwrap_command(cmd: &str) -> String {
    let trimmed = cmd.trim();

    // Try each known wrapper prefix in order
    let prefixes = [
        "wscript.exe ",
        "powershell.exe -WindowStyle Hidden -ExecutionPolicy Bypass -File ",
        "powershell.exe -ExecutionPolicy Bypass -File ",
        "python.exe ",
    ];

    for prefix in prefixes {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim();
            // For wscript.exe VBS launcher, read the VBS to extract the PS1 path
            if prefix == "wscript.exe " {
                // Extract the VBS file path from quotes
                let vbs_path = if let Some(quoted) = rest.strip_prefix('"') {
                    if let Some(end) = quoted.find('"') {
                        quoted[..end].to_string()
                    } else {
                        return quoted.to_string();
                    }
                } else {
                    rest.split_whitespace().next().unwrap_or(rest).to_string()
                };
                if let Some(ps1_path) = extract_script_path_from_vbs(&vbs_path) {
                    return ps1_path;
                }
                return vbs_path;
            }
            // Extract path from quotes if present
            if let Some(quoted) = rest.strip_prefix('"') {
                if let Some(end) = quoted.find('"') {
                    return quoted[..end].to_string();
                }
                // No closing quote found: use the unquoted value
                return quoted.to_string();
            }
            // No quotes: take the first token as the path
            return rest.split_whitespace().next().unwrap_or(rest).to_string();
        }
    }

    trimmed.to_string()
}

// Returns the launcher directory for VBS scripts
fn vbs_launcher_dir() -> Result<std::path::PathBuf, String> {
    let userprofile =
        std::env::var("USERPROFILE").map_err(|_| "USERPROFILE not found".to_string())?;
    let dir = std::path::Path::new(&userprofile)
        .join(".wsldashboard")
        .join("scripts")
        .join("launcher");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create launcher directory: {}", e))?;
    Ok(dir)
}

// Create a VBS launcher that runs the given command_line silently (no window at all).
// `wscript.exe` is a GUI-subsystem process so it doesn't allocate a console,
// and `WScript.Shell.Run` with window style 0 runs the command completely hidden.
// The VBS filename is hashed from `task_id` so it stays stable across edits.
fn ensure_vbs_launcher(task_id: &str, command_line: &str) -> Result<String, String> {
    let dir = vbs_launcher_dir()?;
    let hash = md5_hex(task_id.as_bytes());
    let vbs_path = dir.join(format!("launcher-{}.vbs", hash));
    let vbs_path_str = vbs_path.to_string_lossy().to_string();

    let cmd_escaped = command_line.replace('"', "\"\"");
    let vbs_content = format!(
        "Set shell = CreateObject(\"WScript.Shell\")\nshell.Run \"{}\", 0, True\n",
        cmd_escaped
    );
    std::fs::write(&vbs_path, vbs_content)
        .map_err(|e| format!("Failed to write VBS launcher: {}", e))?;

    Ok(vbs_path_str)
}

// Extract the original script path from a VBS launcher file.
// Works for any format (ps1/py/bat/cmd) since it just looks for the
// first `""...""` pair (VBS-escaped quotes around the script path).
fn extract_script_path_from_vbs(vbs_path: &str) -> Option<String> {
    let content = std::fs::read_to_string(vbs_path).ok()?;
    // Pattern: shell.Run "exe ... ""path""", 0, True
    let prefix = "shell.Run \"";
    let start = content.find(prefix)?;
    let after_prefix = &content[start + prefix.len()..];
    // Find the first "" (opens the VBS-escaped path)
    let first = after_prefix.find("\"\"")?;
    let after_first = &after_prefix[first + 2..];
    // Find the next "" (closes the VBS-escaped path)
    let second = after_first.find("\"\"")?;
    Some(after_first[..second].to_string())
}

// Remove the VBS launcher file associated with a task_id (if it exists).
pub fn cleanup_vbs_launcher(task_id: &str) {
    if let Ok(dir) = vbs_launcher_dir() {
        let hash = md5_hex(task_id.as_bytes());
        let vbs_path = dir.join(format!("launcher-{}.vbs", hash));
        if vbs_path.exists() {
            let _ = std::fs::remove_file(&vbs_path);
        }
    }
}

// Create (or update) a scheduled task
// - requires_elevation: whether elevated execution is needed
// - old_task_id: if editing, the old task ID to delete before creating (batched into same UAC prompt)
pub fn create_task(params: &CreateTaskParams, requires_elevation: bool, old_task_id: Option<&str>) -> Result<(), String> {
    let parsed = parse_cron(&params.cron_expr)?;
    let schtasks = &parsed.schtasks;

    let task_name = format!("{}{}", TASK_PREFIX, params.display_name);
    let old_full_name = old_task_id.map(|id| format!("{}{}", TASK_PREFIX, id));

    let lower_cmd = params.command.to_lowercase();
    let (_is_vbs_wrapped, tr) = if lower_cmd.ends_with(".ps1") {
        let cmd_line = if params.arguments.is_empty() {
            format!("powershell.exe -ExecutionPolicy Bypass -File \"{}\"", params.command)
        } else {
            format!("powershell.exe -ExecutionPolicy Bypass -File \"{}\" {}", params.command, params.arguments)
        };
        let vbs = ensure_vbs_launcher(&params.display_name, &cmd_line)?;
        (true, format!("wscript.exe \"{}\"", vbs))
    } else if lower_cmd.ends_with(".py") {
        let cmd_line = if params.arguments.is_empty() {
            format!("python.exe \"{}\"", params.command)
        } else {
            format!("python.exe \"{}\" {}", params.command, params.arguments)
        };
        let vbs = ensure_vbs_launcher(&params.display_name, &cmd_line)?;
        (true, format!("wscript.exe \"{}\"", vbs))
    } else if lower_cmd.ends_with(".bat") || lower_cmd.ends_with(".cmd") {
        let cmd_line = if params.arguments.is_empty() {
            format!("cmd.exe /c \"{}\"", params.command)
        } else {
            format!("cmd.exe /c \"{}\" {}", params.command, params.arguments)
        };
        let vbs = ensure_vbs_launcher(&params.display_name, &cmd_line)?;
        (true, format!("wscript.exe \"{}\"", vbs))
    } else if params.arguments.is_empty() {
        (false, params.command.clone())
    } else {
        (false, format!("{} {}", params.command, params.arguments))
    };

    let mut args = vec![
        "/Create".to_string(),
        "/TN".to_string(), task_name.clone(),
        "/TR".to_string(), tr,
        "/F".to_string(),
    ];

    args.push("/SC".to_string());
    args.push(schtasks.sc.clone());

    if let Some(ref mo) = schtasks.mo {
        args.push("/MO".to_string());
        args.push(mo.clone());
    }
    if let Some(ref st) = schtasks.st {
        args.push("/ST".to_string());
        args.push(st.clone());
    }
    if let Some(ref d) = schtasks.d {
        args.push("/D".to_string());
        args.push(d.clone());
    }
    if let Some(ref m) = schtasks.m {
        args.push("/M".to_string());
        args.push(m.clone());
    }

    if params.admin_mode {
        args.push("/RL".to_string());
        args.push("HIGHEST".to_string());
    }

    if requires_elevation {
        let mut commands: Vec<String> = Vec::new();

        // Edit mode: delete old task in the same elevated batch
        if let Some(ref old_name) = old_full_name {
            commands.push(format!("schtasks /Delete /TN \"{}\" /F", old_name));
        }

        // Build command - need to properly quote /TR value for cmd.exe
        let mut shell_args = args.clone();
        // Find /TR in args and escape its value for cmd.exe
        for i in 0..shell_args.len() {
            if shell_args[i] == "/TR" {
                if let Some(val) = shell_args.get(i + 1) {
                    if val.contains(' ') || val.contains('"') {
                        // cmd.exe escaping: within "..." use "" for literal "
                        shell_args[i + 1] = format!("\"{}\"", val.replace('"', "\"\""));
                    }
                }
                break;
            }
        }
        commands.push(format!("schtasks {}", shell_args.join(" ")));

        // Set description (cron expression stored in Description field)
        commands.push(build_description_command(&task_name, &params.cron_expr));

        crate::utils::system::run_invisible_elevated_commands(commands)
            .map_err(|e| format!("schtasks elevated create failed: {}", e))?;
    } else {
        let output = Command::new("schtasks")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to execute schtasks: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("schtasks create failed: {}", stderr));
        }

        // Non-elevated: write cron expression to task description separately
        set_task_description(&task_name, &params.cron_expr);
    }

    Ok(())
}

// Delete a scheduled task
// - needs_elevation: whether elevated execution is needed (determined by handler layer based on shield logic)
pub fn delete_task(task_id: &str, needs_elevation: bool) -> Result<(), String> {
    let task_name = format!("{}{}", TASK_PREFIX, task_id);

    if !needs_elevation {
        let output = Command::new("schtasks")
            .args(["/Delete", "/TN", &task_name, "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to execute schtasks: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("schtasks delete failed: {}", stderr));
        }
    } else {
        let cmd_str = format!("schtasks /Delete /TN \"{}\" /F", task_name);
        crate::utils::system::run_invisible_elevated_command(&cmd_str)
            .map_err(|e| format!("schtasks elevated delete failed: {}", e))?;
    }

    // Clean up the VBS launcher file if it exists
    cleanup_vbs_launcher(task_id);

    Ok(())
}

// Enable/disable a task
// - needs_elevation: whether elevated execution is needed (determined by handler layer based on shield logic)
pub fn toggle_task(task_id: &str, enable: bool, needs_elevation: bool) -> Result<(), String> {
    let task_name = format!("{}{}", TASK_PREFIX, task_id);
    let action = if enable { "/ENABLE" } else { "/DISABLE" };

    if !needs_elevation {
        let output = Command::new("schtasks")
            .args(["/Change", "/TN", &task_name, action])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to execute schtasks: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("schtasks change failed: {}", stderr));
        }
    } else {
        let cmd_str = format!("schtasks /Change /TN \"{}\" {}", task_name, action);
        crate::utils::system::run_invisible_elevated_command(&cmd_str)
            .map_err(|e| format!("schtasks elevated change failed: {}", e))?;
    }

    Ok(())
}

// Run a task immediately
// - needs_elevation: whether elevated execution is needed (determined by handler layer based on shield logic)
pub fn run_task(task_id: &str, needs_elevation: bool) -> Result<(), String> {
    let task_name = format!("{}{}", TASK_PREFIX, task_id);

    if !needs_elevation {
        let output = Command::new("schtasks")
            .args(["/Run", "/TN", &task_name])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to execute schtasks: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("schtasks run failed: {}", stderr));
        }
    } else {
        let cmd_str = format!("schtasks /Run /TN \"{}\"", task_name);
        crate::utils::system::run_invisible_elevated_command(&cmd_str)
            .map_err(|e| format!("schtasks elevated run failed: {}", e))?;
    }

    Ok(())
}

// Parse a single cron field (only supports *, single number, */N)
fn parse_field(field: &str, min: i32, max: i32) -> Result<Vec<i32>, String> {
    if field.contains(',') {
        return Err(format!("Comma-separated values are not supported: '{}'. Use a single value, *, or */N", field));
    }

    if field == "*" {
        return Ok((min..=max).collect());
    }

    if let Some(step_str) = field.strip_prefix("*/") {
        let step: i32 = step_str.parse()
            .map_err(|_| format!("Invalid step value: '{}'", field))?;
        if step <= 0 {
            return Err(format!("Step must be positive: '{}'", field));
        }
        return Ok((min..=max).step_by(step as usize).collect());
    }

    let val: i32 = field.parse()
        .map_err(|_| format!("Invalid value: '{}'. Use a single number, *, or */N", field))?;
    if val < min || val > max {
        return Err(format!("Value {} out of range [{}, {}]", val, min, max));
    }
    Ok(vec![val])
}

// Parse a cron expression
pub fn parse_cron(expr: &str) -> Result<CronParsed, String> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err("Cron expression must have 5 fields: minute hour day month weekday".into());
    }

    let minutes = parse_field(parts[0], 0, 59)?;
    let hours = parse_field(parts[1], 0, 23)?;
    let days_of_month = parse_field(parts[2], 1, 31)?;
    let months = parse_field(parts[3], 1, 12)?;
    let days_of_week = parse_field(parts[4], 0, 7)?;

    // Map 7 to 0 (Sunday) and deduplicate
    let mut days_of_week: Vec<i32> = days_of_week.iter().map(|&d| if d == 7 { 0 } else { d }).collect();
    days_of_week.sort();
    days_of_week.dedup();

    let schtasks = convert_to_schtasks(&minutes, &hours, &days_of_month, &months, &days_of_week)?;

    Ok(CronParsed {
        fields: CronFields { minutes, hours, days_of_month, months, days_of_week },
        is_simple: true,
        schtasks,
    })
}

// Convert cron fields to schtasks parameters
fn convert_to_schtasks(
    minutes: &[i32],
    hours: &[i32],
    days_of_month: &[i32],
    months: &[i32],
    days_of_week: &[i32],
) -> Result<SchtasksParams, String> {
    let all_minutes = minutes.len() == 60;
    let all_hours = hours.len() == 24;
    let all_days = days_of_month.len() >= 28;
    let all_months = months.len() == 12;
    let all_weekdays = days_of_week.len() == 7 || days_of_week.is_empty();

    // Pattern 0: every minute (* * * * *)
    if all_minutes && all_hours && all_days && all_months && all_weekdays {
        return Ok(SchtasksParams {
            sc: "MINUTE".into(),
            mo: Some("1".into()),
            st: None,
            d: None,
            m: None,
        });
    }

    // Pattern 1: every N minutes (*/N * * * *) — minutes must have multiple values
    if !all_minutes && all_hours && all_days && all_months && all_weekdays && minutes.len() > 1 {
        let step = minutes[1] - minutes[0];
        return Ok(SchtasksParams {
            sc: "MINUTE".into(),
            mo: Some(step.to_string()),
            st: None,
            d: None,
            m: None,
        });
    }

    // Pattern 2: every hour (M * * * *)
    if !all_minutes && all_hours && all_days && all_months && all_weekdays {
        return Ok(SchtasksParams {
            sc: "HOURLY".into(),
            mo: Some("1".into()),
            st: Some(format!("{:02}:{:02}", 0, minutes[0])),
            d: None,
            m: None,
        });
    }

    // Pattern 3: daily (M H * * *)
    if !all_minutes && !all_hours && all_days && all_months && all_weekdays {
        return Ok(SchtasksParams {
            sc: "DAILY".into(),
            mo: Some("1".into()),
            st: Some(format!("{:02}:{:02}", hours[0], minutes[0])),
            d: None,
            m: None,
        });
    }

    // Pattern 4: weekly (M H * * dow)
    if !all_minutes && !all_hours && all_days && all_months && !all_weekdays {
        let dow_str = days_of_week.iter()
            .map(|d| match d {
                0 => "SUN", 1 => "MON", 2 => "TUE", 3 => "WED",
                4 => "THU", 5 => "FRI", 6 => "SAT", _ => "SUN",
            })
            .collect::<Vec<_>>()
            .join(",");
        return Ok(SchtasksParams {
            sc: "WEEKLY".into(),
            mo: None,
            st: Some(format!("{:02}:{:02}", hours[0], minutes[0])),
            d: Some(dow_str),
            m: None,
        });
    }

    // Pattern 5: monthly (M H d * *)
    if !all_minutes && !all_hours && !all_days && all_months && all_weekdays {
        return Ok(SchtasksParams {
            sc: "MONTHLY".into(),
            mo: None,
            st: Some(format!("{:02}:{:02}", hours[0], minutes[0])),
            d: Some(days_of_month[0].to_string()),
            m: None,
        });
    }

    // Pattern 6: specific months (M H d mon *)
    if !all_minutes && !all_hours && !all_days && !all_months {
        return Ok(SchtasksParams {
            sc: "MONTHLY".into(),
            mo: None,
            st: Some(format!("{:02}:{:02}", hours[0], minutes[0])),
            d: Some(days_of_month[0].to_string()),
            m: Some(month_name(months[0])),
        });
    }

    Err(format!("Unsupported cron pattern: {} {} {} {} {}",
        minutes.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(","),
        hours.iter().map(|h| h.to_string()).collect::<Vec<_>>().join(","),
        days_of_month.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(","),
        months.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(","),
        days_of_week.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(","),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_convert(cron: &str) -> SchtasksParams {
        parse_cron(cron).unwrap().schtasks
    }

    #[test]
    fn test_every_minute() {
        let result = parse_and_convert("* * * * *");
        assert_eq!(result.sc, "MINUTE");
        assert_eq!(result.mo.as_deref(), Some("1"));
    }

    #[test]
    fn test_every_5_minutes() {
        let result = parse_and_convert("*/5 * * * *");
        assert_eq!(result.sc, "MINUTE");
        assert_eq!(result.mo.as_deref(), Some("5"));
    }

    #[test]
    fn test_every_hour_at_minute() {
        let result = parse_and_convert("30 * * * *");
        assert_eq!(result.sc, "HOURLY");
        assert_eq!(result.st.as_deref(), Some("00:30"));
    }

    #[test]
    fn test_daily() {
        let result = parse_and_convert("5 12 * * *");
        assert_eq!(result.sc, "DAILY");
        assert_eq!(result.st.as_deref(), Some("12:05"));
    }

    #[test]
    fn test_weekly_monday() {
        let result = parse_and_convert("0 9 * * 1");
        assert_eq!(result.sc, "WEEKLY");
        assert_eq!(result.d.as_deref(), Some("MON"));
    }

    #[test]
    fn test_monthly() {
        let result = parse_and_convert("0 0 1 * *");
        assert_eq!(result.sc, "MONTHLY");
        assert_eq!(result.d.as_deref(), Some("1"));
    }

    #[test]
    fn test_specific_month() {
        let result = parse_and_convert("5 12 1 6 *");
        assert_eq!(result.sc, "MONTHLY");
        assert_eq!(result.d.as_deref(), Some("1"));
        assert_eq!(result.m.as_deref(), Some("JUN"));
    }

    #[test]
    fn test_all_month_names() {
        let names: Vec<String> = (1..=12).map(month_name).collect();
        assert_eq!(names, vec![
            "JAN", "FEB", "MAR", "APR", "MAY", "JUN",
            "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"
        ]);
    }

    #[test]
    fn test_parse_field_rejects_comma() {
        assert!(parse_field("1,5", 0, 59).is_err());
    }

    #[test]
    fn test_parse_field_rejects_range() {
        assert!(parse_field("1-5", 0, 59).is_err());
    }

    #[test]
    fn test_parse_field_out_of_range() {
        assert!(parse_field("60", 0, 59).is_err());
    }

    #[test]
    fn test_parse_cron_too_few_fields() {
        assert!(parse_cron("5 12 *").is_err());
    }

    #[test]
    fn test_weekday_7_maps_to_0() {
        let result = parse_and_convert("0 9 * * 7");
        assert_eq!(result.d.as_deref(), Some("SUN"));
    }
}
