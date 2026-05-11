use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::{row_to_serde_model, serde_model_to_row},
    models::{datatypes::KalamDataType, rows::Row},
    schemas::TableDefinition,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::error::{DbaError, Result};

pub fn model_to_row<T: Serialize>(model: &T, table_def: &TableDefinition) -> Result<Row> {
    let mut row = serde_model_to_row(model, table_def).map_err(DbaError::Serialization)?;
    normalize_model_timestamp_ms_to_storage_micros(&mut row, table_def)
        .map_err(DbaError::Serialization)?;
    Ok(row)
}

pub fn row_to_model<T: DeserializeOwned>(row: &Row, table_def: &TableDefinition) -> Result<T> {
    let mut row = row.clone();
    normalize_storage_timestamp_micros_to_model_ms(&mut row, table_def)
        .map_err(DbaError::Serialization)?;
    row_to_serde_model(&row, table_def).map_err(DbaError::Serialization)
}

fn normalize_model_timestamp_ms_to_storage_micros(
    row: &mut Row,
    table_def: &TableDefinition,
) -> std::result::Result<(), String> {
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
) -> std::result::Result<(), String> {
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

fn ms_to_micros(value: i64) -> std::result::Result<i64, String> {
    value
        .checked_mul(1_000)
        .ok_or_else(|| format!("timestamp value {value} overflows when converted to microseconds"))
}

fn seconds_to_millis(value: i64) -> std::result::Result<i64, String> {
    value
        .checked_mul(1_000)
        .ok_or_else(|| format!("timestamp value {value} overflows when converted to milliseconds"))
}

#[cfg(test)]
mod tests {
    use datafusion::scalar::ScalarValue;

    use super::{model_to_row, row_to_model};
    use crate::models::StatsRow;

    #[test]
    fn dba_timestamp_models_store_microseconds_and_decode_milliseconds() {
        let row = StatsRow {
            id: "1700000000000:node:metric".to_string(),
            node_id: "node".to_string(),
            metric_name: "metric".to_string(),
            metric_value: 42.0,
            metric_unit: None,
            sampled_at: 1_700_000_000_000,
        };

        let encoded = model_to_row(&row, &StatsRow::definition()).expect("encode stats row");
        assert!(matches!(
            encoded.values.get("sampled_at"),
            Some(ScalarValue::TimestampMicrosecond(Some(1_700_000_000_000_000), None))
        ));

        let decoded: StatsRow =
            row_to_model(&encoded, &StatsRow::definition()).expect("decode row");
        assert_eq!(decoded.sampled_at, row.sampled_at);
    }
}
