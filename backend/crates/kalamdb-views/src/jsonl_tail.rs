//! Reverse JSONL tail reader for virtual log views.
//!
//! Walks a file backward from EOF in small chunks, like `tail -n`. A 1GB log
//! stays on disk: only the newest parseable lines are kept, and a single huge
//! line is dropped instead of filling the scan.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use serde::de::DeserializeOwned;

use crate::error::RegistryError;

/// Caps for one reverse scan of a JSONL (or JSON-lines) log file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonlTailLimits {
    /// Newest parseable lines to keep from this file.
    pub max_rows:       usize,
    /// Reverse-read block size.
    pub chunk_bytes:    usize,
    /// Stop reverse scanning after this many bytes even if fewer lines were found.
    pub max_tail_bytes: usize,
    /// Drop a single line larger than this so one giant message cannot fill the tail.
    pub max_line_bytes: usize,
}

impl JsonlTailLimits {
    /// Default caps used by `system.server_logs` and sibling log views.
    pub const DEFAULT: Self = Self {
        max_rows:       256,
        chunk_bytes:    8 * 1024,
        max_tail_bytes: 256 * 1024,
        max_line_bytes: 64 * 1024,
    };

    /// Override [`Self::max_rows`] while keeping the other default caps.
    #[must_use]
    pub const fn with_max_rows(mut self, max_rows: usize) -> Self {
        self.max_rows = max_rows;
        self
    }
}

impl Default for JsonlTailLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Trim a candidate log line, rejecting empty, non-UTF8, and oversized records.
pub fn utf8_log_line(bytes: &[u8], max_line_bytes: usize) -> Option<&str> {
    if bytes.is_empty() || bytes.len() > max_line_bytes {
        return None;
    }
    let line = std::str::from_utf8(bytes).ok()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

/// Parse one JSONL record, then map it through `into_entry`.
pub fn parse_json_log_line<Raw, Entry>(
    bytes: &[u8],
    max_line_bytes: usize,
    into_entry: impl FnOnce(Raw) -> Option<Entry>,
) -> Option<Entry>
where
    Raw: DeserializeOwned,
{
    let line = utf8_log_line(bytes, max_line_bytes)?;
    into_entry(serde_json::from_str(line).ok()?)
}

/// Sort oldest-first with `cmp` and keep only the newest `max_rows` items.
///
/// Use this after merging tails from several files (procedure logs, rotated
/// siblings) so the virtual view still emits a single bounded batch.
pub fn keep_newest_by<T, F>(mut entries: Vec<T>, max_rows: usize, cmp: F) -> Vec<T>
where
    F: FnMut(&T, &T) -> std::cmp::Ordering,
{
    if max_rows == 0 {
        return Vec::new();
    }
    entries.sort_by(cmp);
    if entries.len() > max_rows {
        let skip = entries.len() - max_rows;
        entries.drain(0..skip);
    }
    entries
}

/// Read the newest parseable lines from `path`, walking backward from EOF.
///
/// Missing files yield an empty vec. Returned lines are oldest-first among the
/// kept tail.
///
/// # Errors
///
/// Returns [`RegistryError::Other`] when the file cannot be opened, stated,
/// seeked, or read.
pub fn read_jsonl_tail<T>(
    path: &Path,
    limits: JsonlTailLimits,
    mut parse_line: impl FnMut(&[u8]) -> Option<T>,
) -> Result<Vec<T>, RegistryError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut file = File::open(path).map_err(|error| {
        RegistryError::Other(format!("Failed to open log file {}: {}", path.display(), error))
    })?;
    let mut pos = file
        .metadata()
        .map_err(|error| {
            RegistryError::Other(format!("Failed to stat log file {}: {}", path.display(), error))
        })?
        .len();
    if pos == 0 || limits.max_rows == 0 {
        return Ok(Vec::new());
    }

    let mut leftover = Vec::new();
    let mut newest_first = Vec::with_capacity(limits.max_rows);
    let mut scanned = 0usize;

    while pos > 0 && newest_first.len() < limits.max_rows && scanned < limits.max_tail_bytes {
        let chunk_len = limits.chunk_bytes.min(pos as usize);
        pos -= chunk_len as u64;
        file.seek(SeekFrom::Start(pos)).map_err(|error| {
            RegistryError::Other(format!("Failed to seek log file {}: {}", path.display(), error))
        })?;

        let mut chunk = vec![0u8; chunk_len];
        let read = file.read(&mut chunk).map_err(|error| {
            RegistryError::Other(format!("Failed to read log file {}: {}", path.display(), error))
        })?;
        chunk.truncate(read);
        scanned = scanned.saturating_add(read);
        if read == 0 {
            break;
        }

        chunk.append(&mut leftover);
        split_newest_lines(
            chunk,
            &mut leftover,
            &mut newest_first,
            limits.max_rows,
            &mut parse_line,
        );
        if leftover.len() > limits.max_line_bytes {
            leftover.clear();
        }
    }

    if pos == 0 && newest_first.len() < limits.max_rows {
        if let Some(entry) = parse_line(&leftover) {
            newest_first.push(entry);
        }
    }

    newest_first.reverse();
    Ok(newest_first)
}

