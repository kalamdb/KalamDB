//! Rotating JSONL logger for procedure invocations, V8 output, and runtime lifecycle.
//!
//! Writes one record per completed root CALL (`ok`/`error`), one record per
//! V8 `console.*` / `ctx.log.*` line (`outcome=log`, `channel=console`), and
//! isolate/deploy events (`channel=lifecycle`: created, reused, idle, dropped,
//! deployed). Scheduled invocations stamp `schedule_id`; other origins omit that
//! field. Never includes request bodies, arguments, results, tokens, or source.
//! Writes are synchronous so `system.procedure_logs` can be queried immediately
//! after CALL.
//!
//! Cost per record is kept to one JSON serialization into a single buffer and one
//! `open`/`write` pair; directories are only created on the first miss and rotation
//! uses the already-open handle's `fstat`.
//!
//! On-disk layout is always local:
//! `{data_path}/functions/runtime/<procedure_id>/logs/procedures.jsonl`.

use std::{
    borrow::Cow,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Serialize, Serializer};

const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PROCEDURE_DIR_LEN: usize = 200;
const LINE_CAPACITY: usize = 512;

/// One procedure log line: a root CALL outcome, a V8 console/ctx.log record, or a lifecycle event.
///
/// Enumerated fields are `Cow` so the common `&'static str` values do not allocate.
#[derive(Debug, Clone)]
pub struct ProcedureLogRecord {
    pub execution_id: String,
    pub request_id:   String,
    pub procedure_id: String,
    pub module_id:    Option<String>,
    pub revision_id:  Option<String>,
    pub actor:        Cow<'static, str>,
    pub origin:       Cow<'static, str>,
    pub outcome:      Cow<'static, str>,
    pub channel:      Cow<'static, str>,
    pub level:        Cow<'static, str>,
    pub error_code:   Option<String>,
    pub message:      Option<String>,
    pub duration_ms:  i64,
    pub timestamp:    i64,
    pub node_id:      String,
    /// Present when `origin` is `schedule`; omitted from JSONL otherwise.
    pub schedule_id:  Option<String>,
}

/// JSONL writer for per-procedure `procedures.jsonl` files under the runtime root.
pub struct ProcedureLogLogger {
    runtime_root: Option<PathBuf>,
}

impl ProcedureLogLogger {
    pub fn new(runtime_root: impl Into<PathBuf>) -> Arc<Self> {
        let runtime_root = runtime_root.into();
        let _ = fs::create_dir_all(&runtime_root);
        Arc::new(Self {
            runtime_root: Some(runtime_root),
        })
    }

    #[cfg(any(test, feature = "test-helpers"))]
    pub fn new_test() -> Self {
        Self { runtime_root: None }
    }

    pub fn record(&self, entry: ProcedureLogRecord) {
        let Some(root) = self.runtime_root.as_deref() else {
            return;
        };
        write_record(&procedure_log_path(root, &entry.procedure_id), &entry);
    }
}

/// Strip obvious credential prefixes from procedure log text. Returns the input untouched
/// (no copy) when nothing needs redacting.
pub fn sanitize_procedure_log_message(message: String) -> String {
    const SECRETS: [&str; 4] = ["Authorization", "Bearer ", "Cookie:", "cookie="];
    if !SECRETS.iter().any(|secret| message.contains(secret)) {
        return message;
    }
    let mut sanitized = message;
    for secret in SECRETS {
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

/// Borrowed wire shape; serialized straight into the line buffer.
#[derive(Serialize)]
struct LogLine<'a> {
    #[serde(serialize_with = "serialize_rfc3339_millis")]
    timestamp:    i64,
    node_id:      &'a str,
    execution_id: &'a str,
    request_id:   &'a str,
    procedure_id: &'a str,
    module_id:    Option<&'a str>,
    revision_id:  Option<&'a str>,
    actor:        &'a str,
    origin:       &'a str,
    outcome:      &'a str,
    channel:      &'a str,
    level:        &'a str,
    error_code:   Option<&'a str>,
    message:      Option<&'a str>,
    duration_ms:  i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    schedule_id:  Option<&'a str>,
}

impl<'a> From<&'a ProcedureLogRecord> for LogLine<'a> {
    fn from(entry: &'a ProcedureLogRecord) -> Self {
        Self {
            timestamp:    entry.timestamp,
            node_id:      &entry.node_id,
            execution_id: &entry.execution_id,
            request_id:   &entry.request_id,
            procedure_id: &entry.procedure_id,
            module_id:    entry.module_id.as_deref(),
            revision_id:  entry.revision_id.as_deref(),
            actor:        &entry.actor,
            origin:       &entry.origin,
            outcome:      &entry.outcome,
            channel:      &entry.channel,
            level:        &entry.level,
            error_code:   entry.error_code.as_deref(),
            message:      entry.message.as_deref(),
            duration_ms:  entry.duration_ms,
            schedule_id:  entry.schedule_id.as_deref(),
        }
    }
}

