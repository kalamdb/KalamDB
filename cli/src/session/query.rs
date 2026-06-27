use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

use indicatif::ProgressBar;
use kalam_client::{FileUpload, UploadProgress, UploadProgressCallback};

use super::{CLISession, OutputFormat};
use crate::{
    config::expand_config_path,
    error::{CLIError, Result},
    fs_atomic::{self, FileReadPolicy},
};

#[derive(Debug, Clone)]
struct FileUploadPart {
    placeholder: String,
    filename: String,
    data: Vec<u8>,
    mime: Option<String>,
}

impl CLISession {
    /// Execute a SQL query and return the raw response.
    pub async fn execute_query_response(
        &mut self,
        sql: &str,
    ) -> Result<kalam_client::QueryResponse> {
        let (sql_to_send, mut upload_parts) = Self::extract_file_uploads(sql)?;

        self.queries_executed += 1;

        let upload_present = !upload_parts.is_empty();
        let spinner = Arc::new(Mutex::new(None::<ProgressBar>));
        if self.animations {
            if upload_present {
                let pb = Self::create_spinner("Uploading files...");
                *spinner.lock().expect("spinner lock should not be poisoned") = Some(pb);
            } else {
                let pb = Self::create_spinner("Waiting for query result...");
                *spinner.lock().expect("spinner lock should not be poisoned") = Some(pb);
            }
        }

        let upload_progress = if self.animations && upload_present {
            let spinner_clone = Arc::clone(&spinner);
            Some(Arc::new(move |progress: UploadProgress| {
                if let Some(pb) =
                    spinner_clone.lock().expect("spinner lock should not be poisoned").as_ref()
                {
                    let message = format!(
                        "Uploading {}/{}: {:>3.0}% file '{}'",
                        progress.file_index,
                        progress.total_files,
                        progress.percent,
                        progress.file_name
                    );
                    pb.set_message(message);
                }
            }) as UploadProgressCallback)
        } else {
            None
        };

        let request_namespace = self.request_namespace();

        let result = if upload_parts.is_empty() {
            self.client.execute_query(&sql_to_send, None, None, request_namespace).await
        } else {
            let mut uploads = Vec::with_capacity(upload_parts.len());
            for part in upload_parts.iter_mut() {
                let data = std::mem::take(&mut part.data);
                let mut upload = FileUpload::new(&part.placeholder, &part.filename, data);
                if let Some(mime) = part.mime.as_deref() {
                    upload = upload.with_mime(mime);
                }
                uploads.push(upload);
            }

            self.client
                .execute_query_with_progress(
                    &sql_to_send,
                    Some(uploads),
                    None,
                    request_namespace,
                    upload_progress,
                )
                .await
        };

        if let Some(pb) = spinner.lock().expect("spinner lock should not be poisoned").take() {
            pb.finish_and_clear();
        }

        result.map_err(Into::into)
    }

    pub async fn execute(&mut self, sql: &str) -> Result<()> {
        let start = Instant::now();
        let namespace_switch = Self::parse_namespace_switch(sql);

        let result = self.execute_query_response(sql).await;

        match result {
            Ok(response) => {
                if let Some(namespace) = namespace_switch {
                    self.current_namespace = Some(namespace);
                }

                if let Some((config, server_message)) =
                    Self::extract_subscription_config(&response)?
                {
                    if let Some(msg) = server_message {
                        println!("{}", msg);
                    }
                    self.run_subscription(config).await?;
                    return Ok(());
                }

                let output = self.formatter.format_response(&response)?;
                println!("{}", output);

                let elapsed = start.elapsed();
                if elapsed.as_millis() >= self.loading_threshold_ms as u128 {
                    let timing = format!("⏱  Time: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
                    let is_machine_format =
                        matches!(self.format, OutputFormat::Json | OutputFormat::Csv);
                    if self.color {
                        if is_machine_format {
                            eprintln!("{}", colored::Colorize::dimmed(timing.as_str()));
                        } else {
                            println!("{}", colored::Colorize::dimmed(timing.as_str()));
                        }
                    } else if is_machine_format {
                        eprintln!("{}", timing);
                    } else {
                        println!("{}", timing);
                    }
                }

                Ok(())
            },
            Err(e) => Err(e.into()),
        }
    }

    fn extract_file_uploads(sql: &str) -> Result<(String, Vec<FileUploadPart>)> {
        let mut modified_sql = String::with_capacity(sql.len());
        let mut specs: Vec<(String, String, Option<String>)> = Vec::new();
        let mut placeholder_counts: HashMap<String, usize> = HashMap::new();

        let mut idx = 0;
        while idx < sql.len() {
            let ch = sql[idx..].chars().next().unwrap_or('\0');
            if ch == '\'' || ch == '"' {
                let (_literal, next_idx) = Self::parse_quoted_string(sql, idx)?;
                modified_sql.push_str(&sql[idx..next_idx]);
                idx = next_idx;
                continue;
            }

            if Self::is_file_call_at(sql, idx) {
                let (next_idx, path, mime) = Self::parse_file_call(sql, idx)?;
                let placeholder = Self::build_placeholder(&path, &mut placeholder_counts);
                modified_sql.push_str(&format!("FILE(\"{}\")", placeholder));
                specs.push((placeholder, path, mime));
                idx = next_idx;
                continue;
            }

            modified_sql.push(ch);
            idx += ch.len_utf8();
        }

        if specs.is_empty() {
            return Ok((sql.to_string(), Vec::new()));
        }

        let mut uploads = Vec::with_capacity(specs.len());
        for (placeholder, path, mime) in specs {
            let expanded = expand_config_path(Path::new(&path));
            let data = fs_atomic::read_bytes(&expanded, FileReadPolicy::UserProvided).map_err(
                |error| match error.kind() {
                    std::io::ErrorKind::NotFound => CLIError::FileError(format!(
                        "File not found: {}",
                        expanded.display()
                    )),
                    _ => CLIError::FileError(format!(
                        "Failed to read file {}: {}",
                        expanded.display(),
                        error
                    )),
                },
            )?;

            let filename = expanded
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&placeholder)
                .to_string();

            uploads.push(FileUploadPart {
                placeholder,
                filename,
                data,
                mime,
            });
        }

        Ok((modified_sql, uploads))
    }