fn split_newest_lines<T>(
    mut chunk: Vec<u8>,
    leftover: &mut Vec<u8>,
    newest_first: &mut Vec<T>,
    max_rows: usize,
    parse_line: &mut impl FnMut(&[u8]) -> Option<T>,
) {
    while newest_first.len() < max_rows {
        let Some(newline_at) = chunk.iter().rposition(|&byte| byte == b'\n') else {
            break;
        };
        let line = chunk.split_off(newline_at + 1);
        chunk.pop();
        if let Some(entry) = parse_line(&line) {
            newest_first.push(entry);
        }
    }
    *leftover = chunk;
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;

    use super::*;

    fn parse_text(bytes: &[u8]) -> Option<String> {
        utf8_log_line(bytes, JsonlTailLimits::DEFAULT.max_line_bytes).map(str::to_string)
    }

    fn write_lines(path: &std::path::Path, lines: &[&str]) {
        let mut file = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    #[test]
    fn missing_file_returns_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.jsonl");
        let lines = read_jsonl_tail(&path, JsonlTailLimits::DEFAULT, parse_text).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn keeps_only_the_newest_max_rows() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("app.jsonl");
        let total = JsonlTailLimits::DEFAULT.max_rows + 12;
        let lines: Vec<String> = (0..total).map(|index| format!("row-{index}")).collect();
        write_lines(&path, &lines.iter().map(String::as_str).collect::<Vec<_>>());

        let kept = read_jsonl_tail(&path, JsonlTailLimits::DEFAULT, parse_text).unwrap();
        assert_eq!(kept.len(), JsonlTailLimits::DEFAULT.max_rows);
        assert_eq!(kept[0], format!("row-{}", total - JsonlTailLimits::DEFAULT.max_rows));
        assert_eq!(kept.last().unwrap(), &format!("row-{}", total - 1));
    }

    #[test]
    fn skips_oversized_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("app.jsonl");
        let huge = "x".repeat(JsonlTailLimits::DEFAULT.max_line_bytes + 1);
        write_lines(&path, &["before", &huge, "after"]);

        let kept = read_jsonl_tail(&path, JsonlTailLimits::DEFAULT, parse_text).unwrap();
        assert_eq!(kept, ["before", "after"]);
    }

    #[test]
    fn reads_final_line_without_trailing_newline() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("app.jsonl");
        std::fs::write(&path, "one\ntwo").unwrap();

        let kept = read_jsonl_tail(&path, JsonlTailLimits::DEFAULT, parse_text).unwrap();
        assert_eq!(kept, ["one", "two"]);
    }

    #[test]
    fn stops_after_max_tail_bytes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("app.jsonl");
        let line = "abcdefghijklmnopqrstuvwxyz";
        let mut file = std::fs::File::create(&path).unwrap();
        for _ in 0..50 {
            writeln!(file, "{line}").unwrap();
        }
        let limits = JsonlTailLimits {
            max_rows:       50,
            chunk_bytes:    16,
            max_tail_bytes: 80,
            max_line_bytes: 64,
        };
        let kept = read_jsonl_tail(&path, limits, parse_text).unwrap();
        assert!(!kept.is_empty());
        assert!(kept.len() < 50);
        assert!(kept.iter().all(|value| value == line));
    }

    #[test]
    fn keep_newest_by_drops_older_merged_rows() {
        let kept = keep_newest_by(vec![("a", 1), ("c", 3), ("b", 2)], 2, |left, right| {
            left.1.cmp(&right.1)
        });
        assert_eq!(kept, [("b", 2), ("c", 3)]);
    }
}
