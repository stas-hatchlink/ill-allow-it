use crate::accessibility;
use crate::config::Config;
use crate::keystroke;
use crate::process::{self, ClaudeProcess};
use crate::rules;
use crate::types::{ActionLogEntry, ApprovalAction, DetectedPrompt};
use regex::Regex;
use std::collections::HashMap;
use std::time::{Instant, SystemTime};
use sysinfo::System;

pub struct Monitor {
    system: System,
    config: Config,
    /// Prompts we've already acted on, keyed by claude PID.
    /// Cleared when the prompt is no longer detected.
    known_prompts: HashMap<u32, Instant>,
    /// Recent action log for display in the menu
    pub action_log: Vec<ActionLogEntry>,
    /// Regex for detecting permission prompts
    prompt_regex: Regex,
    /// Regex for extracting tool name
    tool_regex: Regex,
}

impl Monitor {
    pub fn new(config: Config) -> Self {
        // Pattern to detect Claude Code permission prompts.
        // Actual format: "Allow Claude to Read ..?" / "Allow Claude to Web Search?"
        // with buttons "Allow once", "Always allow for session", "Deny"
        let prompt_regex = Regex::new(
            r"(?si)Allow Claude to .+\?.*(?:Allow once|Deny)"
        ).expect("invalid prompt regex");

        // Pattern to extract tool name from the permission prompt.
        // Actual format: "Allow Claude to <ToolName> [detail]?"
        // e.g. "Allow Claude to Read 25-26 Cyber Policy.PDF?"
        //      "Allow Claude to Web Search?"
        //      "Allow Claude to Bash?"
        //      "Allow Claude to Edit src/main.rs?"
        let tool_regex = Regex::new(
            r"(?i)Allow Claude to (Read|Write|Edit|Bash|Glob|Grep|WebFetch|Web Search|WebSearch|Agent|TodoWrite|NotebookEdit|mcp\S+)"
        ).expect("invalid tool regex");

        Monitor {
            system: System::new(),
            config,
            known_prompts: HashMap::new(),
            action_log: Vec::new(),
            prompt_regex,
            tool_regex,
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

        let processes = process::find_claude_processes(&mut self.system);
        let mut actions_taken = 0;
        let mut still_active: Vec<u32> = Vec::new();

        for proc in &processes {
            match self.check_process(proc) {
                Ok(Some(entry)) => {
                    log::info!("Action: {}", entry);
                    self.action_log.push(entry);
                    // Keep only last 50 entries
                    if self.action_log.len() > 50 {
                        self.action_log.remove(0);
                    }
                    actions_taken += 1;
                }
                Ok(None) => {
                    // No prompt detected or already handled
                }
                Err(e) => {
                    log::debug!(
                        "Error checking process {} (parent {}): {}",
                        proc.pid,
                        proc.parent_app_name,
                        e
                    );
                }
            }
            still_active.push(proc.pid);
        }

        // Clean up prompts for processes that no longer exist
        self.known_prompts
            .retain(|pid, _| still_active.contains(pid));

        actions_taken
    }

    fn check_process(&mut self, proc: &ClaudeProcess) -> anyhow::Result<Option<ActionLogEntry>> {
        // Read the window text from the parent GUI app
        let text = accessibility::read_window_text(proc.parent_app_pid as i32)?;

        // Check if there's a permission prompt
        if !self.prompt_regex.is_match(&text) {
            // No prompt — clear any known prompt for this PID
            self.known_prompts.remove(&proc.pid);
            return Ok(None);
        }

        // We found a prompt. Check if we already acted on it.
        if let Some(last_seen) = self.known_prompts.get(&proc.pid) {
            // If we saw this less than 3 seconds ago, skip it.
            // This prevents double-acting on the same prompt.
            if last_seen.elapsed().as_secs() < 3 {
                return Ok(None);
            }
        }

        // Parse out the tool name and detail
        let prompt = self.parse_prompt(&text, proc);

        // Evaluate rules
        let (action, rule_name) = rules::evaluate_rules(&self.config, &prompt);

        if action == ApprovalAction::Ignore {
            // Don't send anything, don't mark as handled
            return Ok(None);
        }

        // Send the keystroke
        keystroke::send_keystroke(proc.parent_app_pid as i32, action)?;

        // Mark this prompt as handled
        self.known_prompts.insert(proc.pid, Instant::now());

        let entry = ActionLogEntry {
            timestamp: SystemTime::now(),
            tool_name: prompt.tool_name.unwrap_or_else(|| "Unknown".to_string()),
            tool_detail: prompt.tool_detail.unwrap_or_default(),
            action,
            rule_name,
        };

        Ok(Some(entry))
    }

    fn parse_prompt(&self, text: &str, proc: &ClaudeProcess) -> DetectedPrompt {
        let tool_name = self
            .tool_regex
            .captures(text)
            .map(|caps| caps.get(1).unwrap().as_str().to_string());

        // Extract detail: everything between the tool name and the "?"
        // e.g. "Allow Claude to Read 25-26 Cyber Policy.PDF?" -> "25-26 Cyber Policy.PDF"
        // Also check for file paths on separate lines
        let tool_detail = extract_detail(text);

        DetectedPrompt {
            claude_pid: proc.pid,
            parent_app_pid: proc.parent_app_pid,
            parent_app_name: proc.parent_app_name.clone(),
            prompt_text: text.to_string(),
            tool_name,
            tool_detail,
            detected_at: Instant::now(),
        }
    }

    pub fn process_count(&mut self) -> usize {
        let processes = process::find_claude_processes(&mut self.system);
        processes.len()
    }
}

/// Extract detail from permission prompt text.
fn extract_detail(text: &str) -> Option<String> {
    // Pattern 1: "Allow Claude to ToolName DETAIL?"
    // e.g. "Allow Claude to Read 25-26 Cyber Policy.PDF?"
    let detail_re = Regex::new(
        r"(?i)Allow Claude to (?:Read|Write|Edit|Bash|Glob|Grep|WebFetch|Web Search|WebSearch|Agent|TodoWrite|NotebookEdit|mcp\S+)\s+(.+?)\?"
    ).ok()?;
    if let Some(caps) = detail_re.captures(text) {
        let detail = caps.get(1)?.as_str().trim().to_string();
        if !detail.is_empty() {
            return Some(detail);
        }
    }

    // Pattern 2: File paths on their own line (e.g. /Users/stas/Downloads/file.pdf)
    let path_re = Regex::new(r"(/[^\s]+(?:\s[^\s/]+)*\.\w+)").ok()?;
    if let Some(caps) = path_re.captures(text) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    // Pattern 3: backtick-quoted content
    let backtick_re = Regex::new(r"`([^`]+)`").ok()?;
    if let Some(caps) = backtick_re.captures(text) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    None
}
