use crate::accessibility;
use crate::config::Config;
use crate::process;
use crate::rules;
use crate::types::{ActionLogEntry, ApprovalAction, DetectedPrompt, PromptSource};
use regex::Regex;
use std::collections::HashMap;
use std::io::Write;
use std::time::{Instant, SystemTime};
use sysinfo::System;

/// Write a debug message to the log file
fn debug_log(msg: &str) {
    let log_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".ill-allow-it")
        .join("debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let now = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(f, "[{}] {}", now, msg);
    }
}

/// Button titles we look for to detect permission prompts.
const APPROVE_BUTTONS: &[&str] = &["Allow once", "Allow Once", "Always allow for session"];
const DENY_BUTTONS: &[&str] = &["Deny"];

/// Button titles for VSCode workspace trust.
const TRUST_BUTTONS: &[&str] = &["Yes, I trust the authors", "I trust the authors", "Trust"];

/// Button titles for macOS system notification permission banners.
const NOTIFICATION_ALLOW_BUTTONS: &[&str] = &["Allow once", "Allow Once", "Allow"];

pub struct Monitor {
    system: System,
    config: Config,
    /// Prompts we've already acted on, keyed by target PID.
    known_prompts: HashMap<u32, Instant>,
    /// Recent action log for display in the menu
    pub action_log: Vec<ActionLogEntry>,
    /// Regex for extracting tool name from prompt text
    tool_regex: Regex,
    /// Tick counter for periodic logging
    tick_count: u64,
}

impl Monitor {
    pub fn new(config: Config) -> Self {
        let tool_regex = Regex::new(
            r"(?i)Allow Claude to (Read|Write|Edit|Run|Bash|Glob|Grep|WebFetch|Web Search|WebSearch|Agent|TodoWrite|NotebookEdit|mcp\S+)"
        ).expect("invalid tool regex");

        Monitor {
            system: System::new(),
            config,
            known_prompts: HashMap::new(),
            action_log: Vec::new(),
            tool_regex,
            tick_count: 0,
        }
    }

    pub fn update_config(&mut self, config: Config) {
        self.config = config;
    }

    /// Run one tick of the monitoring loop. Returns the number of actions taken.
    pub fn tick(&mut self) -> usize {
        if !self.config.enabled {
            return 0;
        }

        self.tick_count += 1;
        // Log a heartbeat every 60 ticks (~30 seconds)
        if self.tick_count % 60 == 1 {
            let trusted = crate::accessibility::is_accessibility_trusted();
            debug_log(&format!(
                "Heartbeat: tick #{}, enabled={}, accessibility_trusted={}",
                self.tick_count, self.config.enabled, trusted
            ));
            if !trusted {
                debug_log("WARNING: Accessibility not trusted! Re-grant permission in System Settings.");
            }
        }

        let mut actions_taken = 0;
        let mut still_active: Vec<u32> = Vec::new();

        // Path 1: Claude Code processes - find them and scan their parent app windows
        let claude_processes = process::find_claude_processes(&mut self.system);
        if !claude_processes.is_empty() {
            debug_log(&format!("Found {} Claude process(es)", claude_processes.len()));
        }

        // Deduplicate by parent PID (multiple claude processes may share one parent)
        let mut seen_parents: Vec<u32> = Vec::new();
        for proc in &claude_processes {
            if seen_parents.contains(&proc.parent_app_pid) {
                still_active.push(proc.parent_app_pid);
                continue;
            }
            seen_parents.push(proc.parent_app_pid);

            log::debug!(
                "Scanning parent app {} (PID {}) for Claude PID {}",
                proc.parent_app_name, proc.parent_app_pid, proc.pid
            );

            match self.check_for_claude_prompt(proc.parent_app_pid, &proc.parent_app_name) {
                Ok(Some(entry)) => {
                    log::info!("Action: {}", entry);
                    self.action_log.push(entry);
                    actions_taken += 1;
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!(
                        "Error scanning {} (PID {}): {}",
                        proc.parent_app_name, proc.parent_app_pid, e
                    );
                }
            }
            still_active.push(proc.parent_app_pid);
        }

        // Path 2: VSCode windows - scan for workspace trust and extension prompts
        if self.config.vscode_enabled {
            let vscode_processes = process::find_vscode_processes(&mut self.system);
            for app in &vscode_processes {
                if seen_parents.contains(&app.pid) {
                    still_active.push(app.pid);
                    continue;
                }

                match self.check_for_vscode_prompt(app.pid, &app.app_name) {
                    Ok(Some(entry)) => {
                        log::info!("Action: {}", entry);
                        self.action_log.push(entry);
                        actions_taken += 1;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::debug!("Error scanning VSCode (PID {}): {}", app.pid, e);
                    }
                }
                still_active.push(app.pid);
            }
        }

        // Path 3: System notification permission banners (e.g. "Claude - Notification - Allow once")
        let system_notif_processes = process::find_system_notification_processes(&mut self.system);
        for app in &system_notif_processes {
            if seen_parents.contains(&app.pid) {
                continue;
            }

            match self.check_for_system_notification(app.pid, &app.app_name) {
                Ok(Some(entry)) => {
                    log::info!("Action: {}", entry);
                    self.action_log.push(entry);
                    actions_taken += 1;
                }
                Ok(None) => {}
                Err(e) => {
                    log::debug!("Error scanning {} (PID {}): {}", app.app_name, app.pid, e);
                }
            }
        }

        // Keep only last 50 entries
        if self.action_log.len() > 50 {
            self.action_log.drain(0..self.action_log.len() - 50);
        }

        // Clean up prompts for processes that no longer exist
        self.known_prompts
            .retain(|pid, _| still_active.contains(pid));

        actions_taken
    }