    fn is_file_call_at(sql: &str, idx: usize) -> bool {
        let bytes = sql.as_bytes();
        let needle = b"file";
        if idx + needle.len() > bytes.len() {
            return false;
        }

        if bytes[idx..idx + needle.len()] != *needle {
            return false;
        }

        if idx > 0 {
            if let Some(prev) = sql[..idx].chars().last() {
                if Self::is_ident_char(prev) {
                    return false;
                }
            }
        }

        let mut j = idx + needle.len();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }

        j < bytes.len() && bytes[j] == b'('
    }

    fn parse_file_call(sql: &str, start: usize) -> Result<(usize, String, Option<String>)> {
        let bytes = sql.as_bytes();
        let mut idx = start + 4;
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }

        if idx >= bytes.len() || bytes[idx] != b'(' {
            return Err(CLIError::ParseError("Invalid file() syntax: expected '('".into()));
        }
        idx += 1;

        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }

        let (path, mut idx) = Self::parse_quoted_string(sql, idx)?;

        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }

        let mut mime: Option<String> = None;
        if idx < bytes.len() && bytes[idx] == b',' {
            idx += 1;
            while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
                idx += 1;
            }

            let (mime_value, next_idx) = Self::parse_quoted_string(sql, idx)?;
            mime = Some(mime_value);
            idx = next_idx;
        }

        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }

        if idx >= bytes.len() || bytes[idx] != b')' {
            return Err(CLIError::ParseError("Invalid file() syntax: expected ')'".into()));
        }

        Ok((idx + 1, path, mime))
    }

    fn parse_quoted_string(sql: &str, start: usize) -> Result<(String, usize)> {
        let bytes = sql.as_bytes();
        if start >= bytes.len() {
            return Err(CLIError::ParseError(
                "Invalid file() syntax: expected string literal".into(),
            ));
        }

        let quote = bytes[start];
        if quote != b'\'' && quote != b'"' {
            return Err(CLIError::ParseError(
                "Invalid file() syntax: expected quoted string".into(),
            ));
        }

        let mut out = String::new();
        let mut idx = start + 1;
        while idx < bytes.len() {
            let b = bytes[idx];
            if b == quote {
                if idx + 1 < bytes.len() && bytes[idx + 1] == quote {
                    out.push(quote as char);
                    idx += 2;
                    continue;
                }
                return Ok((out, idx + 1));
            }

            if b == b'\\' {
                if idx + 1 >= bytes.len() {
                    return Err(CLIError::ParseError(
                        "Invalid file() syntax: unterminated escape".into(),
                    ));
                }
                let next = bytes[idx + 1];
                out.push(next as char);
                idx += 2;
                continue;
            }

            out.push(b as char);
            idx += 1;
        }

        Err(CLIError::ParseError("Invalid file() syntax: unterminated string".into()))
    }

    fn build_placeholder(path: &str, counts: &mut HashMap<String, usize>) -> String {
        let filename = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or("file");

        let mut base = filename
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();

        if base.is_empty() {
            base = "file".to_string();
        }

        let count = counts.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            base
        } else {
            format!("{}_{}", base, count)
        }
    }

    fn is_ident_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_'
    }
}
