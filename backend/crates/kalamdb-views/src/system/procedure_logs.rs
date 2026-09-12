//! system.procedure_logs virtual view
//!
//! Bounded tail of node-local rotating `procedures.jsonl` files. Current layout
//! is `{data_path}/functions/runtime/<procedure>/logs/procedures.jsonl`. Legacy
//! `{logs_path}/procedures.jsonl` is still read when present. Records never
//! include request bodies, arguments, results, tokens, or source. V8
//! `console.*` / `ctx.log.*` lines use `outcome=log`; root CALL completion
//! uses `outcome=ok` or `outcome=error`.

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::SystemTable;

use super::common::{
    int_col, nullable_text_col, system_view_definition, text_col, view_provider_with,
    SystemViewProvider,
};
use crate::{error::RegistryError, view_base::VirtualView};

crate::memoized_view_schema!(procedure_logs_schema, ProcedureLogsView);

const PROCEDURE_LOG_READ_BYTES: u64 = 1024 * 1024;
const PROCEDURE_LOG_MAX_ROWS: usize = 200;

#[derive(Debug)]
struct ProcedureLogEntry {
    timestamp:    String,
    node_id:      String,
    execution_id: String,
    request_id:   String,
    procedure_id: String,
    module_id:    Option<String>,
    revision_id:  Option<String>,
    actor:        String,
    origin:       String,
    outcome:      String,
    channel:      String,
    level:        String,
    error_code:   Option<String>,
    message:      Option<String>,
    duration_ms:  i64,
}

#[derive(Debug, serde::Deserialize)]
struct RawProcedureLogEntry {
    timestamp:    Option<String>,
    node_id:      Option<String>,
    execution_id: Option<String>,
    request_id:   Option<String>,
    procedure_id: Option<String>,
    module_id:    Option<String>,
    revision_id:  Option<String>,
    actor:        Option<String>,
    origin:       Option<String>,
    outcome:      Option<String>,
    channel:      Option<String>,
    level:        Option<String>,
    error_code:   Option<String>,
    message:      Option<String>,
    duration_ms:  Option<i64>,
}

impl RawProcedureLogEntry {
    fn into_entry(self) -> Option<ProcedureLogEntry> {
        let outcome = self.outcome?;
        let channel = self.channel.filter(|value| !value.is_empty()).unwrap_or_else(|| {
            if outcome == "log" {
                "console".into()
            } else {
                "invocation".into()
            }
        });
        let level = self.level.filter(|value| !value.is_empty()).unwrap_or_else(|| {
            if outcome == "error" {
                "error".into()
            } else {
                "info".into()
            }
        });
        Some(ProcedureLogEntry {
            timestamp: self.timestamp?,
            node_id: self.node_id?,
            execution_id: self.execution_id?,
            request_id: self.request_id?,
            procedure_id: self.procedure_id?,
            module_id: self.module_id.filter(|value| !value.is_empty()),
            revision_id: self.revision_id.filter(|value| !value.is_empty()),
            actor: self.actor?,
            origin: self.origin?,
            outcome,
            channel,
            level,
            error_code: self.error_code.filter(|value| !value.is_empty()),
            message: self.message.filter(|value| !value.is_empty()),
            duration_ms: self.duration_ms?,
        })
    }
}

#[derive(Debug)]
pub struct ProcedureLogsView {
    runtime_path:     PathBuf,
    legacy_logs_path: PathBuf,
}

impl ProcedureLogsView {
    pub fn new(runtime_path: PathBuf, legacy_logs_path: PathBuf) -> Self {
        Self {
            runtime_path,
            legacy_logs_path,
        }
    }

    pub fn definition() -> kalamdb_commons::schemas::TableDefinition {
        system_view_definition(
            SystemTable::ProcedureLogs,
            vec![
                text_col(1, "timestamp", "RFC 3339 timestamp"),
                text_col(2, "node_id", "Node that wrote the record"),
                text_col(3, "execution_id", "Root execution identifier"),
                text_col(4, "request_id", "Request transaction id"),
                text_col(5, "procedure_id", "Schema-qualified procedure name"),
                nullable_text_col(6, "module_id", "Module when implementation is module"),
                nullable_text_col(7, "revision_id", "Pinned module revision"),
                text_col(8, "actor", "Calling user id"),
                text_col(9, "origin", "sql | http | topic"),
                text_col(10, "outcome", "ok | error | log"),
                text_col(11, "channel", "invocation | console | ctx.log"),
                text_col(12, "level", "debug | info | warn | error"),
                nullable_text_col(13, "error_code", "Typed procedure error code"),
                nullable_text_col(14, "message", "V8 console/ctx.log text or sanitized error"),
                int_col(15, "duration_ms", "Root invocation duration; 0 for V8 log lines"),
            ],
            "Disk-backed procedure invocation, V8 console/ctx.log, and error records (no bodies \
             or secrets)",
        )
    }

