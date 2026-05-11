use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::{row_to_serde_model, serde_model_to_row},
    models::{
        datatypes::KalamDataType,
        rows::{Row, SystemTableRow},
    },
    schemas::TableDefinition,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::error::SystemError;

pub fn model_to_system_row<T: Serialize>(
    model: &T,
    table_def: &TableDefinition,
) -> Result<SystemTableRow, SystemError> {
    let mut fields =
        serde_model_to_row(model, table_def).map_err(SystemError::SerializationError)?;
    normalize_model_timestamp_ms_to_storage_micros(&mut fields, table_def)
        .map_err(SystemError::SerializationError)?;
    Ok(SystemTableRow { fields })
}

pub fn system_row_to_model<T: DeserializeOwned>(
    row: &SystemTableRow,
    table_def: &TableDefinition,
) -> Result<T, SystemError> {
    let mut fields = row.fields.clone();
    normalize_storage_timestamp_micros_to_model_ms(&mut fields, table_def)
        .map_err(SystemError::SerializationError)?;
    row_to_serde_model(&fields, table_def).map_err(SystemError::SerializationError)
}

fn normalize_model_timestamp_ms_to_storage_micros(
    row: &mut Row,
    table_def: &TableDefinition,
) -> Result<(), String> {
    for column in &table_def.columns {
        if !matches!(column.data_type, KalamDataType::Timestamp | KalamDataType::DateTime) {
            continue;
        }

        let Some(value) = row.values.get_mut(&column.column_name) else {
            continue;
        };

        let current = std::mem::replace(value, ScalarValue::Null);
        *value = match current {
            ScalarValue::Int64(Some(ms)) => {
                ScalarValue::TimestampMicrosecond(Some(ms_to_micros(ms)?), None)
            },
            ScalarValue::TimestampMicrosecond(Some(ms), timezone) => {
                ScalarValue::TimestampMicrosecond(Some(ms_to_micros(ms)?), timezone)
            },
            ScalarValue::TimestampMillisecond(Some(ms), timezone) => {
                ScalarValue::TimestampMicrosecond(Some(ms_to_micros(ms)?), timezone)
            },
            other => other,
        };
    }

    Ok(())
}

fn normalize_storage_timestamp_micros_to_model_ms(
    row: &mut Row,
    table_def: &TableDefinition,
) -> Result<(), String> {
    for column in &table_def.columns {
        if !matches!(column.data_type, KalamDataType::Timestamp | KalamDataType::DateTime) {
            continue;
        }

        let Some(value) = row.values.get_mut(&column.column_name) else {
            continue;
        };

        let current = std::mem::replace(value, ScalarValue::Null);
        *value = match current {
            ScalarValue::TimestampSecond(Some(seconds), _) => {
                ScalarValue::Int64(Some(seconds_to_millis(seconds)?))
            },
            ScalarValue::TimestampMillisecond(Some(ms), _) => ScalarValue::Int64(Some(ms)),
            ScalarValue::TimestampMicrosecond(Some(micros), _) => {
                ScalarValue::Int64(Some(micros / 1_000))
            },
            ScalarValue::TimestampNanosecond(Some(nanos), _) => {
                ScalarValue::Int64(Some(nanos / 1_000_000))
            },
            other => other,
        };
    }

    Ok(())
}

fn ms_to_micros(value: i64) -> Result<i64, String> {
    value
        .checked_mul(1_000)
        .ok_or_else(|| format!("timestamp value {value} overflows when converted to microseconds"))
}

fn seconds_to_millis(value: i64) -> Result<i64, String> {
    value
        .checked_mul(1_000)
        .ok_or_else(|| format!("timestamp value {value} overflows when converted to milliseconds"))
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        models::rows::SystemTableRow, schemas::TableDefinition, NamespaceId, TableName,
    };
    use serde::{Deserialize, Serialize};

    use super::{model_to_system_row, system_row_to_model};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestModel {
        id: String,
        name: String,
        created_at: i64,
        routes: Vec<String>,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct UuidDecimalModel {
        id: String,
        user_uuid: String,
        amount: String,
    }

    #[test]
    fn test_system_row_model_roundtrip() {
        let model = TestModel {
            id: "a1".to_string(),
            name: "hello".to_string(),
            created_at: 12345,
            routes: vec!["x".to_string(), "y".to_string()],
        };

        let table_def = TableDefinition::new(
            NamespaceId::system(),
            TableName::new("test_models"),
            kalamdb_commons::schemas::TableType::System,
            vec![
                kalamdb_commons::schemas::ColumnDefinition::primary_key(
                    1,
                    "id",
                    1,
                    kalamdb_commons::datatypes::KalamDataType::Text,
                ),
                kalamdb_commons::schemas::ColumnDefinition::simple(
                    2,
                    "name",
                    2,
                    kalamdb_commons::datatypes::KalamDataType::Text,
                ),
                kalamdb_commons::schemas::ColumnDefinition::simple(
                    3,
                    "created_at",
                    3,
                    kalamdb_commons::datatypes::KalamDataType::Timestamp,
                ),
                kalamdb_commons::schemas::ColumnDefinition::simple(
                    4,
                    "routes",
                    4,
                    kalamdb_commons::datatypes::KalamDataType::Json,
                ),
            ],
            kalamdb_commons::schemas::TableOptions::system(),
            None,
        )
        .expect("table definition");

        let row: SystemTableRow = model_to_system_row(&model, &table_def).expect("to row");
        assert!(matches!(
            row.fields.values.get("created_at"),
            Some(datafusion::scalar::ScalarValue::TimestampMicrosecond(Some(12_345_000), None))
        ));

        let decoded: TestModel = system_row_to_model(&row, &table_def).expect("from row");
        assert_eq!(decoded, model);
    }

    #[test]
    fn test_system_row_uuid_and_decimal_roundtrip() {
        let model = UuidDecimalModel {
            id: "r1".to_string(),
            user_uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            amount: "200.75".to_string(),
        };

        let table_def = TableDefinition::new(
            NamespaceId::system(),
            TableName::new("uuid_decimal_models"),
            kalamdb_commons::schemas::TableType::System,
            vec![
                kalamdb_commons::schemas::ColumnDefinition::primary_key(
                    1,
                    "id",
                    1,
                    kalamdb_commons::datatypes::KalamDataType::Text,
                ),
                kalamdb_commons::schemas::ColumnDefinition::simple(
                    2,
                    "user_uuid",
                    2,
                    kalamdb_commons::datatypes::KalamDataType::Uuid,
                ),
                kalamdb_commons::schemas::ColumnDefinition::simple(
                    3,
                    "amount",
                    3,
                    kalamdb_commons::datatypes::KalamDataType::Decimal {
                        precision: 10,
                        scale: 2,
                    },
                ),
            ],
            kalamdb_commons::schemas::TableOptions::system(),
            None,
        )
        .expect("table definition");

        let row: SystemTableRow = model_to_system_row(&model, &table_def).expect("to row");
        let decoded: UuidDecimalModel = system_row_to_model(&row, &table_def).expect("from row");

        assert_eq!(decoded.id, model.id);
        assert_eq!(decoded.user_uuid, model.user_uuid);
        assert_eq!(decoded.amount, model.amount);
    }
}
