//! Stable, machine-readable CLI errors for agent and automation mode.

use std::fmt;

/// Stable identifier for an agent-mode CLI failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentErrorCode {
    AuthRequired,
    DestructiveSchemaChange,
    DevStartFailed,
    MigrationFailed,
    PortInUse,
    SchemaFailed,
    ServerBinaryMissing,
    ServerDownloadFailed,
    ServerStartFailed,
    ServerUnavailable,
}

impl AgentErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::DestructiveSchemaChange => "DESTRUCTIVE_SCHEMA_CHANGE",
            Self::DevStartFailed => "DEV_START_FAILED",
            Self::MigrationFailed => "MIGRATION_FAILED",
            Self::PortInUse => "PORT_IN_USE",
            Self::SchemaFailed => "SCHEMA_FAILED",
            Self::ServerBinaryMissing => "SERVER_BINARY_MISSING",
            Self::ServerDownloadFailed => "SERVER_DOWNLOAD_FAILED",
            Self::ServerStartFailed => "SERVER_START_FAILED",
            Self::ServerUnavailable => "SERVER_UNAVAILABLE",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "AUTH_REQUIRED" => Some(Self::AuthRequired),
            "DESTRUCTIVE_SCHEMA_CHANGE" => Some(Self::DestructiveSchemaChange),
            "DEV_START_FAILED" => Some(Self::DevStartFailed),
            "MIGRATION_FAILED" => Some(Self::MigrationFailed),
            "PORT_IN_USE" => Some(Self::PortInUse),
            "SCHEMA_FAILED" => Some(Self::SchemaFailed),
            "SERVER_BINARY_MISSING" => Some(Self::ServerBinaryMissing),
            "SERVER_DOWNLOAD_FAILED" => Some(Self::ServerDownloadFailed),
            "SERVER_START_FAILED" => Some(Self::ServerStartFailed),
            "SERVER_UNAVAILABLE" => Some(Self::ServerUnavailable),
            _ => None,
        }
    }
}

/// Compact agent-mode failure with a stable code, optional fields, and an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentError {
    pub code:    AgentErrorCode,
    pub fields:  Vec<(String, String)>,
    pub message: String,
    pub action:  Option<String>,
}

impl AgentError {
    pub fn new(code: AgentErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            fields: Vec::new(),
            message: message.into(),
            action: None,
        }
    }

    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push((key.into(), value.into()));
        self
    }

    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    pub fn render(&self) -> String {
        let mut line = format!("KALAM_ERROR code={}", self.code.as_str());
        for (key, value) in &self.fields {
            line.push(' ');
            line.push_str(key);
            line.push('=');
            line.push_str(&quote_agent_field(value));
        }

        let mut out = line;
        if !self.message.is_empty() {
            out.push('\n');
            out.push('\n');
            out.push_str(self.message.trim());
        }
        if let Some(action) = &self.action {
            out.push('\n');
            out.push_str("Action:\n");
            out.push_str(action.trim());
        }
        out
    }

    pub fn destructive_schema_change(object: &str) -> Self {
        Self::new(
            AgentErrorCode::DestructiveSchemaChange,
            format!("schema.sql requires rebuilding {object}."),
        )
        .with_field("object", object)
        .with_action(
            "run `kalam dev --force` if you intentionally want to rebuild development data.",
        )
    }

    pub fn server_download_failed(version: &str, detail: &str) -> Self {
        Self::new(
            AgentErrorCode::ServerDownloadFailed,
            format!("Could not download the compatible KalamDB server.\n{detail}"),
        )
        .with_field("version", version)
        .with_action("check network access or set KALAMDB_SERVER_BIN.")
    }

    pub fn server_unavailable(url: &str) -> Self {
        Self::new(
            AgentErrorCode::ServerUnavailable,
            format!("Could not reach KalamDB server at {url}."),
        )
        .with_field("url", url)
        .with_action("start KalamDB or rerun `kalam dev --agent` to start a local server.")
    }

    pub fn migration_failed(migration: &str, detail: &str) -> Self {
        Self::new(AgentErrorCode::MigrationFailed, detail.to_string())
            .with_field("migration", migration)
            .with_action("inspect the migration and retry `kalam dev --agent`.")
    }

    pub fn schema_failed(detail: &str) -> Self {
        Self::new(AgentErrorCode::SchemaFailed, detail.to_string())
            .with_action("fix schema.sql and retry `kalam dev --agent`.")
    }

    pub fn auth_required(profile: &str) -> Self {
        Self::new(
            AgentErrorCode::AuthRequired,
            format!("Authentication is required for profile `{profile}`."),
        )
        .with_field("profile", profile)
        .with_action("kalam login")
    }

    pub fn server_start_failed(detail: &str) -> Self {
        let mut error = Self::new(AgentErrorCode::ServerStartFailed, detail.to_string())
            .with_action("inspect the server log or set KALAMDB_SERVER_BIN.");
        if let Some(port) = parse_port_in_use(detail) {
            error.code = AgentErrorCode::PortInUse;
            error.fields.push(("port".into(), port));
            error.action =
                Some("stop the process using that port or change the KalamDB URL.".into());
        }
        error
    }

    pub fn dev_start_failed(detail: &str) -> Self {
        Self::new(AgentErrorCode::DevStartFailed, detail.to_string())
            .with_action("inspect `kalam dev logs` or retry `kalam dev start --agent`.")
    }

    pub fn from_log_code(code: &str, detail: &str) -> Option<Self> {
        let parsed = AgentErrorCode::from_code(code)?;
        Some(Self::new(parsed, detail.to_string()))
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render())
    }
}

