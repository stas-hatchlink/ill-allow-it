use crate::config::{Config, Rule};
use crate::types::{ApprovalAction, DetectedPrompt};

/// Evaluate the rules against a detected prompt. First match wins.
pub fn evaluate_rules(config: &Config, prompt: &DetectedPrompt) -> (ApprovalAction, Option<String>) {
    let tool_name = prompt
        .tool_name
        .as_deref()
        .unwrap_or("Unknown");
    let tool_detail = prompt
        .tool_detail
        .as_deref()
        .unwrap_or("");

    for rule in &config.rules {
        if matches_rule(rule, tool_name, tool_detail) {
            let action = parse_action(&rule.action);
            return (action, Some(rule.name.clone()));
        }
    }

    // Fall back to default action
    let default = parse_action(&config.default_action);
    (default, None)
}

fn matches_rule(rule: &Rule, tool_name: &str, tool_detail: &str) -> bool {
    // Check tool name match (normalize spaces: "Web Search" == "WebSearch")
    if rule.tool != "*" {
        let rule_normalized = rule.tool.replace(' ', "").to_ascii_lowercase();
        let name_normalized = tool_name.replace(' ', "").to_ascii_lowercase();
        if rule_normalized != name_normalized {
            return false;
        }
    }

    // If no pattern specified, match any detail
    let pattern = match &rule.pattern {
        Some(p) => p,
        None => return true,
    };

    glob_match(pattern, tool_detail)
}

/// Simple glob matching supporting `*` (any chars) and `**` (same as * for simplicity).
fn glob_match(pattern: &str, text: &str) -> bool {
    // Convert glob to a simple regex-like matcher
    let mut chars = pattern.chars().peekable();
    let mut regex_str = String::from("^");

    while let Some(c) = chars.next() {
        match c {
            '*' => {
                // Consume consecutive *'s
                while chars.peek() == Some(&'*') {
                    chars.next();
                }
                regex_str.push_str(".*");
            }
            '?' => regex_str.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            _ => regex_str.push(c),
        }
    }

    regex_str.push('$');

    regex::Regex::new(&regex_str)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

fn parse_action(action: &str) -> ApprovalAction {
    match action {
        "approve" => ApprovalAction::Approve,
        "approve_always" => ApprovalAction::ApproveAlways,
        "deny" => ApprovalAction::Deny,
        _ => ApprovalAction::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("git status*", "git status"));
        assert!(glob_match("git status*", "git status --short"));
        assert!(!glob_match("git status*", "git push origin"));

        assert!(glob_match("npm *", "npm run build"));
        assert!(glob_match("npm *", "npm install"));
        assert!(!glob_match("npm *", "yarn install"));

        assert!(glob_match("*", "anything at all"));
        assert!(glob_match("src/**", "src/main.rs"));
        assert!(glob_match("src/**", "src/deep/nested/file.rs"));

        assert!(glob_match("git push*", "git push origin main"));
        assert!(!glob_match("git push*", "git pull origin main"));
    }

    #[test]
    fn test_matches_rule() {
        let rule = Rule {
            name: "test".to_string(),
            tool: "Bash".to_string(),
            pattern: Some("git status*".to_string()),
            action: "approve".to_string(),
        };

        assert!(matches_rule(&rule, "Bash", "git status"));
        assert!(matches_rule(&rule, "Bash", "git status --short"));
        assert!(!matches_rule(&rule, "Bash", "git push"));
        assert!(!matches_rule(&rule, "Edit", "git status"));
    }

    #[test]
    fn test_wildcard_tool() {
        let rule = Rule {
            name: "test".to_string(),
            tool: "*".to_string(),
            pattern: None,
            action: "approve".to_string(),
        };

        assert!(matches_rule(&rule, "Bash", "anything"));
        assert!(matches_rule(&rule, "Read", "anything"));
        assert!(matches_rule(&rule, "Edit", "anything"));
    }
}
