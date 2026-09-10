//! Rotating JSONL logger for procedure invocations and V8 output.
//!
//! Writes one record per completed root CALL (`ok`/`error`) and one record per
//! V8 `console.*` / `ctx.log.*` line (`outcome=log`). Never includes request
//! bodies, arguments, results, tokens, or source. Writes are synchronous so
//! `system.procedure_logs` can be queried immediately after CALL.
//!
//! On-disk layout is always local:
//! `{data_path}/functions/runtime/<procedure_id>/logs/procedures.jsonl`.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::Mutex;

const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROCEDURE_DIR_LEN: usize = 200;

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

/// JSONL writer for per-procedure `procedures.jsonl` files under the runtime root.
pub struct ProcedureLogLogger {
    runtime_root: Mutex<Option<PathBuf>>,
}

impl ProcedureLogLogger {
    pub fn new(runtime_root: impl Into<PathBuf>) -> Arc<Self> {
        let runtime_root = runtime_root.into();
        let _ = fs::create_dir_all(&runtime_root);
        Arc::new(Self {
            runtime_root: Mutex::new(Some(runtime_root)),
        })
    }

    #[cfg(any(test, feature = "test-helpers"))]
    pub fn new_test() -> Self {
        Self {
            runtime_root: Mutex::new(None),
        }
    }

    pub fn record(&self, entry: ProcedureLogRecord) {
        let root = {
            let guard = self.runtime_root.lock();
            match guard.as_ref() {
                Some(root) => root.clone(),
                None => return,
            }
        };
        let path = procedure_log_path(&root, &entry.procedure_id);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        write_record(&path, &entry);
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

/// Safe directory name for `{runtime}/{procedure_id}/logs`.
pub fn sanitize_procedure_dir_name(procedure_id: &str) -> String {
    let mut out = String::with_capacity(procedure_id.len().max(1));
    for ch in procedure_id.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.contains("..") {
        out = out.replace("..", "_");
    }
    if out.is_empty() {
        return "_unknown".to_string();
    }
    if out.len() > MAX_PROCEDURE_DIR_LEN {
        out.truncate(MAX_PROCEDURE_DIR_LEN);
    }
    out
}

pub fn procedure_log_path(runtime_root: &Path, procedure_id: &str) -> PathBuf {
    runtime_root
        .join(sanitize_procedure_dir_name(procedure_id))
        .join("logs")
        .join("procedures.jsonl")
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

    fn sample_record(procedure_id: &str, message: &str) -> ProcedureLogRecord {
        ProcedureLogRecord {
            execution_id: "e1".into(),
            request_id:   "r1".into(),
            procedure_id: procedure_id.into(),
            module_id:    Some("backend".into()),
            revision_id:  Some("backend:abc".into()),
            actor:        "alice".into(),
            origin:       "sql".into(),
            outcome:      "log".into(),
            channel:      "console".into(),
            level:        "info".into(),
            error_code:   None,
            message:      Some(message.into()),
            duration_ms:  0,
            timestamp:    1_700_000_000_000,
            node_id:      "1".into(),
        }
    }

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
    fn record_writes_jsonl_line_under_procedure_runtime_dir() {
        let dir = tempfile::tempdir().unwrap();
        let logger = ProcedureLogLogger::new(dir.path());
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
        logger.record(sample_record("chat.send_message", "hello-from-v8"));
        let path = procedure_log_path(dir.path(), "chat.send_message");
        assert_eq!(
            path,
            dir.path().join("chat.send_message").join("logs").join("procedures.jsonl")
        );
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("chat.send_message"));
        assert!(contents.contains("PROCEDURE_TIMEOUT"));
        assert!(contents.contains("\"channel\":\"console\""));
        assert!(contents.contains("hello-from-v8"));
        assert!(!contents.contains("Authorization"));
    }

    #[test]
    fn record_isolates_logs_per_procedure() {
        let dir = tempfile::tempdir().unwrap();
        let logger = ProcedureLogLogger::new(dir.path());
        logger.record(sample_record("chat.send_message", "from-chat"));
        logger.record(sample_record("billing.charge", "from-billing"));
        let chat = fs::read_to_string(procedure_log_path(dir.path(), "chat.send_message")).unwrap();
        let billing = fs::read_to_string(procedure_log_path(dir.path(), "billing.charge")).unwrap();
        assert!(chat.contains("from-chat"));
        assert!(!chat.contains("from-billing"));
        assert!(billing.contains("from-billing"));
        assert!(!billing.contains("from-chat"));
    }

    #[test]
    fn sanitize_procedure_dir_name_blocks_path_traversal() {
        assert_eq!(sanitize_procedure_dir_name("chat.send_message"), "chat.send_message");
        assert_eq!(sanitize_procedure_dir_name("ns/name"), "ns_name");
        assert_eq!(sanitize_procedure_dir_name("../etc"), "__etc");
        assert_eq!(sanitize_procedure_dir_name(""), "_unknown");
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