fn serialize_rfc3339_millis<S: Serializer>(millis: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    match chrono::DateTime::from_timestamp_millis(*millis) {
        Some(dt) => serializer.collect_str(&dt.format("%Y-%m-%dT%H:%M:%S%.3fZ")),
        None => serializer.serialize_str("unknown"),
    }
}

fn write_record(path: &Path, entry: &ProcedureLogRecord) {
    let mut line = Vec::with_capacity(LINE_CAPACITY);
    if serde_json::to_writer(&mut line, &LogLine::from(entry)).is_err() {
        return;
    }
    line.push(b'\n');

    let Some(mut file) = open_append(path) else {
        return;
    };
    let over_limit = file
        .metadata()
        .map(|meta| meta.len().saturating_add(line.len() as u64) > MAX_LOG_BYTES)
        .unwrap_or(false);
    if over_limit {
        drop(file);
        rotate(path);
        let Some(reopened) = open_append(path) else {
            return;
        };
        file = reopened;
    }
    let _ = file.write_all(&line);
}

/// Open for append; create the procedure directory only when the first open reports it missing.
fn open_append(path: &Path) -> Option<File> {
    let open = || OpenOptions::new().create(true).append(true).open(path);
    match open() {
        Ok(file) => Some(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path.parent()?).ok()?;
            open().ok()
        },
        Err(_) => None,
    }
}

fn rotate(path: &Path) {
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
            schedule_id:  None,
        }
    }

    #[test]
    fn write_rotates_when_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("procedures.jsonl");
        fs::write(&path, vec![b'x'; (MAX_LOG_BYTES as usize) + 1]).unwrap();
        write_record(&path, &sample_record("p", "after-rotate"));
        let rotated = path.with_extension("jsonl.1");
        assert!(rotated.exists());
        assert_eq!(fs::metadata(&rotated).unwrap().len(), MAX_LOG_BYTES + 1);
        let fresh = fs::read_to_string(&path).unwrap();
        assert!(fresh.contains("after-rotate"));
        assert!(fresh.ends_with('\n'));
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
            schedule_id:  None,
        });
        logger.record(sample_record("chat.send_message", "hello-from-v8"));
        let path = procedure_log_path(dir.path(), "chat.send_message");
        assert_eq!(
            path,
            dir.path().join("chat.send_message").join("logs").join("procedures.jsonl")
        );
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"timestamp\":\"2023-11-14T22:13:20.000Z\""));
        assert!(contents.contains("chat.send_message"));
        assert!(contents.contains("PROCEDURE_TIMEOUT"));
        assert!(contents.contains("\"channel\":\"console\""));
        assert!(contents.contains("\"module_id\":\"backend\""));
        assert!(contents.contains("hello-from-v8"));
        assert!(!contents.contains("Authorization"));
        assert!(!contents.contains("schedule_id"));
        assert_eq!(contents.lines().count(), 2);
    }

    #[test]
    fn record_includes_schedule_id_only_when_set() {
        let dir = tempfile::tempdir().unwrap();
        let logger = ProcedureLogLogger::new(dir.path());
        let mut scheduled = sample_record("reports.summary", "from-schedule");
        scheduled.origin = "schedule".into();
        scheduled.channel = "invocation".into();
        scheduled.outcome = "ok".into();
        scheduled.schedule_id = Some("reports.daily".into());
        logger.record(scheduled);
        let contents =
            fs::read_to_string(procedure_log_path(dir.path(), "reports.summary")).unwrap();
        assert!(contents.contains("\"schedule_id\":\"reports.daily\""));
        assert!(contents.contains("\"origin\":\"schedule\""));
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
        let message =
            sanitize_procedure_log_message("Authorization Bearer secret Cookie: a=b".into());
        assert!(!message.contains("Authorization"));
        assert!(!message.contains("Bearer "));
        assert!(!message.contains("Cookie:"));
        assert!(message.contains("[redacted]"));
        assert_eq!(sanitize_procedure_log_message("plain".into()), "plain");
    }
}
