//! Structured procedure logs emitted on the process logger.

use serde_json::Value;

use crate::{error::Result, host::trait_host::FunctionHost};

const LOG_TARGET: &str = "kalamdb::functions";

/// Which JS surface produced a log record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogChannel {
    CtxLog,
    Console,
}

impl LogChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CtxLog => "ctx.log",
            Self::Console => "console",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("console") {
            Self::Console
        } else {
            Self::CtxLog
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLogRecord {
    pub channel:   LogChannel,
    pub level:     String,
    pub message:   String,
    pub error:     Option<String>,
    pub args_json: Option<String>,
}

pub fn parse_log_payload(level: &str, payload: &str, channel: &str) -> HostLogRecord {
    let channel = LogChannel::parse(channel);
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(payload) else {
        return HostLogRecord {
            channel,
            level: level.to_string(),
            message: payload.to_string(),
            error: None,
            args_json: None,
        };
    };
    let mut items = items.into_iter();
    let first = items.next();
    let (message, error, rest_start) = match first {
        Some(value) if is_error_like(&value) => {
            let error = error_text(&value);
            match items.next() {
                Some(Value::String(message)) => (message, Some(error), items.collect::<Vec<_>>()),
                Some(other) => {
                    let mut rest = vec![other];
                    rest.extend(items);
                    (error.clone(), Some(error), rest)
                },
                None => (error.clone(), Some(error), Vec::new()),
            }
        },
        Some(Value::String(message)) => (message, None, items.collect::<Vec<_>>()),
        Some(other) => {
            let mut rest = vec![other];
            rest.extend(items);
            (String::new(), None, rest)
        },
        None => (String::new(), None, Vec::new()),
    };
    let args_json = if rest_start.is_empty() {
        None
    } else {
        serde_json::to_string(&rest_start).ok()
    };
    HostLogRecord {
        channel,
        level: level.to_string(),
        message,
        error,
        args_json,
    }
}

fn is_error_like(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.contains_key("message")
        && (object.contains_key("stack")
            || object.contains_key("name")
            || object.contains_key("code"))
}

fn error_text(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return value.to_string();
    };
    let message = object.get("message").and_then(Value::as_str).unwrap_or("");
    let name = object.get("name").and_then(Value::as_str).unwrap_or("Error");
    let stack = object.get("stack").and_then(Value::as_str).unwrap_or("");
    if stack.is_empty() {
        format!("{name}: {message}")
    } else {
        format!("{name}: {message}\n{stack}")
    }
}

/// Flatten a V8 `console.*` / `ctx.log.*` record into the text operators see.
pub fn format_host_log_message(record: &HostLogRecord) -> String {
    let error = record.error.as_deref().filter(|value| !value.is_empty());
    let message = if error == Some(record.message.as_str()) {
        ""
    } else {
        record.message.as_str()
    };
    let args = record.args_json.as_deref().filter(|value| !value.is_empty());
    match (error, message, args) {
        (None, "", None) => String::new(),
        (None, message, None) => message.to_string(),
        (Some(error), "", None) => error.to_string(),
        (error, message, args) => {
            let mut out = String::new();
            if let Some(error) = error {
                out.push_str(error);
            }
            if !message.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(message);
            }
            if let Some(args) = args {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(args);
            }
            out
        },
    }
}

pub fn emit_function_log(host: &(impl FunctionHost + ?Sized), record: HostLogRecord) -> Result<()> {
    let max = host.max_log_bytes();
    let mut message = record.message;
    if message.len() > max {
        return Err(crate::FunctionsError::ResourceLimit("log message".into()));
    }
    if let Some(error) = record.error.as_ref() {
        if error.len() > max {
            return Err(crate::FunctionsError::ResourceLimit("log message".into()));
        }
    }
    if let Some(args) = record.args_json.as_ref() {
        if args.len() > max {
            return Err(crate::FunctionsError::ResourceLimit("log message".into()));
        }
    }
    if message.len()
        + record.error.as_ref().map(String::len).unwrap_or(0)
        + record.args_json.as_ref().map(String::len).unwrap_or(0)
        > max
    {
        message.truncate(max);
    }
    let level = match record.level.to_ascii_lowercase().as_str() {
        "debug" => log::Level::Debug,
        "warn" | "warning" => log::Level::Warn,
        "error" => log::Level::Error,
        _ => log::Level::Info,
    };
    let meta = host.metadata();
    let request_id = meta.as_ref().map(|m| m.request_id.as_str()).unwrap_or("");
    let actor = meta.as_ref().map(|m| m.actor.id.as_str()).unwrap_or("");
    let principal = meta.as_ref().map(|m| m.principal.id.as_str()).unwrap_or("");
    let namespace = meta.as_ref().map(|m| m.namespace.as_str()).unwrap_or("");
    let routine = host.procedure_stack();
    let channel = record.channel.as_str();
    let error = record.error.as_deref().unwrap_or("");
    let args = record.args_json.as_deref().unwrap_or("");
    log::log!(
        target: LOG_TARGET,
        level,
        "channel={channel} request_id={request_id} routine={routine} actor={actor} principal={principal} namespace={namespace} error={error} args={args} {message}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_error_object_then_message_and_args() {
        let payload = r#"[{"name":"Error","message":"boom","stack":"at x"},"failed",{"id":1}]"#;
        let record = parse_log_payload("error", payload, "ctx.log");
        assert_eq!(record.channel, LogChannel::CtxLog);
        assert_eq!(record.message, "failed");
        assert!(record.error.as_ref().is_some_and(|e| e.contains("boom")));
        assert!(record.args_json.as_ref().is_some_and(|a| a.contains("\"id\":1")));
    }

    #[test]
    fn parses_string_message() {
        let record = parse_log_payload("info", r#"["hello"]"#, "console");
        assert_eq!(record.channel, LogChannel::Console);
        assert_eq!(record.message, "hello");
        assert!(record.error.is_none());
        assert!(record.args_json.is_none());
    }

    #[test]
    fn unknown_channel_defaults_to_ctx_log() {
        let record = parse_log_payload("info", r#"["hello"]"#, "");
        assert_eq!(record.channel, LogChannel::CtxLog);
    }

    #[test]
    fn format_host_log_message_joins_v8_console_output() {
        let record = parse_log_payload("info", r#"["hello",{"n":1}]"#, "console");
        assert_eq!(format_host_log_message(&record), r#"hello [{"n":1}]"#);
    }

    #[test]
    fn format_host_log_message_prefers_error_stack_then_message() {
        let record = parse_log_payload(
            "error",
            r#"[{"name":"Error","message":"boom","stack":"Error: boom\n    at x"},"failed"]"#,
            "ctx.log",
        );
        let formatted = format_host_log_message(&record);
        assert!(formatted.contains("Error: boom"));
        assert!(formatted.contains("failed"));
    }

    #[test]
    fn format_host_log_message_does_not_duplicate_error_only_payload() {
        let record = parse_log_payload(
            "error",
            r#"[{"name":"Error","message":"boom","stack":"    at x"}]"#,
            "ctx.log",
        );
        let formatted = format_host_log_message(&record);
        assert_eq!(formatted, record.error.as_deref().unwrap());
        assert_eq!(formatted.matches("Error: boom").count(), 1);
        assert!(formatted.contains("at x"));
    }
}