    fn log_paths(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        push_jsonl_pair(&mut files, &self.legacy_logs_path);
        if let Ok(entries) = fs::read_dir(&self.runtime_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    push_jsonl_pair(&mut files, &path.join("logs"));
                }
            }
        }
        files
    }

    fn read_log_tail(path: &Path) -> Result<String, RegistryError> {
        if !path.exists() {
            return Ok(String::new());
        }
        let mut file = File::open(path).map_err(|error| {
            RegistryError::Other(format!(
                "Failed to open procedure log {}: {}",
                path.display(),
                error
            ))
        })?;
        let file_len = file
            .metadata()
            .map_err(|error| {
                RegistryError::Other(format!(
                    "Failed to stat procedure log {}: {}",
                    path.display(),
                    error
                ))
            })?
            .len();
        let offset = file_len.saturating_sub(PROCEDURE_LOG_READ_BYTES);
        file.seek(SeekFrom::Start(offset)).map_err(|error| {
            RegistryError::Other(format!(
                "Failed to seek procedure log {}: {}",
                path.display(),
                error
            ))
        })?;
        let mut content = String::new();
        file.read_to_string(&mut content).map_err(|error| {
            RegistryError::Other(format!(
                "Failed to read procedure log {}: {}",
                path.display(),
                error
            ))
        })?;
        if offset == 0 {
            return Ok(content);
        }
        Ok(content.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default())
    }

    fn read_entries(&self) -> Result<Vec<ProcedureLogEntry>, RegistryError> {
        let mut entries = Vec::new();
        for path in self.log_paths() {
            let content = Self::read_log_tail(&path)?;
            for line in content.lines().map(str::trim).filter(|line| !line.is_empty()) {
                if let Ok(raw) = serde_json::from_str::<RawProcedureLogEntry>(line) {
                    if let Some(entry) = raw.into_entry() {
                        entries.push(entry);
                    }
                }
            }
        }
        entries.sort_by_key(|entry| entry.timestamp.clone());
        if entries.len() > PROCEDURE_LOG_MAX_ROWS {
            let skip = entries.len() - PROCEDURE_LOG_MAX_ROWS;
            entries.drain(0..skip);
        }
        Ok(entries)
    }
}

