//! Structured `ctx.log` records emitted on the process logger.

use serde_json::Value;

use crate::{error::Result, host::trait_host::FunctionHost};

const LOG_TARGET: &str = "kalamdb::functions";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLogRecord {
    pub level:     String,
    pub message:   String,
    pub error:     Option<String>,
    pub args_json: Option<String>,
}

pub fn parse_log_payload(level: &str, payload: &str) -> HostLogRecord {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(payload) else {
        return HostLogRecord {
            level:     level.to_string(),
            message:   payload.to_string(),
            error:     None,
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
    let error = record.error.as_deref().unwrap_or("");
    let args = record.args_json.as_deref().unwrap_or("");
    log::log!(
        target: LOG_TARGET,
        level,
        "request_id={request_id} routine={routine} actor={actor} principal={principal} namespace={namespace} error={error} args={args} {message}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_error_object_then_message_and_args() {
        let payload = r#"[{"name":"Error","message":"boom","stack":"at x"},"failed",{"id":1}]"#;
        let record = parse_log_payload("error", payload);
        assert_eq!(record.message, "failed");
        assert!(record.error.as_ref().is_some_and(|e| e.contains("boom")));
        assert!(record.args_json.as_ref().is_some_and(|a| a.contains("\"id\":1")));
    }

    #[test]
    fn parses_string_message() {
        let record = parse_log_payload("info", r#"["hello"]"#);
        assert_eq!(record.message, "hello");
        assert!(record.error.is_none());
        assert!(record.args_json.is_none());
    }
}