pub fn quote_agent_field(value: &str) -> String {
    if value.is_empty() || value.chars().any(|ch: char| ch.is_whitespace() || ch == '"') {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

/// Compact `KALAM_*` event line used by `kalam dev --agent`.
pub fn format_agent_event(event: &str, fields: &[(&str, &str)]) -> String {
    let mut line = event.to_string();
    for (key, value) in fields {
        line.push(' ');
        line.push_str(key);
        line.push('=');
        line.push_str(&quote_agent_field(value));
    }
    line
}

/// Compact JSON object for `kalam ... --json` agent/workflow events.
pub fn format_agent_event_json(event: &str, fields: &[(&str, &str)]) -> String {
    let mut map = serde_json::Map::new();
    map.insert("event".into(), serde_json::Value::String(event.to_string()));
    for (key, value) in fields {
        map.insert((*key).into(), serde_json::Value::String((*value).to_string()));
    }
    serde_json::Value::Object(map).to_string()
}

fn parse_port_in_use(detail: &str) -> Option<String> {
    let lower = detail.to_ascii_lowercase();
    if !lower.contains("already in use") {
        return None;
    }
    if let Some(port) = detail.split(':').find_map(port_token) {
        return Some(port);
    }
    detail.split(|ch: char| !ch.is_ascii_digit()).find_map(port_token)
}

fn port_token(part: &str) -> Option<String> {
    let trimmed = part.trim();
    if (2..=5).contains(&trimmed.len()) && trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_code_fields_message_and_action() {
        let error = AgentError::destructive_schema_change("table `messages`");
        let rendered = error.render();
        assert!(rendered.starts_with("KALAM_ERROR code=DESTRUCTIVE_SCHEMA_CHANGE"));
        assert!(rendered.contains("object=\"table `messages`\""));
        assert!(rendered.contains("schema.sql requires rebuilding table `messages`."));
        assert!(rendered.contains("Action:"));
        assert!(rendered.contains("kalam dev --force"));
    }

    #[test]
    fn quote_agent_field_quotes_whitespace() {
        assert_eq!(quote_agent_field("http://127.0.0.1:2900"), "http://127.0.0.1:2900");
        assert_eq!(quote_agent_field("npm run dev"), "\"npm run dev\"");
    }

    #[test]
    fn format_agent_event_is_compact() {
        let line = format_agent_event(
            "KALAM_READY",
            &[
                ("url", "http://127.0.0.1:2900"),
                ("namespace", "task_app"),
                ("schema", "applied"),
                ("types", "typescript"),
            ],
        );
        assert_eq!(
            line,
            "KALAM_READY url=http://127.0.0.1:2900 namespace=task_app schema=applied \
             types=typescript"
        );
    }

    #[test]
    fn server_start_failed_maps_port_in_use() {
        let error =
            AgentError::server_start_failed("listen tcp 127.0.0.1:2900: address already in use");
        assert_eq!(error.code, AgentErrorCode::PortInUse);
        assert!(error.fields.iter().any(|(key, value)| key == "port" && value == "2900"));

        let error = AgentError::server_start_failed("port 5432 already in use");
        assert_eq!(error.code, AgentErrorCode::PortInUse);
        assert!(error.fields.iter().any(|(key, value)| key == "port" && value == "5432"));
    }
}