impl VirtualView for ProcedureLogsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::ProcedureLogs
    }

    fn schema(&self) -> SchemaRef {
        procedure_logs_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, RegistryError> {
        let entries = self.read_entries()?;
        let mut timestamps = StringBuilder::new();
        let mut node_ids = StringBuilder::new();
        let mut execution_ids = StringBuilder::new();
        let mut request_ids = StringBuilder::new();
        let mut procedure_ids = StringBuilder::new();
        let mut module_ids = StringBuilder::new();
        let mut revision_ids = StringBuilder::new();
        let mut actors = StringBuilder::new();
        let mut origins = StringBuilder::new();
        let mut outcomes = StringBuilder::new();
        let mut channels = StringBuilder::new();
        let mut levels = StringBuilder::new();
        let mut error_codes = StringBuilder::new();
        let mut messages = StringBuilder::new();
        let mut duration_ms = Int64Builder::new();

        for entry in entries {
            timestamps.append_value(&entry.timestamp);
            node_ids.append_value(&entry.node_id);
            execution_ids.append_value(&entry.execution_id);
            request_ids.append_value(&entry.request_id);
            procedure_ids.append_value(&entry.procedure_id);
            match entry.module_id {
                Some(module_id) => module_ids.append_value(module_id),
                None => module_ids.append_null(),
            }
            match entry.revision_id {
                Some(revision_id) => revision_ids.append_value(revision_id),
                None => revision_ids.append_null(),
            }
            actors.append_value(&entry.actor);
            origins.append_value(&entry.origin);
            outcomes.append_value(&entry.outcome);
            channels.append_value(&entry.channel);
            levels.append_value(&entry.level);
            match entry.error_code {
                Some(code) => error_codes.append_value(code),
                None => error_codes.append_null(),
            }
            match entry.message {
                Some(message) => messages.append_value(message),
                None => messages.append_null(),
            }
            duration_ms.append_value(entry.duration_ms);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(timestamps.finish()) as ArrayRef,
                Arc::new(node_ids.finish()) as ArrayRef,
                Arc::new(execution_ids.finish()) as ArrayRef,
                Arc::new(request_ids.finish()) as ArrayRef,
                Arc::new(procedure_ids.finish()) as ArrayRef,
                Arc::new(module_ids.finish()) as ArrayRef,
                Arc::new(revision_ids.finish()) as ArrayRef,
                Arc::new(actors.finish()) as ArrayRef,
                Arc::new(origins.finish()) as ArrayRef,
                Arc::new(outcomes.finish()) as ArrayRef,
                Arc::new(channels.finish()) as ArrayRef,
                Arc::new(levels.finish()) as ArrayRef,
                Arc::new(error_codes.finish()) as ArrayRef,
                Arc::new(messages.finish()) as ArrayRef,
                Arc::new(duration_ms.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| {
            RegistryError::Other(format!("Failed to build procedure_logs batch: {error}"))
        })
    }
}

pub type ProcedureLogsTableProvider = SystemViewProvider<ProcedureLogsView>;

pub fn create_procedure_logs_provider(
    runtime_path: impl Into<PathBuf>,
    legacy_logs_path: impl Into<PathBuf>,
) -> ProcedureLogsTableProvider {
    view_provider_with(
        (runtime_path.into(), legacy_logs_path.into()),
        |(runtime_path, legacy_logs_path)| ProcedureLogsView::new(runtime_path, legacy_logs_path),
    )
}

fn push_jsonl_pair(files: &mut Vec<PathBuf>, logs_dir: &Path) {
    if logs_dir.as_os_str().is_empty() {
        return;
    }
    files.push(logs_dir.join("procedures.jsonl"));
    files.push(logs_dir.join("procedures.jsonl.1"));
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use datafusion::arrow::array::StringArray;
    use tempfile::tempdir;

    use super::*;

    fn write_procedure_log(runtime: &Path, procedure_id: &str, lines: &[&str]) {
        let logs_dir = runtime.join(procedure_id).join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        let mut file = std::fs::File::create(logs_dir.join("procedures.jsonl")).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    #[test]
    fn procedure_logs_view_reads_jsonl() {
        let dir = tempdir().unwrap();
        write_procedure_log(
            dir.path(),
            "chat.send_message",
            &[
                r#"{"timestamp":"2026-09-10T00:00:00.000Z","node_id":"1","execution_id":"e1","request_id":"r1","procedure_id":"chat.send_message","module_id":"backend","revision_id":"backend:abc","actor":"alice","origin":"sql","outcome":"error","error_code":"PROCEDURE_TIMEOUT","message":"deadline exceeded","duration_ms":12}"#,
                r#"{"timestamp":"2026-09-10T00:00:00.001Z","node_id":"1","execution_id":"e1","request_id":"r1","procedure_id":"chat.send_message","module_id":"backend","revision_id":"backend:abc","actor":"alice","origin":"sql","outcome":"log","channel":"console","level":"info","message":"hello-from-v8","duration_ms":0}"#,
            ],
        );
        let view = ProcedureLogsView::new(dir.path().to_path_buf(), PathBuf::new());
        let batch = view.compute_batch().expect("batch");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 15);
        let messages =
            batch.column(13).as_any().downcast_ref::<StringArray>().expect("message column");
        assert!(messages.value(1).contains("hello-from-v8"));
        let channels =
            batch.column(10).as_any().downcast_ref::<StringArray>().expect("channel column");
        assert_eq!(channels.value(0), "invocation");
        assert_eq!(channels.value(1), "console");
        assert_eq!(ProcedureLogsView::definition().table_name.as_str(), "procedure_logs");
    }

    #[test]
    fn procedure_logs_view_merges_per_procedure_and_legacy_files() {
        let runtime = tempdir().unwrap();
        let legacy = tempdir().unwrap();
        write_procedure_log(
            runtime.path(),
            "chat.send_message",
            &[
                r#"{"timestamp":"2026-09-10T00:00:00.002Z","node_id":"1","execution_id":"e2","request_id":"r2","procedure_id":"chat.send_message","actor":"alice","origin":"sql","outcome":"ok","channel":"invocation","level":"info","duration_ms":4}"#,
            ],
        );
        write_procedure_log(
            runtime.path(),
            "billing.charge",
            &[
                r#"{"timestamp":"2026-09-10T00:00:00.003Z","node_id":"1","execution_id":"e3","request_id":"r3","procedure_id":"billing.charge","actor":"alice","origin":"sql","outcome":"ok","channel":"invocation","level":"info","duration_ms":8}"#,
            ],
        );
        let mut legacy_file =
            std::fs::File::create(legacy.path().join("procedures.jsonl")).unwrap();
        writeln!(
            legacy_file,
            r#"{{"timestamp":"2026-09-10T00:00:00.001Z","node_id":"1","execution_id":"e0","request_id":"r0","procedure_id":"legacy.proc","actor":"alice","origin":"sql","outcome":"ok","channel":"invocation","level":"info","duration_ms":1}}"#
        )
        .unwrap();
        let view =
            ProcedureLogsView::new(runtime.path().to_path_buf(), legacy.path().to_path_buf());
        let batch = view.compute_batch().expect("batch");
        assert_eq!(batch.num_rows(), 3);
        let procedure_ids = batch
            .column(4)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("procedure_id column");
        let mut ids: Vec<&str> = (0..batch.num_rows()).map(|i| procedure_ids.value(i)).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["billing.charge", "chat.send_message", "legacy.proc"]);
    }
}
