//! Type-safe SQL query parameters for `$1`, `$2`, … placeholders.

use serde_json::{Number, Value as JsonValue};

use crate::models::FileRef;

/// A single bound SQL parameter.
///
/// Prefer this over raw [`serde_json::Value`] so common KalamDB types — including
/// [`FileRef`] — serialize correctly without manual JSON construction.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryParam {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    /// Existing FILE column reference (serialized as JSON for SQL binding).
    File(FileRef),
    /// Advanced JSON values when no typed variant fits.
    Json(JsonValue),
}

impl QueryParam {
    /// Convert to the JSON wire value expected by `/v1/api/sql`.
    pub fn to_json(&self) -> JsonValue {
        match self {
            Self::Null => JsonValue::Null,
            Self::Bool(value) => JsonValue::Bool(*value),
            Self::Int(value) => JsonValue::Number(Number::from(*value)),
            Self::Float(value) => {
                Number::from_f64(*value).map(JsonValue::Number).unwrap_or(JsonValue::Null)
            },
            Self::Text(value) => JsonValue::String(value.clone()),
            Self::File(file_ref) => JsonValue::String(file_ref.to_json()),
            Self::Json(value) => value.clone(),
        }
    }
}

impl From<()> for QueryParam {
    fn from((): ()) -> Self {
        Self::Null
    }
}

impl From<bool> for QueryParam {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i16> for QueryParam {
    fn from(value: i16) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<i32> for QueryParam {
    fn from(value: i32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<i64> for QueryParam {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<u64> for QueryParam {
    fn from(value: u64) -> Self {
        Self::Int(value as i64)
    }
}

impl From<f32> for QueryParam {
    fn from(value: f32) -> Self {
        Self::Float(f64::from(value))
    }
}

impl From<f64> for QueryParam {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<String> for QueryParam {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for QueryParam {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<FileRef> for QueryParam {
    fn from(value: FileRef) -> Self {
        Self::File(value)
    }
}

impl From<&FileRef> for QueryParam {
    fn from(value: &FileRef) -> Self {
        Self::File(value.clone())
    }
}

impl From<JsonValue> for QueryParam {
    fn from(value: JsonValue) -> Self {
        Self::Json(value)
    }
}

impl From<QueryParam> for JsonValue {
    fn from(value: QueryParam) -> Self {
        value.to_json()
    }
}

pub(crate) fn params_to_json(params: Option<Vec<QueryParam>>) -> Option<Vec<JsonValue>> {
    params.map(|items| items.into_iter().map(|param| param.to_json()).collect())
}

/// Convert a JSON parameter value into a typed [`QueryParam`].
///
/// Detects [`FileRef`] objects (and JSON strings containing file references)
/// so FILE column bindings do not require manual serialization.
pub fn from_json_value(value: JsonValue) -> QueryParam {
    match value {
        JsonValue::Null => QueryParam::Null,
        JsonValue::Bool(value) => QueryParam::Bool(value),
        JsonValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                QueryParam::Int(value)
            } else if let Some(value) = number.as_f64() {
                QueryParam::Float(value)
            } else {
                QueryParam::Json(JsonValue::Number(number))
            }
        },
        JsonValue::String(text) => {
            if let Some(file_ref) = FileRef::from_json(&text) {
                QueryParam::File(file_ref)
            } else {
                QueryParam::Text(text)
            }
        },
        JsonValue::Object(_) => FileRef::from_json_value(&value)
            .map(QueryParam::File)
            .unwrap_or(QueryParam::Json(value)),
        JsonValue::Array(_) => QueryParam::Json(value),
    }
}

/// Convert a JSON array of parameter values into typed [`QueryParam`] items.
pub fn params_from_json_values(values: Vec<JsonValue>) -> Vec<QueryParam> {
    values.into_iter().map(from_json_value).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_param_serializes_as_string() {
        let param = QueryParam::from("doc1");
        assert_eq!(param.to_json(), JsonValue::String("doc1".into()));
    }

    #[test]
    fn file_param_serializes_as_json_string() {
        let file_ref = FileRef {
            id:     "1".into(),
            sub:    "f0001".into(),
            name:   "a.png".into(),
            size:   10,
            mime:   "image/png".into(),
            sha256: "abc".into(),
            shard:  None,
        };
        let json = QueryParam::from(file_ref).to_json();
        assert!(json.is_string());
        assert!(json.as_str().unwrap().contains("\"id\":\"1\""));
    }

    #[test]
    fn from_json_value_detects_file_object() {
        let value = serde_json::json!({
            "id": "1",
            "sub": "f0001",
            "name": "a.png",
            "size": 10,
            "mime": "image/png",
            "sha256": "abc"
        });
        match from_json_value(value) {
            QueryParam::File(file_ref) => assert_eq!(file_ref.id, "1"),
            other => panic!("expected File param, got {other:?}"),
        }
    }
}
