use sysinfo::System;

#[derive(Debug, Clone)]
pub struct ClaudeProcess {
    pub pid: u32,
    pub parent_app_pid: u32,
    pub parent_app_name: String,
}

/// Known GUI application names that can host Claude Code
const KNOWN_GUI_APPS: &[&str] = &[
    "Terminal",
    "iTerm2",
    "Claude",
    "Electron",
    "Code Helper",
    "Code",
    "Warp",
    "Alacritty",
    "kitty",
    "WezTerm",
    "Hyper",
    "Ghostty",
];

/// Find all running Claude Code processes and map them to their parent GUI app.
pub fn find_claude_processes(system: &mut System) -> Vec<ClaudeProcess> {
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let mut results = Vec::new();

    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_string();

        // Look for the `claude` binary
        if name != "claude" {
            continue;
        }

        // Check if this looks like a Claude Code process by inspecting command args
        let cmd: Vec<String> = process
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let cmd_str = cmd.join(" ");

        // Claude Code processes typically have recognizable args
        let looks_like_claude_code = cmd_str.contains("permission-prompt-tool")
            || cmd_str.contains("claude")
            || cmd.len() <= 3; // bare `claude` invocation

        if !looks_like_claude_code {
            continue;
        }

        // Walk the parent chain to find the GUI application
        if let Some(parent) = find_parent_gui_app(system, pid.as_u32()) {
            results.push(ClaudeProcess {
                pid: pid.as_u32(),
                parent_app_pid: parent.0,
                parent_app_name: parent.1,
            });
        }
    }

    results
}

/// Walk up the process tree to find the nearest known GUI application.
fn find_parent_gui_app(system: &System, start_pid: u32) -> Option<(u32, String)> {
    let mut current_pid = start_pid;
    let mut last_known = None;

    for _ in 0..15 {
        let pid = sysinfo::Pid::from_u32(current_pid);
        let process = system.process(pid)?;

        let parent_pid = process.parent()?;
        let parent = system.process(parent_pid)?;
        let parent_name = parent.name().to_string_lossy().to_string();

        // Check if this is a known GUI app
        for &app in KNOWN_GUI_APPS {
            if parent_name.contains(app) {
                return Some((parent_pid.as_u32(), parent_name));
            }
        }

        last_known = Some((parent_pid.as_u32(), parent_name));
        current_pid = parent_pid.as_u32();

        if parent_pid.as_u32() <= 1 {
            break;
        }
    }

    last_known
}

/// A detected GUI application window (for direct app scanning, e.g. VSCode).
#[derive(Debug, Clone)]
pub struct AppWindow {
    pub pid: u32,
    pub app_name: String,
}

/// System processes that show notification/permission banners
const SYSTEM_NOTIFICATION_PROCESSES: &[&str] = &[
    "NotificationCenter",
    "UserNotificationCenter",
    "SystemUIServer",
    "CoreServicesUIAgent",
];

/// Known VSCode-like process names
const VSCODE_PROCESS_NAMES: &[&str] = &["Code", "Electron"];

/// Find all running VSCode processes.
/// Returns the main app process (not helper/renderer processes).
pub fn find_vscode_processes(system: &mut System) -> Vec<AppWindow> {
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut results = Vec::new();

    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_string();

        // Skip helper processes - we want the main app process
        if name.contains("Helper") || name.contains("GPU") || name.contains("Utility") {
            continue;
        }

        let is_vscode = VSCODE_PROCESS_NAMES.iter().any(|&n| name == n);
        if !is_vscode {
            continue;
        }

        // For "Electron" processes, verify it's actually VSCode by checking args
        if name == "Electron" {
            let cmd: Vec<String> = process
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect();
            let cmd_str = cmd.join(" ");
            if !cmd_str.contains("Visual Studio Code") && !cmd_str.contains("/code") {
                continue;
            }
        }

        results.push(AppWindow {
            pid: pid.as_u32(),
            app_name: name,
        });
    }

    results
}

/// Find system processes that render notification/permission dialogs.
pub fn find_system_notification_processes(system: &mut System) -> Vec<AppWindow> {
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut results = Vec::new();

    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_string();

        let is_system_notif = SYSTEM_NOTIFICATION_PROCESSES.iter().any(|&n| name == n);
        if !is_system_notif {
            continue;
        }

        results.push(AppWindow {
            pid: pid.as_u32(),
            app_name: name,
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_processes_does_not_crash() {
        let mut system = System::new();
        let processes = find_claude_processes(&mut system);
        println!("Found {} Claude processes", processes.len());
    }

    #[test]
    fn test_find_vscode_does_not_crash() {
        let mut system = System::new();
        let processes = find_vscode_processes(&mut system);
        println!("Found {} VSCode processes", processes.len());
    }
}
