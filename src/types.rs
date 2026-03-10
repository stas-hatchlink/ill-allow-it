use std::fmt;
use std::time::{Instant, SystemTime};

/// macOS virtual keycodes
pub const KEYCODE_RETURN: u16 = 0x24; // Enter/Return
pub const KEYCODE_ESCAPE: u16 = 0x35; // Escape

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAction {
    /// Send Enter keystroke (Allow once)
    Approve,
    /// Send Cmd+Enter keystroke (Always allow for session)
    ApproveAlways,
    /// Send Escape keystroke (Deny)
    Deny,
    /// Do nothing, let user decide
    Ignore,
}

impl fmt::Display for ApprovalAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApprovalAction::Approve => write!(f, "Approved"),
            ApprovalAction::ApproveAlways => write!(f, "Approved (always)"),
            ApprovalAction::Deny => write!(f, "Denied"),
            ApprovalAction::Ignore => write!(f, "Ignored"),
        }
    }
}

impl ApprovalAction {
    /// The virtual keycode to send for this action, if any.
    pub fn keycode(&self) -> Option<u16> {
        match self {
            ApprovalAction::Approve => Some(KEYCODE_RETURN),       // Enter = Allow once
            ApprovalAction::ApproveAlways => Some(KEYCODE_RETURN), // Cmd+Enter = Always allow
            ApprovalAction::Deny => Some(KEYCODE_ESCAPE),          // Esc = Deny
            ApprovalAction::Ignore => None,
        }
    }

    /// Whether this action requires the Cmd modifier key.
    pub fn needs_cmd(&self) -> bool {
        matches!(self, ApprovalAction::ApproveAlways)
    }
}

#[derive(Debug, Clone)]
pub struct DetectedPrompt {
    /// PID of the `claude` process
    pub claude_pid: u32,
    /// PID of the parent GUI application (Terminal.app, Claude.app, etc.)
    pub parent_app_pid: u32,
    /// Name of the parent application
    pub parent_app_name: String,
    /// The full text of the permission prompt
    pub prompt_text: String,
    /// Parsed tool name (e.g., "Bash", "Edit", "Write", "Read")
    pub tool_name: Option<String>,
    /// Parsed detail (e.g., the command or file path)
    pub tool_detail: Option<String>,
    /// When we first detected this prompt
    pub detected_at: Instant,
}

#[derive(Debug, Clone)]
pub struct ActionLogEntry {
    pub timestamp: SystemTime,
    pub tool_name: String,
    pub tool_detail: String,
    pub action: ApprovalAction,
    pub rule_name: Option<String>,
}

impl fmt::Display for ActionLogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tool = if self.tool_detail.is_empty() {
            self.tool_name.clone()
        } else {
            format!("{}({})", self.tool_name, self.tool_detail)
        };
        write!(f, "{}: {}", self.action, tool)
    }
}
