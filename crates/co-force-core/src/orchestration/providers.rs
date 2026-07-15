//! Provider specs, default registry, and auth status parsers.
//!
//! Implements Plan 08 §2-3.

use anyhow::Result;

/// Provider specification describing binary names, execution flags, and paths.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ProviderSpec {
    pub name: String,
    pub binary_names: Vec<String>,
    pub headless_command: Vec<String>,
    pub auto_approve_flags: Vec<String>,
    pub sandbox_bypass_flags: Vec<String>,
    pub resume_flags: Vec<String>,
    pub mcp_config_kind: String,
    pub auth_marker_paths: Vec<String>,
    pub auth_status_command: Vec<String>,
    pub login_hint: String,
    pub rules_files: Vec<String>,
    pub placements: Vec<String>,
}

impl ProviderSpec {
    /// Returns the default list of provider specifications.
    pub fn defaults() -> Vec<Self> {
        vec![
            ProviderSpec {
                name: "claude-code".to_string(),
                binary_names: vec!["claude".to_string()],
                headless_command: vec!["claude".to_string(), "-p".to_string(), "{prompt}".to_string()],
                auto_approve_flags: vec!["--permission-mode".to_string(), "acceptEdits".to_string()],
                sandbox_bypass_flags: vec!["--dangerously-skip-permissions".to_string()],
                resume_flags: vec!["--resume".to_string(), "{session_id}".to_string()],
                mcp_config_kind: "claude-json".to_string(),
                auth_marker_paths: vec!["~/.claude.json".to_string(), "~/.claude/auth.json".to_string()],
                auth_status_command: vec!["claude".to_string(), "auth".to_string(), "status".to_string()],
                login_hint: "claude login (subscription) · claude setup-token (headless, long-term token)".to_string(),
                rules_files: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
                placements: vec!["L1".to_string(), "L2".to_string(), "L3".to_string()],
            },
            ProviderSpec {
                name: "codex".to_string(),
                binary_names: vec!["codex".to_string()],
                headless_command: vec!["codex".to_string(), "exec".to_string(), "--json".to_string(), "{prompt}".to_string()],
                auto_approve_flags: vec!["--full-auto".to_string()],
                sandbox_bypass_flags: vec!["--dangerously-bypass-approvals-and-sandbox".to_string()],
                resume_flags: vec!["codex".to_string(), "exec".to_string(), "resume".to_string()],
                mcp_config_kind: "codex-toml".to_string(),
                auth_marker_paths: vec!["~/.codex/auth.json".to_string()],
                auth_status_command: vec!["codex".to_string(), "login".to_string(), "status".to_string()],
                login_hint: "codex login (subscription)".to_string(),
                rules_files: vec!["AGENTS.md".to_string()],
                placements: vec!["L1".to_string(), "L2".to_string(), "L3".to_string()],
            },
            ProviderSpec {
                name: "antigravity".to_string(),
                binary_names: vec!["agy".to_string()],
                headless_command: vec!["agy".to_string(), "-p".to_string(), "{prompt}".to_string()],
                auto_approve_flags: vec![],
                sandbox_bypass_flags: vec!["--dangerously-skip-permissions".to_string()],
                resume_flags: vec!["--conversation".to_string(), "{session_id}".to_string()],
                mcp_config_kind: "agy-json".to_string(),
                auth_marker_paths: vec![],
                auth_status_command: vec!["agy".to_string(), "auth".to_string(), "status".to_string()],
                login_hint: "agy login (subscription) or GOOGLE_API_KEY".to_string(),
                rules_files: vec!["AGENTS.md".to_string()],
                placements: vec!["L1".to_string(), "L2".to_string(), "L3".to_string()],
            },
            ProviderSpec {
                name: "cursor-agent".to_string(),
                binary_names: vec!["cursor-agent".to_string()],
                headless_command: vec!["cursor-agent".to_string(), "-p".to_string(), "{prompt}".to_string()],
                auto_approve_flags: vec![],
                sandbox_bypass_flags: vec![],
                resume_flags: vec![],
                mcp_config_kind: "cursor-json".to_string(),
                auth_marker_paths: vec!["~/.cursor/mcp.json".to_string()],
                auth_status_command: vec!["cursor-agent".to_string(), "status".to_string()],
                login_hint: "cursor login".to_string(),
                rules_files: vec!["AGENTS.md".to_string()],
                placements: vec!["L1".to_string(), "L2".to_string()],
            }
        ]
    }

