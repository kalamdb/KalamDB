//! Rotating JSONL logger for procedure invocations and V8 output.
//!
//! Writes one record per completed root CALL (`ok`/`error`) and one record per
//! V8 `console.*` / `ctx.log.*` line (`outcome=log`). Never includes request
//! bodies, arguments, results, tokens, or source. Writes are synchronous so
//! `system.procedure_logs` can be queried immediately after CALL.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::Mutex;

const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

/// One procedure log line: a root CALL outcome or a V8 console/ctx.log record.
#[derive(Debug, Clone)]
pub struct ProcedureLogRecord {
    pub execution_id: String,
    pub request_id:   String,
    pub procedure_id: String,
    pub module_id:    Option<String>,
    pub revision_id:  Option<String>,
    pub actor:        String,
    pub origin:       String,
    pub outcome:      String,
    pub channel:      String,
    pub level:        String,
    pub error_code:   Option<String>,
    pub message:      Option<String>,
    pub duration_ms:  i64,
    pub timestamp:    i64,
    pub node_id:      String,
}

/// JSONL writer for `procedures.jsonl`.
pub struct ProcedureLogLogger {
    path: Mutex<Option<PathBuf>>,
}

impl ProcedureLogLogger {
    pub fn new(log_path: String) -> Arc<Self> {
        let path = PathBuf::from(log_path);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        Arc::new(Self {
            path: Mutex::new(Some(path)),
        })
    }

    #[cfg(any(test, feature = "test-helpers"))]
    pub fn new_test() -> Self {
        Self {
            path: Mutex::new(None),
        }
    }

    pub fn record(&self, entry: ProcedureLogRecord) {
        let guard = self.path.lock();
        let Some(path) = guard.as_ref() else {
            return;
        };
        write_record(path, &entry);
    }
}

/// Strip obvious credential prefixes from procedure log text.
pub fn sanitize_procedure_log_message(message: &str) -> String {
    let mut sanitized = message.to_string();
    for secret in ["Authorization", "Bearer ", "Cookie:", "cookie="] {
        if sanitized.contains(secret) {
            sanitized = sanitized.replace(secret, "[redacted]");
        }
    }
    sanitized
}

fn write_record(path: &Path, entry: &ProcedureLogRecord) {
    let timestamp = chrono::DateTime::from_timestamp_millis(entry.timestamp)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| "unknown".to_string());
    let log_line = serde_json::json!({
        "timestamp": timestamp,
        "node_id": entry.node_id,
        "execution_id": entry.execution_id,
        "request_id": entry.request_id,
        "procedure_id": entry.procedure_id,
        "module_id": entry.module_id,
        "revision_id": entry.revision_id,
        "actor": entry.actor,
        "origin": entry.origin,
        "outcome": entry.outcome,
        "channel": entry.channel,
        "level": entry.level,
        "error_code": entry.error_code,
        "message": entry.message,
        "duration_ms": entry.duration_ms,
    })
    .to_string();
    let bytes = format!("{log_line}\n");
    rotate_if_needed(path, bytes.len() as u64);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(bytes.as_bytes());
    }
}

fn rotate_if_needed(path: &Path, incoming: u64) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len().saturating_add(incoming) <= MAX_LOG_BYTES {
        return;
    }
    let rotated = path.with_extension("jsonl.1");
    let _ = fs::remove_file(&rotated);
    let _ = fs::rename(path, rotated);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_renames_when_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("procedures.jsonl");
        fs::write(&path, vec![b'x'; (MAX_LOG_BYTES as usize) + 1]).unwrap();
        rotate_if_needed(&path, 16);
        assert!(!path.exists());
        assert!(path.with_extension("jsonl.1").exists());
    }

    #[test]
    fn record_writes_jsonl_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("procedures.jsonl");
        let logger = ProcedureLogLogger::new(path.to_string_lossy().into_owned());
        logger.record(ProcedureLogRecord {
            execution_id: "e1".into(),
            request_id:   "r1".into(),
            procedure_id: "chat.send_message".into(),
            module_id:    Some("backend".into()),
            revision_id:  Some("backend:abc".into()),
            actor:        "alice".into(),
            origin:       "sql".into(),
            outcome:      "error".into(),
            channel:      "invocation".into(),
            level:        "error".into(),
            error_code:   Some("PROCEDURE_TIMEOUT".into()),
            message:      Some("deadline exceeded".into()),
            duration_ms:  12,
            timestamp:    1_700_000_000_000,
            node_id:      "1".into(),
        });
        logger.record(ProcedureLogRecord {
            execution_id: "e1".into(),
            request_id:   "r1".into(),
            procedure_id: "chat.send_message".into(),
            module_id:    Some("backend".into()),
            revision_id:  Some("backend:abc".into()),
            actor:        "alice".into(),
            origin:       "sql".into(),
            outcome:      "log".into(),
            channel:      "console".into(),
            level:        "info".into(),
            error_code:   None,
            message:      Some("hello-from-v8".into()),
            duration_ms:  0,
            timestamp:    1_700_000_000_001,
            node_id:      "1".into(),
        });
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("chat.send_message"));
        assert!(contents.contains("PROCEDURE_TIMEOUT"));
        assert!(contents.contains("\"channel\":\"console\""));
        assert!(contents.contains("hello-from-v8"));
        assert!(!contents.contains("Authorization"));
    }

    #[test]
    fn sanitize_redacts_credential_prefixes() {
        let message = sanitize_procedure_log_message("Authorization Bearer secret Cookie: a=b");
        assert!(!message.contains("Authorization"));
        assert!(!message.contains("Bearer "));
        assert!(!message.contains("Cookie:"));
        assert!(message.contains("[redacted]"));
    }
}