    /// Scan an app's windows for Claude permission prompt buttons and click them.
    fn check_for_claude_prompt(&mut self, pid: u32, app_name: &str) -> anyhow::Result<Option<ActionLogEntry>> {
        // Dedup check
        if let Some(last_seen) = self.known_prompts.get(&pid) {
            if last_seen.elapsed().as_secs() < 3 {
                return Ok(None);
            }
        }

        let scan = accessibility::scan_app_windows(pid as i32)?;

        // Log all buttons found (useful for debugging what labels the app uses)
        if !scan.buttons.is_empty() {
            let btn_titles: Vec<&str> = scan.buttons.iter().map(|b| b.title.as_str()).collect();
            debug_log(&format!(
                "Buttons in {} (PID {}): {:?}",
                app_name, pid, btn_titles
            ));
        } else {
            debug_log(&format!("No buttons in {} (PID {})", app_name, pid));
        }

        // Look for "Allow once" or "Always allow for session" buttons
        let approve_button = scan.buttons.iter().find(|b| {
            APPROVE_BUTTONS.iter().any(|&target| b.title.contains(target))
        });
        let deny_button = scan.buttons.iter().find(|b| {
            DENY_BUTTONS.iter().any(|&target| b.title == target)
        });

        debug_log(&format!(
            "approve_button={:?}, deny_button={:?}",
            approve_button.map(|b| &b.title),
            deny_button.map(|b| &b.title),
        ));

        // Must have both an approve and deny button to confirm it's a permission prompt
        if approve_button.is_none() || deny_button.is_none() {
            self.known_prompts.remove(&pid);
            return Ok(None);
        }

        // Extract tool name from the window text
        let all_text = scan.texts.join("\n");
        let tool_name = self.tool_regex
            .captures(&all_text)
            .map(|caps| caps.get(1).unwrap().as_str().to_string());
        let tool_detail = extract_detail(&all_text);

        log::info!(
            "Permission prompt detected in {} (PID {}): tool={:?} detail={:?}",
            app_name, pid, tool_name, tool_detail
        );

        // Build prompt for rule evaluation
        let prompt = DetectedPrompt {
            source: PromptSource::ClaudeCode,
            target_pid: pid,
            app_name: app_name.to_string(),
            prompt_text: all_text,
            tool_name: tool_name.clone(),
            tool_detail: tool_detail.clone(),
            detected_at: Instant::now(),
        };

        let (action, rule_name) = rules::evaluate_rules(&self.config, &prompt);

        if action == ApprovalAction::Ignore {
            return Ok(None);
        }

        // Click the appropriate button
        let button_to_click = match action {
            ApprovalAction::Approve | ApprovalAction::ApproveAlways => approve_button.unwrap(),
            ApprovalAction::Deny => deny_button.unwrap(),
            ApprovalAction::Ignore => return Ok(None),
        };

        log::info!("Clicking button: {:?} in {} (PID {})", button_to_click.title, app_name, pid);
        accessibility::click_button(button_to_click)?;

        self.known_prompts.insert(pid, Instant::now());

        Ok(Some(ActionLogEntry {
            timestamp: SystemTime::now(),
            source: PromptSource::ClaudeCode,
            tool_name: tool_name.unwrap_or_else(|| "Unknown".to_string()),
            tool_detail: tool_detail.unwrap_or_default(),
            action,
            rule_name,
        }))
    }

