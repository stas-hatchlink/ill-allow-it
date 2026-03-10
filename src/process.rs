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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_processes_does_not_crash() {
        let mut system = System::new();
        let processes = find_claude_processes(&mut system);
        println!("Found {} Claude processes", processes.len());
    }
}
