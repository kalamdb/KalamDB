//! Catalog-derived row, key, insert, and patch contracts.
//!
//! Keys come from primary-key metadata. A column named `id` is not a key
//! unless the catalog marks it as one.

use super::{
    snapshot::{ContractField, ContractTable},
    ContractError,
};

/// One catalog column as seen by generated row, insert, and patch models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnContract {
    pub name:                String,
    pub type_name:           String,
    pub not_null:            bool,
    pub has_default:         bool,
    pub is_identity:         bool,
    pub is_generated:        bool,
    pub is_read_only:        bool,
    pub primary_key_ordinal: Option<u16>,
}

/// Immutable table access surface derived from [`ContractTable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRowContract {
    pub table_id:        String,
    pub schema:          String,
    pub name:            String,
    pub key_addressable: bool,
    pub key_columns:     Vec<ColumnContract>,
    pub row_columns:     Vec<ColumnContract>,
    pub insert_columns:  Vec<ColumnContract>,
    pub required_insert: Vec<String>,
    pub patch_columns:   Vec<ColumnContract>,
}

/// Which write shape a column list is being checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteShape {
    Insert,
    Patch,
}

/// Build the row, key, insert, and patch models for one catalog table.
pub fn table_row_contract(table: &ContractTable) -> Result<TableRowContract, ContractError> {
    let key_fields = table.ordered_key_fields();
    if table.primary_key.len() != key_fields.len()
        || table
            .primary_key
            .iter()
            .zip(key_fields.iter())
            .any(|(declared, field)| !declared.eq_ignore_ascii_case(&field.name))
    {
        return Err(ContractError::new(
            "primary key list does not match primary-key column metadata",
        ));
    }

    let key_columns = key_fields.iter().copied().map(column_contract).collect::<Vec<_>>();
    Ok(TableRowContract {
        table_id: table.table_id.clone(),
        schema: table.schema.clone(),
        name: table.name.clone(),
        key_addressable: !key_columns.is_empty(),
        key_columns,
        row_columns: table
            .fields
            .iter()
            .filter(|field| !field.dropped)
            .map(column_contract)
            .collect(),
        insert_columns: table.writable_insert_fields().into_iter().map(column_contract).collect(),
        required_insert: table
            .required_insert_fields()
            .into_iter()
            .map(|field| field.name.clone())
            .collect(),
        patch_columns: table.patchable_fields().into_iter().map(column_contract).collect(),
    })
}

/// Reject unknown columns and writes the catalog marks read-only or generated.
///
/// Omitted patch fields are not writes and are not nulls. Callers pass only
/// the columns present in the payload.
pub fn reject_forbidden_write(
    table: &ContractTable,
    shape: WriteShape,
    columns: &[&str],
) -> Result<(), ContractError> {
    let mut seen = Vec::with_capacity(columns.len());
    for column in columns {
        if seen.iter().any(|existing: &&str| existing.eq_ignore_ascii_case(column)) {
            return Err(ContractError::new(format!("duplicate write column {column}")));
        }
        seen.push(*column);
        let Some(field) = table
            .fields
            .iter()
            .find(|field| !field.dropped && field.name.eq_ignore_ascii_case(column))
        else {
            return Err(ContractError::new(format!("unknown column {column}")));
        };
        let forbidden = match shape {
            WriteShape::Insert => field.is_read_only || field.is_generated,
            WriteShape::Patch => field.primary_key || field.is_read_only || field.is_generated,
        };
        if forbidden {
            return Err(ContractError::new(format!("column {column} is not writable")));
        }
    }
    Ok(())
}

fn column_contract(field: &ContractField) -> ColumnContract {
    ColumnContract {
        name:                field.name.clone(),
        type_name:           field.type_name.clone(),
        not_null:            field.not_null,
        has_default:         field.has_default,
        is_identity:         field.is_identity,
        is_generated:        field.is_generated,
        is_read_only:        field.is_read_only,
        primary_key_ordinal: field.primary_key_ordinal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{
        compile_contract_sql,
        snapshot::{ContractField, ContractTable},
    };

    #[test]
    fn t017_catalog_models_keep_real_keys_and_reject_forbidden_writes() {
        let snapshot = compile_contract_sql(
            r#"
            CREATE SCHEMA school;
            CREATE TABLE school.enrollments (
              enrollment_id TEXT PRIMARY KEY,
              id TEXT,
              seq_no BIGINT GENERATED ALWAYS AS IDENTITY,
              display_code TEXT GENERATED ALWAYS AS (enrollment_id) STORED,
              role TEXT NOT NULL DEFAULT 'student',
              tuition NUMERIC(12, 2) NOT NULL DEFAULT 0.00
            ) WITH (TYPE = 'SHARED');
            "#,
            "public",
        )
        .unwrap();
        let table = &snapshot.tables["school.enrollments"];
        let contract = table_row_contract(table).unwrap();

        assert!(contract.key_addressable);
        assert_eq!(
            contract
                .key_columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            vec!["enrollment_id"]
        );
        assert!(!contract.key_columns.iter().any(|column| column.name == "id"));
        assert_eq!(contract.required_insert, vec!["enrollment_id".to_string()]);
        assert_eq!(
            contract
                .patch_columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            vec!["id", "role", "tuition"]
        );
        assert!(contract
            .insert_columns
            .iter()
            .all(|column| { column.name != "display_code" && column.name != "seq_no" }));

        assert!(reject_forbidden_write(table, WriteShape::Insert, &["role"]).is_ok());
        assert!(reject_forbidden_write(table, WriteShape::Insert, &["display_code"]).is_err());
        assert!(reject_forbidden_write(table, WriteShape::Patch, &["enrollment_id"]).is_err());
        assert!(reject_forbidden_write(table, WriteShape::Patch, &["missing"]).is_err());
    }

    #[test]
    fn t017_disagrees_when_declared_key_does_not_match_column_metadata() {
        let mut table = ContractTable {
            table_id: "school.tickets".to_string(),
            schema: "school".to_string(),
            name: "tickets".to_string(),
            ..ContractTable::default()
        };
        table.primary_key = vec!["id".to_string()];
        table.fields.push(ContractField {
            name: "id".to_string(),
            type_name: "text".to_string(),
            primary_key: false,
            ..ContractField::default()
        });
        let error = table_row_contract(&table).unwrap_err();
        assert!(error.message.contains("does not match"));
    }
}