    /// Scan VSCode windows for trust dialogs or extension prompts.
    fn check_for_vscode_prompt(&mut self, pid: u32, app_name: &str) -> anyhow::Result<Option<ActionLogEntry>> {
        if let Some(last_seen) = self.known_prompts.get(&pid) {
            if last_seen.elapsed().as_secs() < 3 {
                return Ok(None);
            }
        }

        let scan = accessibility::scan_app_windows(pid as i32)?;

        // Look for workspace trust buttons
        let trust_button = scan.buttons.iter().find(|b| {
            TRUST_BUTTONS.iter().any(|&target| b.title.contains(target))
        });

        if let Some(button) = trust_button {
            let prompt = DetectedPrompt {
                source: PromptSource::Vscode,
                target_pid: pid,
                app_name: app_name.to_string(),
                prompt_text: String::new(),
                tool_name: Some("WorkspaceTrust".to_string()),
                tool_detail: None,
                detected_at: Instant::now(),
            };

            let (action, rule_name) = rules::evaluate_rules(&self.config, &prompt);
            if action == ApprovalAction::Ignore {
                return Ok(None);
            }

            if matches!(action, ApprovalAction::Approve | ApprovalAction::ApproveAlways) {
                log::info!("Clicking trust button: {:?} in VSCode (PID {})", button.title, pid);
                accessibility::click_button(button)?;
            }

            self.known_prompts.insert(pid, Instant::now());

            return Ok(Some(ActionLogEntry {
                timestamp: SystemTime::now(),
                source: PromptSource::Vscode,
                tool_name: "WorkspaceTrust".to_string(),
                tool_detail: String::new(),
                action,
                rule_name,
            }));
        }

        self.known_prompts.remove(&pid);
        Ok(None)
    }

    /// Scan system notification processes for permission banners and click "Allow".
    fn check_for_system_notification(&mut self, pid: u32, app_name: &str) -> anyhow::Result<Option<ActionLogEntry>> {
        // Use a synthetic key for dedup: offset to avoid collision with real PIDs
        let dedup_key = pid + 1_000_000;
        if let Some(last_seen) = self.known_prompts.get(&dedup_key) {
            if last_seen.elapsed().as_secs() < 3 {
                return Ok(None);
            }
        }

        let scan = accessibility::scan_app_windows(pid as i32)?;

        // Look for notification "Allow once" / "Allow" buttons
        let allow_button = scan.buttons.iter().find(|b| {
            NOTIFICATION_ALLOW_BUTTONS.iter().any(|&target| b.title == target)
        });

        if let Some(button) = allow_button {
            // Check if the text mentions a known app name to confirm it's a notification prompt
            let all_text = scan.texts.join(" ");
            let is_notification_prompt = all_text.contains("Notification")
                || all_text.contains("notification");

            if !is_notification_prompt {
                return Ok(None);
            }

            log::info!(
                "System notification permission detected in {} (PID {}): {:?}",
                app_name, pid, all_text
            );

            log::info!("Clicking notification allow button: {:?}", button.title);
            accessibility::click_button(button)?;

            self.known_prompts.insert(dedup_key, Instant::now());

            return Ok(Some(ActionLogEntry {
                timestamp: SystemTime::now(),
                source: PromptSource::ClaudeCode,
                tool_name: "Notification".to_string(),
                tool_detail: "System notification permission".to_string(),
                action: ApprovalAction::Approve,
                rule_name: Some("system-notification-allow".to_string()),
            }));
        }

        self.known_prompts.remove(&dedup_key);
        Ok(None)
    }

    pub fn process_count(&mut self) -> usize {
        let processes = process::find_claude_processes(&mut self.system);
        processes.len()
    }
}

/// Extract detail from Claude Code permission prompt text.
fn extract_detail(text: &str) -> Option<String> {
    let detail_re = Regex::new(
        r"(?i)Allow Claude to (?:Read|Write|Edit|Run|Bash|Glob|Grep|WebFetch|Web Search|WebSearch|Agent|TodoWrite|NotebookEdit|mcp\S+)\s+(.+?)\?"
    ).ok()?;
    if let Some(caps) = detail_re.captures(text) {
        let detail = caps.get(1)?.as_str().trim().to_string();
        if !detail.is_empty() {
            return Some(detail);
        }
    }

    let path_re = Regex::new(r"(/[^\s]+(?:\s[^\s/]+)*\.\w+)").ok()?;
    if let Some(caps) = path_re.captures(text) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    let backtick_re = Regex::new(r"`([^`]+)`").ok()?;
    if let Some(caps) = backtick_re.captures(text) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    None
}
