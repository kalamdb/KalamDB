//! Primary key validation for MVCC INSERT operations
//!
//! This module implements `validate_primary_key()` and `extract_user_pk_value()`
//! using the internal `Row` representation (`BTreeMap<String, ScalarValue>`).

use kalamdb_commons::{models::rows::Row, scalar_to_pk_string};

use crate::error::KalamDbError;

/// Extract primary key value from a `Row`
pub fn extract_user_pk_value(fields: &Row, pk_column: &str) -> Result<String, KalamDbError> {
    let pk_value = fields.get(pk_column).ok_or_else(|| {
        KalamDbError::InvalidOperation(format!(
            "Primary key column '{}' not found in fields",
            pk_column
        ))
    })?;

    scalar_to_pk_string(pk_value).map_err(|_| {
        KalamDbError::InvalidOperation(format!(
            "Primary key column '{}' has unsupported or NULL value",
            pk_column
        ))
    })
}

/// Validate primary key constraints on an INSERT payload
pub fn validate_primary_key(
    fields: &Row,
    pk_column: &str,
    existing_pk_values: &std::collections::HashSet<String>,
) -> Result<(), KalamDbError> {
    let pk_value = extract_user_pk_value(fields, pk_column)?;

    if existing_pk_values.contains(&pk_value) {
        return Err(KalamDbError::AlreadyExists(format!(
            "Primary key violation: value '{}' already exists",
            pk_value
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use datafusion::scalar::ScalarValue;

    use super::*;

    #[test]
    fn test_extract_user_pk_value_string() {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Utf8(Some("user123".to_string())));
        let row = Row::new(values);
        let pk = extract_user_pk_value(&row, "id").unwrap();
        assert_eq!(pk, "user123");
    }

    #[test]
    fn test_extract_user_pk_value_number() {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(42)));
        let row = Row::new(values);
        let pk = extract_user_pk_value(&row, "id").unwrap();
        assert_eq!(pk, "42");
    }

    #[test]
    fn test_extract_user_pk_value_missing() {
        let mut values = BTreeMap::new();
        values.insert("name".to_string(), ScalarValue::Utf8(Some("Alice".to_string())));
        let row = Row::new(values);
        let result = extract_user_pk_value(&row, "id");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_extract_user_pk_value_null() {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(None));
        let row = Row::new(values);
        let result = extract_user_pk_value(&row, "id");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unsupported or NULL"));
    }

    #[test]
    fn test_validate_primary_key_success() {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Utf8(Some("new_user".to_string())));
        let row = Row::new(values);
        let existing = HashSet::from(["user1".to_string(), "user2".to_string()]);

        let result = validate_primary_key(&row, "id", &existing);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_primary_key_duplicate() {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Utf8(Some("user1".to_string())));
        let row = Row::new(values);
        let existing = HashSet::from(["user1".to_string(), "user2".to_string()]);

        let result = validate_primary_key(&row, "id", &existing);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_validate_primary_key_missing_column() {
        let mut values = BTreeMap::new();
        values.insert("name".to_string(), ScalarValue::Utf8(Some("Alice".to_string())));
        let row = Row::new(values);
        let existing = HashSet::new();

        let result = validate_primary_key(&row, "id", &existing);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
