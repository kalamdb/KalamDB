//! Canonical PostgreSQL setting defaults and validation, shared by every transport.
use crate::SearchPath;

pub const SERVER_VERSION: &str = "16.0";
pub const SERVER_VERSION_NUM: &str = "160000";
pub const DEFAULT_SEARCH_PATH: &str = "\"$user\", public, default";
pub const SETTINGS: &[(&str, &str)] = &[
    ("server_version", SERVER_VERSION),
    ("server_version_num", SERVER_VERSION_NUM),
    ("server_encoding", "UTF8"),
    ("client_encoding", "UTF8"),
    ("timezone", "UTC"),
    ("search_path", DEFAULT_SEARCH_PATH),
    ("integer_datetimes", "on"),
    ("standard_conforming_strings", "on"),
    ("transaction_isolation", "read committed"),
    ("default_transaction_isolation", "read committed"),
    ("transaction_read_only", "off"),
    ("datestyle", "ISO, MDY"),
    ("extra_float_digits", "3"),
    ("application_name", ""),
    ("max_index_keys", "32"),
    ("max_identifier_length", "63"),
    ("statement_timeout", "0"),
    ("lock_timeout", "0"),
    ("idle_in_transaction_session_timeout", "0"),
    ("bytea_output", "hex"),
    ("intervalstyle", "postgres"),
];

pub fn canonical_name(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "time zone" => "timezone".into(),
        "transaction isolation level" => "transaction_isolation".into(),
        "default transaction isolation level" => "default_transaction_isolation".into(),
        "transaction read only" => "transaction_read_only".into(),
        other => other.into(),
    }
}

pub fn default_value(name: &str) -> Option<&'static str> {
    SETTINGS.iter().find(|(key, _)| *key == name).map(|(_, value)| *value)
}

pub fn is_custom(name: &str) -> bool {
    name.contains('.')
        && name.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
        })
}

pub fn validate(name: &str, value: Option<&str>) -> Result<String, String> {
    if name.len() > 256 || value.is_some_and(|v| v.len() > 8192) {
        return Err("session setting exceeds size limit".into());
    }
    let default = default_value(name);
    if default.is_none() && !is_custom(name) {
        return Err(format!("unrecognized configuration parameter \"{name}\""));
    }
    let value = value.unwrap_or(default.unwrap_or(""));
    if name == "search_path" {
        SearchPath::parse(value).map_err(|e| e.to_string())?;
    } else if name == "application_name" || is_custom(name) {
        // These values have no execution-side effects.
    } else if name == "extra_float_digits" {
        if !value.parse::<i32>().is_ok_and(|v| (-15..=3).contains(&v)) {
            return Err("extra_float_digits must be between -15 and 3".into());
        }
    } else if !value.eq_ignore_ascii_case(default.unwrap_or("")) {
        // Do not pretend to enforce timeouts, encodings, isolation, or formats.
        return Err(format!("unsupported value for configuration parameter \"{name}\": {value}"));
    }
    Ok(value.to_string())
}
