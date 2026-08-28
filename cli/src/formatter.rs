//! Output formatters for query results
//!
//! **Implements T086**: OutputFormatter for table/JSON/CSV formats using tabled
//!
//! Provides consistent, colorized output formatting for query results.

use kalam_client::{ErrorDetail, KalamDataType, QueryResponse, TimestampFormatter};
use serde_json::Value as JsonValue;

use crate::{error::Result, session::OutputFormat};

/// Maximum column width before truncation
const MAX_COLUMN_WIDTH: usize = 32;

/// Minimum column width when resizing to fit the terminal
const MIN_COLUMN_WIDTH: usize = 6;

/// Formats query results for display
pub struct OutputFormatter {
    format:              OutputFormat,
    color:               bool,
    timestamp_formatter: TimestampFormatter,
}

impl OutputFormatter {
    /// Create a new formatter
    pub fn new(format: OutputFormat, color: bool, timestamp_formatter: TimestampFormatter) -> Self {
        Self {
            format,
            color,
            timestamp_formatter,
        }
    }

    /// Format a consumer record for display
    pub fn format_consumer_record(
        &self,
        offset: u64,
        operation: &str,
        payload_bytes: &[u8],
    ) -> String {
        // Parse payload as JSON
        let payload: JsonValue = serde_json::from_slice(payload_bytes)
            .unwrap_or_else(|_| JsonValue::String("<invalid json>".to_string()));

        match self.format {
            OutputFormat::Table => {
                let payload_str =
                    serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_string());
                format!("[offset={}] {}: {}", offset, operation, payload_str)
            },
            OutputFormat::Json => {
                let record = serde_json::json!({
                    "offset": offset,
                    "operation": operation,
                    "payload": payload,
                });
                serde_json::to_string(&record).unwrap_or_else(|_| "{}".to_string())
            },
            OutputFormat::Csv => {
                let payload_str =
                    serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_string());
                format!("{},{},{}", offset, operation, payload_str)
            },
        }
    }

    /// Get terminal width, defaulting to 80 if unavailable
    fn get_terminal_width() -> usize {
        if let Some((w, _h)) = term_size::dimensions() {
            w
        } else {
            80 // Default fallback
        }
    }

    /// Truncate a string to max width with ellipsis
    fn truncate_value(value: &str, max_width: usize) -> String {
        if value.len() <= max_width {
            value.to_string()
        } else if max_width <= 3 {
            value.chars().take(max_width).collect()
        } else {
            let take = max_width - 3;
            format!("{}...", value.chars().take(take).collect::<String>())
        }
    }

    /// Format a query response
    pub fn format_response(&self, response: &QueryResponse) -> Result<String> {
        if let Some(ref error) = response.error {
            return Ok(self.format_error_detail(error));
        }

        let as_user = Self::query_as_user(response);

        match self.format {
            OutputFormat::Table => self.format_table(response, as_user),
            OutputFormat::Json => self.format_json(response),
            OutputFormat::Csv => self.format_csv(response),
        }
    }

    fn query_as_user(response: &QueryResponse) -> Option<&str> {
        response
            .results
            .first()
            .and_then(|result| result.as_user.as_deref())
            .filter(|value| !value.trim().is_empty())
    }

    fn is_explain_plan_result(columns: &[String]) -> bool {
        columns.len() == 2
            && columns[0].eq_ignore_ascii_case("plan_type")
            && columns[1].eq_ignore_ascii_case("plan")
    }

    /// Render DataFusion EXPLAIN output without the generic 32-char cap and with
    /// multi-line plan cells expanded across table rows (psql-style).
    fn format_explain_plan_table(
        &self,
        columns: &[String],
        rows: &[Vec<kalam_client::KalamCellValue>],
        result: &kalam_client::QueryResult,
        exec_time_ms: f64,
        as_user: Option<&str>,
    ) -> Result<String> {
        let terminal_width = Self::get_terminal_width();

        let mut string_rows: Vec<[String; 2]> = Vec::with_capacity(rows.len());
        for row in rows {
            let plan_type = row
                .first()
                .map(|v| {
                    self.format_json_value_with_type(v, result.schema.first().map(|f| &f.data_type))
                })
                .unwrap_or_else(|| "NULL".to_string());
            let plan = row
                .get(1)
                .map(|v| {
                    self.format_json_value_with_type(v, result.schema.get(1).map(|f| &f.data_type))
                })
                .unwrap_or_else(|| "NULL".to_string());
            string_rows.push([plan_type, plan]);
        }

        struct ExplainDisplayRow {
            plan_type: String,
            plan_line: String,
        }

        let mut display_rows: Vec<ExplainDisplayRow> = Vec::new();
        for [plan_type, plan] in &string_rows {
            let lines: Vec<&str> = if plan.is_empty() {
                vec![""]
            } else {
                plan.split('\n').collect()
            };
            for (line_idx, line) in lines.iter().enumerate() {
                display_rows.push(ExplainDisplayRow {
                    plan_type: if line_idx == 0 {
                        plan_type.clone()
                    } else {
                        String::new()
                    },
                    plan_line: (*line).to_string(),
                });
            }
        }

        let plan_type_header = &columns[0];
        let mut plan_type_width = plan_type_header.len();
        for row in &display_rows {
            plan_type_width = plan_type_width.max(row.plan_type.len());
        }

        let border_padding = 7; // two columns: "│ " + " │" + separator between cols
        let available_for_plan = terminal_width.saturating_sub(border_padding + plan_type_width);
        let mut plan_width = display_rows
            .iter()
            .map(|row| row.plan_line.len())
            .max()
            .unwrap_or(columns[1].len());
        plan_width = plan_width.max(columns[1].len());
        if available_for_plan >= MIN_COLUMN_WIDTH {
            plan_width = plan_width.min(available_for_plan);
        }
        plan_width = plan_width.max(MIN_COLUMN_WIDTH);

        let col_widths = [plan_type_width, plan_width];
        let mut output = String::new();

        output.push('┌');
        for (idx, width) in col_widths.iter().enumerate() {
            output.push_str(&"─".repeat(width + 2));
            output.push(if idx == col_widths.len() - 1 {
                '┐'
            } else {
                '┬'
            });
        }
        output.push('\n');

        output.push('│');
        for (i, col) in columns.iter().enumerate() {
            output.push(' ');
            output.push_str(&format!("{:width$}", col, width = col_widths[i]));
            output.push(' ');
            output.push('│');
        }
        output.push('\n');

        output.push('├');
        for (idx, width) in col_widths.iter().enumerate() {
            output.push_str(&"─".repeat(width + 2));
            output.push(if idx == col_widths.len() - 1 {
                '┤'
            } else {
                '┼'
            });
        }
        output.push('\n');

        for row in &display_rows {
            output.push('│');
            output.push(' ');
            output.push_str(&format!("{:width$}", row.plan_type, width = col_widths[0]));
            output.push(' ');
            output.push('│');
            output.push(' ');
            let plan_cell = if row.plan_line.len() <= col_widths[1] {
                row.plan_line.clone()
            } else {
                Self::truncate_value(&row.plan_line, col_widths[1])
            };
            output.push_str(&format!("{:width$}", plan_cell, width = col_widths[1]));
            output.push(' ');
            output.push('│');
            output.push('\n');
        }

        output.push('└');
        for (idx, width) in col_widths.iter().enumerate() {
            output.push_str(&"─".repeat(width + 2));
            output.push(if idx == col_widths.len() - 1 {
                '┘'
            } else {
                '┴'
            });
        }
        output.push('\n');

        let row_count = string_rows.len();
        let row_label = if row_count == 1 { "row" } else { "rows" };
        output.push_str(&format!("({} {})\n\n", row_count, row_label));
        output.push_str(&Self::format_table_footer(exec_time_ms, as_user));

        Ok(output)
    }

    fn format_table_footer(exec_time_ms: f64, as_user: Option<&str>) -> String {
        let mut footer = String::new();
        if let Some(as_user) = as_user {
            footer.push_str(&format!("As: {as_user}\n"));
        }
        footer.push_str(&format!("Took: {:.3} ms", exec_time_ms));
        footer
    }

    /// Format as table
    fn format_table(&self, response: &QueryResponse, as_user: Option<&str>) -> Result<String> {
        if response.results.is_empty() {
            let exec_time_ms = response.took.unwrap_or(0.0);
            return Ok(format!(
                "Query OK, 0 rows affected\n\n{}",
                Self::format_table_footer(exec_time_ms, as_user)
            ));
        }

        let result = &response.results[0];
        let exec_time_ms = response.took.unwrap_or(0.0);

        // Check if this is a message-only result (DDL statements)
        if let Some(ref message) = result.message {
            // Format DDL message similar to MySQL/PostgreSQL
            let row_count = result.row_count;
            return Ok(format!(
                "{}\nQuery OK, {} rows affected\n\n{}",
                message,
                row_count,
                Self::format_table_footer(exec_time_ms, as_user)
            ));
        }

        // Handle data results
        if let Some(ref rows) = result.rows {
            // Get column names from schema
            let columns: Vec<String> = result.column_names();

            if Self::is_explain_plan_result(&columns) {
                return self.format_explain_plan_table(
                    &columns,
                    rows,
                    result,
                    exec_time_ms,
                    as_user,
                );
            }

            let terminal_width = Self::get_terminal_width();

            // Precompute string values once to avoid double formatting
            // Rows are now arrays of values ordered by schema index
            let mut string_rows: Vec<Vec<String>> = Vec::with_capacity(rows.len());
            let mut col_widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
            for row in rows {
                let mut srow: Vec<String> = Vec::with_capacity(columns.len());
                for (i, _col) in columns.iter().enumerate() {
                    let data_type = result.schema.get(i).map(|field| &field.data_type);
                    let value = row
                        .get(i)
                        .map(|v| self.format_json_value_with_type(v, data_type))
                        .unwrap_or_else(|| "NULL".to_string());
                    col_widths[i] = col_widths[i].max(value.len());
                    srow.push(value);
                }
                string_rows.push(srow);
            }

            let column_count = col_widths.len();
            if column_count > 0 {
                // Calculate available width for columns
                let border_padding = column_count * 3 + 1;
                let mut available = terminal_width.saturating_sub(border_padding);
                if available < column_count {
                    available = column_count;
                }

                // Only truncate if total width exceeds available space
                let mut total_width = col_widths.iter().sum::<usize>();
                if total_width > available {
                    // First pass: cap at MAX_COLUMN_WIDTH if needed
                    for width in col_widths.iter_mut() {
                        if *width > MAX_COLUMN_WIDTH {
                            *width = MAX_COLUMN_WIDTH;
                        }
                    }
                    total_width = col_widths.iter().sum();

                    // Second pass: shrink columns to fit terminal if still too wide
                    while total_width > available {
                        if let Some((idx, _)) = col_widths
                            .iter()
                            .enumerate()
                            .filter(|(_, width)| **width > MIN_COLUMN_WIDTH)
                            .max_by_key(|(_, width)| *width)
                        {
                            col_widths[idx] -= 1;
                        } else if let Some((idx, _)) = col_widths
                            .iter()
                            .enumerate()
                            .filter(|(_, width)| **width > 1)
                            .max_by_key(|(_, width)| *width)
                        {
                            col_widths[idx] -= 1;
                        } else {
                            break;
                        }
                        total_width = col_widths.iter().sum();
                    }
                }
            }

            let mut output = String::new();

            // Top border
            output.push('┌');
            for (idx, width) in col_widths.iter().enumerate() {
                output.push_str(&"─".repeat(width + 2));
                output.push(if idx == col_widths.len() - 1 {
                    '┐'
                } else {
                    '┬'
                });
            }
            output.push('\n');

            // Header row
            output.push('│');
            for (i, col) in columns.iter().enumerate() {
                output.push(' ');
                let truncated = Self::truncate_value(col, col_widths[i]);
                output.push_str(&format!("{:width$}", truncated, width = col_widths[i]));
                output.push(' ');
                output.push('│');
            }
            output.push('\n');

            // Header separator
            output.push('├');
            for (idx, width) in col_widths.iter().enumerate() {
                output.push_str(&"─".repeat(width + 2));
                output.push(if idx == col_widths.len() - 1 {
                    '┤'
                } else {
                    '┼'
                });
            }
            output.push('\n');

            // Data rows
            for srow in &string_rows {
                output.push('│');
                for (i, value) in srow.iter().enumerate() {
                    output.push(' ');
                    let truncated = Self::truncate_value(value, col_widths[i]);
                    output.push_str(&format!("{:width$}", truncated, width = col_widths[i]));
                    output.push(' ');
                    output.push('│');
                }
                output.push('\n');
            }

            // Bottom border
            output.push('└');
            for (idx, width) in col_widths.iter().enumerate() {
                output.push_str(&"─".repeat(width + 2));
                output.push(if idx == col_widths.len() - 1 {
                    '┘'
                } else {
                    '┴'
                });
            }
            output.push('\n');

            let row_count = string_rows.len();
            let row_label = if row_count == 1 { "row" } else { "rows" };
            output.push_str(&format!("({} {})\n", row_count, row_label));
            // Add blank line for psql-style formatting
            output.push('\n');
            output.push_str(&Self::format_table_footer(exec_time_ms, as_user));

            Ok(output)
        } else {
            // Non-query statement (INSERT, UPDATE, DELETE)
            let row_count = result.row_count;
            let exec_time_ms = response.took.unwrap_or(0.0);
            Ok(format!(
                "Query OK, {} rows affected\n\n{}",
                row_count,
                Self::format_table_footer(exec_time_ms, as_user)
            ))
        }
    }

    /// Format as JSON
    fn format_json(&self, response: &QueryResponse) -> Result<String> {
        let json = serde_json::to_string_pretty(response)
            .map_err(|e| crate::error::CLIError::FormatError(e.to_string()))?;
        Ok(json)
    }

    /// Format as CSV
    fn format_csv(&self, response: &QueryResponse) -> Result<String> {
        if response.results.is_empty() {
            return Ok("".to_string());
        }

        let result = &response.results[0];

        // Handle message-only results
        if result.rows.is_none() {
            return Ok("".to_string());
        }

        let rows = result.rows.as_ref().unwrap();
        if rows.is_empty() {
            return Ok("".to_string());
        }

        // Extract columns from schema
        let columns: Vec<String> = result.column_names();

        // Build CSV
        let mut output = columns.join(",") + "\n";

        for row in rows {
            let values: Vec<String> = columns
                .iter()
                .enumerate()
                .map(|(i, _col)| {
                    let data_type = result.schema.get(i).map(|field| &field.data_type);
                    row.get(i)
                        .map(|v| self.format_csv_value_with_type(v, data_type))
                        .unwrap_or_else(|| "".to_string())
                })
                .collect();
            output.push_str(&values.join(","));
            output.push('\n');
        }

        Ok(output)
    }

    /// Format error detail (with code and details) - MySQL/PostgreSQL style
    fn format_error_detail(&self, error: &ErrorDetail) -> String {
        let mut output = String::new();

        if self.color {
            output.push_str(&format!("\x1b[31mERROR {}\x1b[0m: {}\n", error.code, error.message));
        } else {
            output.push_str(&format!("ERROR {}: {}\n", error.code, error.message));
        }

        if let Some(ref details) = error.details {
            output.push_str(&format!("Details: {}", details));
        }

        output
    }

    /// Format JSON value for table display
    /// Handles both simple JSON values and typed ScalarValue format from the server
    fn format_json_value(&self, value: &JsonValue) -> String {
        match value {
            JsonValue::Null => "NULL".to_string(),
            JsonValue::Bool(b) => b.to_string(),
            JsonValue::Number(n) => n.to_string(),
            JsonValue::String(s) => s.clone(),
            JsonValue::Object(map) if map.len() == 1 => {
                // Handle typed ScalarValue format: {"Int64": "42"}, {"Utf8": "hello"}, etc.
                let (type_name, inner_value) = map.iter().next().unwrap();
                match type_name.as_str() {
                    "Null" => "NULL".to_string(),
                    "Boolean" => inner_value
                        .as_bool()
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "NULL".to_string()),
                    "Int8" | "Int16" | "Int32" | "Int64" | "UInt8" | "UInt16" | "UInt32"
                    | "UInt64" => {
                        // These are stored as strings to preserve precision
                        inner_value.as_str().unwrap_or("NULL").to_string()
                    },
                    "Float32" | "Float64" => inner_value
                        .as_f64()
                        .map(|f| f.to_string())
                        .unwrap_or_else(|| "NULL".to_string()),
                    "Utf8" | "LargeUtf8" => inner_value.as_str().unwrap_or("NULL").to_string(),
                    "Binary" | "LargeBinary" => {
                        // Binary data - show as hex or indicate it's binary
                        "<binary>".to_string()
                    },
                    "Date32" => inner_value
                        .as_i64()
                        .map(|days| self.timestamp_formatter.format(Some(days * 86_400_000)))
                        .unwrap_or_else(|| "NULL".to_string()),
                    "Time64Microsecond" => inner_value
                        .as_i64()
                        .map(|micros| self.timestamp_formatter.format(Some(micros / 1000)))
                        .unwrap_or_else(|| "NULL".to_string()),
                    "Time64Nanosecond" => inner_value
                        .as_i64()
                        .map(|nanos| self.timestamp_formatter.format(Some(nanos / 1_000_000)))
                        .unwrap_or_else(|| "NULL".to_string()),
                    "TimestampSecond"
                    | "TimestampMillisecond"
                    | "TimestampMicrosecond"
                    | "TimestampNanosecond" => {
                        // Timestamp objects have 'value' and optionally 'timezone'
                        if let Some(obj) = inner_value.as_object() {
                            if let Some(val) = obj.get("value").and_then(|v| v.as_i64()) {
                                let ms = match type_name.as_str() {
                                    "TimestampSecond" => val * 1000,
                                    "TimestampMillisecond" => val,
                                    "TimestampMicrosecond" => val / 1000,
                                    "TimestampNanosecond" => val / 1_000_000,
                                    _ => val,
                                };
                                return self.timestamp_formatter.format(Some(ms));
                            }
                        }
                        "NULL".to_string()
                    },
                    "Decimal128" => {
                        // Decimal has 'value', 'precision', 'scale'
                        if let Some(obj) = inner_value.as_object() {
                            if let Some(val) = obj.get("value") {
                                return val.as_str().unwrap_or("NULL").to_string();
                            }
                        }
                        "NULL".to_string()
                    },
                    "FixedSizeList" => {
                        // Complex array type - show abbreviated
                        "<array>".to_string()
                    },
                    _ => {
                        // Fallback for unknown types - show the inner value
                        inner_value.to_string()
                    },
                }
            },
            JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
        }
    }

    fn format_json_value_with_type(
        &self,
        value: &JsonValue,
        data_type: Option<&KalamDataType>,
    ) -> String {
        match data_type {
            Some(KalamDataType::Timestamp) => self.format_timestamp_value(value),
            Some(KalamDataType::Date) => self.format_date_value(value),
            Some(KalamDataType::DateTime) => self.format_timestamp_value(value),
            _ => self.format_json_value(value),
        }
    }

    fn format_date_value(&self, value: &JsonValue) -> String {
        self.parse_i64(value)
            .map(|days| self.timestamp_formatter.format(Some(days * 86_400_000)))
            .unwrap_or_else(|| "NULL".to_string())
    }

    fn format_timestamp_value(&self, value: &JsonValue) -> String {
        match value {
            JsonValue::Object(map) if map.len() == 1 => {
                let (type_name, inner_value) = map.iter().next().unwrap();
                match type_name.as_str() {
                    "TimestampSecond"
                    | "TimestampMillisecond"
                    | "TimestampMicrosecond"
                    | "TimestampNanosecond" => {
                        if let Some(obj) = inner_value.as_object() {
                            if let Some(val) = obj.get("value").and_then(|v| v.as_i64()) {
                                let ms = match type_name.as_str() {
                                    "TimestampSecond" => val * 1000,
                                    "TimestampMillisecond" => val,
                                    "TimestampMicrosecond" => val / 1000,
                                    "TimestampNanosecond" => val / 1_000_000,
                                    _ => val,
                                };
                                return self.timestamp_formatter.format(Some(ms));
                            }
                        }
                        "NULL".to_string()
                    },
                    "Date32" => self.format_date_value(inner_value),
                    "Time64Microsecond" => self
                        .parse_i64(inner_value)
                        .map(|micros| self.timestamp_formatter.format(Some(micros / 1000)))
                        .unwrap_or_else(|| "NULL".to_string()),
                    "Time64Nanosecond" => self
                        .parse_i64(inner_value)
                        .map(|nanos| self.timestamp_formatter.format(Some(nanos / 1_000_000)))
                        .unwrap_or_else(|| "NULL".to_string()),
                    _ => self.format_json_value(inner_value),
                }
            },
            _ => {
                let ms = self.parse_i64(value).map(|raw| self.raw_timestamp_millis(raw));
                ms.map(|val| self.timestamp_formatter.format(Some(val)))
                    .unwrap_or_else(|| "NULL".to_string())
            },
        }
    }

    fn raw_timestamp_millis(&self, raw: i64) -> i64 {
        let magnitude = raw.abs();
        if magnitude >= 100_000_000_000_000_000 {
            raw / 1_000_000
        } else if magnitude >= 100_000_000_000_000 {
            raw / 1000
        } else if magnitude >= 100_000_000_000 {
            raw
        } else {
            raw * 1000
        }
    }

    fn parse_i64(&self, value: &JsonValue) -> Option<i64> {
        match value {
            JsonValue::Number(n) => n.as_i64(),
            JsonValue::String(s) => s.parse::<i64>().ok(),
            _ => None,
        }
    }

    /// Format JSON value for CSV (escape commas and quotes)
    fn format_csv_value_with_type(
        &self,
        value: &JsonValue,
        data_type: Option<&KalamDataType>,
    ) -> String {
        let s = self.format_json_value_with_type(value, data_type);
        if s.contains(',') || s.contains('"') || s.contains('\n') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use kalam_client::{query::models::ResponseStatus, QueryResult, SchemaField, TimestampFormat};
    use serde_json::json;

    use super::*;

    #[test]
    fn test_format_json_value() {
        let formatter = OutputFormatter::new(
            OutputFormat::Table,
            false,
            TimestampFormatter::new(TimestampFormat::Iso8601),
        );
        assert_eq!(formatter.format_json_value(&JsonValue::Null), "NULL");
        assert_eq!(formatter.format_json_value(&JsonValue::Bool(true)), "true");
        assert_eq!(formatter.format_json_value(&JsonValue::String("test".into())), "test");
    }

    #[test]
    fn test_csv_escaping() {
        let formatter = OutputFormatter::new(
            OutputFormat::Csv,
            false,
            TimestampFormatter::new(TimestampFormat::Iso8601),
        );
        let value = JsonValue::String("hello, world".into());
        assert_eq!(formatter.format_csv_value_with_type(&value, None), "\"hello, world\"");
    }

    #[test]
    fn test_truncate_value() {
        // No truncation needed
        assert_eq!(OutputFormatter::truncate_value("short", 10), "short");

        // Truncation with ellipsis
        assert_eq!(
            OutputFormatter::truncate_value("this is a very long string that needs truncation", 20),
            "this is a very lo..."
        );

        // Edge case: max_width = 3 (can't fit ellipsis, just truncate)
        assert_eq!(OutputFormatter::truncate_value("test", 3), "tes");

        // Edge case: max_width < 3 (just truncate)
        assert_eq!(OutputFormatter::truncate_value("test", 2), "te");

        // Edge case: exactly at max_width = 4
        assert_eq!(OutputFormatter::truncate_value("test", 4), "test");

        // Edge case: one over max_width with ellipsis
        assert_eq!(OutputFormatter::truncate_value("hello", 4), "h...");
    }

    #[test]
    fn test_terminal_width_detection() {
        // Should return a reasonable default if terminal size unavailable
        let width = OutputFormatter::get_terminal_width();
        assert!(width >= 80); // Should be at least 80 columns
    }

    #[test]
    fn test_format_timestamp_value_auto_infers_milliseconds() {
        let formatter = OutputFormatter::new(
            OutputFormat::Table,
            false,
            TimestampFormatter::new(TimestampFormat::Iso8601),
        );

        let rendered = formatter.format_json_value_with_type(
            &JsonValue::Number(1735689600000_i64.into()),
            Some(&KalamDataType::Timestamp),
        );

        assert_eq!(rendered, "2025-01-01T00:00:00.000Z");
    }

    #[test]
    fn test_raw_timestamp_millis_auto_handles_microseconds_and_nanoseconds() {
        let formatter = OutputFormatter::new(
            OutputFormat::Table,
            false,
            TimestampFormatter::new(TimestampFormat::Iso8601),
        );

        assert_eq!(formatter.raw_timestamp_millis(1735689600000000), 1735689600000);
        assert_eq!(formatter.raw_timestamp_millis(1735689600000000000), 1735689600000);
    }

    #[test]
    fn test_format_explain_plan_expands_multiline_without_short_cap() {
        let formatter = OutputFormatter::new(
            OutputFormat::Table,
            false,
            TimestampFormatter::new(TimestampFormat::Iso8601),
        );

        let long_plan = "GlobalLimitExec: skip=0, fetch=3\n  FilterExec: user_id = test\n  \
                         DeferredBatchExec: hot_rows_scanned=12";
        let response = QueryResponse {
            status:  ResponseStatus::Success,
            results: vec![QueryResult {
                schema:     vec![
                    SchemaField {
                        name:      "plan_type".to_string(),
                        data_type: KalamDataType::Text,
                        index:     0,
                        flags:     None,
                    },
                    SchemaField {
                        name:      "plan".to_string(),
                        data_type: KalamDataType::Text,
                        index:     1,
                        flags:     None,
                    },
                ],
                rows:       Some(vec![vec![
                    json!("Plan with Metrics").into(),
                    json!(long_plan).into(),
                ]]),
                named_rows: None,
                row_count:  1,
                message:    None,
                as_user:    None,
            }],
            took:    Some(1.0),
            error:   None,
        };

        let output = formatter.format_response(&response).unwrap();
        assert!(output.contains("GlobalLimitExec: skip=0, fetch=3"));
        assert!(output.contains("DeferredBatchExec: hot_rows_scanned=12"));
        assert!(!output.contains("fetc..."));
        assert!(output.contains("(1 row)"));
    }

    #[test]
    fn test_format_table_footer_includes_effective_user() {
        let formatter = OutputFormatter::new(
            OutputFormat::Table,
            false,
            TimestampFormatter::new(TimestampFormat::Iso8601),
        );

        let response = QueryResponse {
            status:  ResponseStatus::Success,
            results: vec![QueryResult {
                schema:     vec![SchemaField {
                    name:      "name".to_string(),
                    data_type: KalamDataType::Text,
                    index:     0,
                    flags:     None,
                }],
                rows:       Some(vec![vec![json!("Alice").into()]]),
                named_rows: None,
                row_count:  1,
                message:    None,
                as_user:    Some("alice".to_string()),
            }],
            took:    Some(2.146),
            error:   None,
        };

        let output = formatter.format_response(&response).unwrap();
        assert!(output.contains("(1 row)\n\nAs: alice\nTook: 2.146 ms"));
    }
}
