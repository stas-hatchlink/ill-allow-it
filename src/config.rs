use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub vscode_enabled: bool,
    pub poll_interval_ms: u64,
    pub default_action: String,
    pub rules: Vec<Rule>,
    pub log_actions: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    /// Optional source filter: "claude_code", "vscode", or omitted to match any source
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Tool name to match: "Bash", "Edit", "Write", "Read", "WebFetch", "WorkspaceTrust", "*" for any
    pub tool: String,
    /// Glob pattern to match against the tool detail (command, file path, etc.)
    /// If omitted, matches any detail for the given tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Action: "approve", "approve_always", "deny", "ignore"
    pub action: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            vscode_enabled: true,
            poll_interval_ms: 500,
            default_action: "ignore".to_string(),
            rules: vec![
                // Claude Code rules
                // Using approve_always (Cmd+Enter) which works in both:
                // - CLI: triggers "Always allow for session"
                // - Desktop app: triggers "Allow once" (which requires Cmd+Enter)
                Rule {
                    name: "Allow file reads".to_string(),
                    source: None,
                    tool: "Read".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Allow glob searches".to_string(),
                    source: None,
                    tool: "Glob".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Allow grep searches".to_string(),
                    source: None,
                    tool: "Grep".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Allow web search".to_string(),
                    source: None,
                    tool: "Web Search".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Allow web fetch".to_string(),
                    source: None,
                    tool: "WebFetch".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Allow file edits".to_string(),
                    source: None,
                    tool: "Edit".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Allow file writes".to_string(),
                    source: None,
                    tool: "Write".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Deny git push".to_string(),
                    source: None,
                    tool: "Bash".to_string(),
                    pattern: Some("git push*".to_string()),
                    action: "deny".to_string(),
                },
                Rule {
                    name: "Allow all Bash".to_string(),
                    source: None,
                    tool: "Bash".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Deny git push (Run)".to_string(),
                    source: None,
                    tool: "Run".to_string(),
                    pattern: Some("*git push*".to_string()),
                    action: "deny".to_string(),
                },
                Rule {
                    name: "Allow all Run".to_string(),
                    source: None,
                    tool: "Run".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                // VSCode rules
                Rule {
                    name: "Auto-trust VSCode workspaces".to_string(),
                    source: Some("vscode".to_string()),
                    tool: "WorkspaceTrust".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
                Rule {
                    name: "Allow Claude VSCode extension prompts".to_string(),
                    source: Some("vscode".to_string()),
                    tool: "ClaudeExtension".to_string(),
                    pattern: None,
                    action: "approve_always".to_string(),
                },
            ],
            log_actions: true,
        }
    }
}

fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ill-allow-it")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn log_path() -> PathBuf {
    config_dir().join("actions.log")
}

pub fn load_config() -> Result<Config> {
    let path = config_path();

    if !path.exists() {
        // Create default config
        let config = Config::default();
        save_config(&config)?;
        return Ok(config);
    }

    let contents = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config from {}", path.display()))?;

    let config: Config = serde_json::from_str(&contents)
        .with_context(|| format!("Failed to parse config from {}", path.display()))?;

    Ok(config)
}

pub fn save_config(config: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create config directory {}", dir.display()))?;

    let path = config_path();
    let contents = serde_json::to_string_pretty(config)?;
    fs::write(&path, contents)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;

    Ok(())
}

pub fn config_mtime() -> Option<SystemTime> {
    fs::metadata(config_path())
        .ok()
        .and_then(|m| m.modified().ok())
}