    /// Renders headless commands with prompt, session, and cwd parameters.
    pub fn render_headless_command(&self, prompt: &str, cwd: &str, session_id: &str) -> Vec<String> {
        self.headless_command.iter().map(|arg| {
            arg.replace("{prompt}", prompt)
               .replace("{cwd}", cwd)
               .replace("{session_id}", session_id)
        }).collect()
    }
}

/// Provider authentication status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AuthStatus {
    LoggedIn,
    Expired,
    Absent,
}

/// Trait to parse vendor auth status outputs.
pub trait AuthStatusParser {
    fn parse(&self, stdout: &str, stderr: &str) -> AuthStatus;
}

pub struct ClaudeAuthParser;
impl AuthStatusParser for ClaudeAuthParser {
    fn parse(&self, stdout: &str, stderr: &str) -> AuthStatus {
        let combined = format!("{} {}", stdout, stderr).to_lowercase();
        if combined.contains("logged in as") || combined.contains("active session") {
            AuthStatus::LoggedIn
        } else if combined.contains("expired") {
            AuthStatus::Expired
        } else {
            AuthStatus::Absent
        }
    }
}

pub struct CodexAuthParser;
impl AuthStatusParser for CodexAuthParser {
    fn parse(&self, stdout: &str, stderr: &str) -> AuthStatus {
        let combined = format!("{} {}", stdout, stderr).to_lowercase();
        if combined.contains("logged in to") || combined.contains("status: ok") || combined.contains("authorized") {
            AuthStatus::LoggedIn
        } else if combined.contains("expired") || combined.contains("session expired") {
            AuthStatus::Expired
        } else {
            AuthStatus::Absent
        }
    }
}

pub struct AntigravityAuthParser;
impl AuthStatusParser for AntigravityAuthParser {
    fn parse(&self, stdout: &str, stderr: &str) -> AuthStatus {
        let combined = format!("{} {}", stdout, stderr).to_lowercase();
        if combined.contains("logged in as") || combined.contains("authorized") || combined.contains("active credentials") {
            AuthStatus::LoggedIn
        } else if combined.contains("expired") || combined.contains("credentials expired") {
            AuthStatus::Expired
        } else {
            AuthStatus::Absent
        }
    }
}

pub struct CursorAuthParser;
impl AuthStatusParser for CursorAuthParser {
    fn parse(&self, stdout: &str, stderr: &str) -> AuthStatus {
        let combined = format!("{} {}", stdout, stderr).to_lowercase();
        if combined.contains("logged in") || combined.contains("status: ok") {
            AuthStatus::LoggedIn
        } else if combined.contains("expired") {
            AuthStatus::Expired
        } else {
            AuthStatus::Absent
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_rendering() {
        let spec = &ProviderSpec::defaults()[0]; // Claude Code
        let cmd = spec.render_headless_command("hello world", "/workspace", "sess-123");
        assert_eq!(cmd, vec!["claude", "-p", "hello world"]);
    }

    #[test]
    fn test_auth_parsers() {
        let claude = ClaudeAuthParser;
        assert_eq!(claude.parse("Logged in as user@test.com", ""), AuthStatus::LoggedIn);
        assert_eq!(claude.parse("Session expired", ""), AuthStatus::Expired);
        assert_eq!(claude.parse("No credentials found", ""), AuthStatus::Absent);
    }
}
