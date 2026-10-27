//! Ordinal nested row codec.

mod array;
mod batch;
mod decode;
mod encode;
mod fixed_value_encoding;
mod from_table;
mod metadata;
mod scalar;
mod schema;
mod value;

pub use batch::{
    decode_payloads_to_arrow_batch, MAX_BATCH_BYTES, MAX_BATCH_ROWS, MAX_LIST_LEN, MAX_NESTING,
};
pub use decode::{
    decode_row_fields, decode_row_fields_columns, decode_shared_row, decode_shared_row_selected,
    decode_stream_row, decode_user_row, decode_user_row_selected, open_v2_row, project_row_fields,
    RowView,
};
pub use encode::{
    encode_row_envelope, encode_row_fields, encode_row_fields_from_columns, encode_shared_row,
    encode_shared_row_v2, encode_stream_row, encode_tombstone_row_v2, encode_user_row,
    encode_user_row_from_columns, encode_user_row_v2,
};
pub use from_table::{
    storage_data_type_from_arrow, storage_data_type_from_kalam, storage_schema_from_table,
    storage_schema_from_table_versions, storage_schema_from_table_versions_with_arrow,
    storage_schema_from_table_with_arrow,
};
pub use metadata::{decode_row_metadata, RowMetadata};
pub use schema::{StorageDataType, StorageField, StorageSchema, StorageVersionLayout};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::{
        array::{Int64Array, ListArray, StringArray, StructArray},
        buffer::OffsetBuffer,
        datatypes::{DataType, Field},
    };
    use datafusion_common::ScalarValue;
    use kalamdb_commons::{
        ids::VersionId,
        models::{
            rows::{Row, SharedTableRow, StreamTableRow, UserTableRow},
            UserId,
        },
    };

    use super::*;
    use crate::error::SerializationError;

    fn customer_type() -> StorageDataType {
        StorageDataType::Struct(vec![
            StorageField::new("id", StorageDataType::Int64),
            StorageField::new("name", StorageDataType::Utf8),
        ])
    }

    fn orders_schema() -> StorageSchema {
        StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("customer", customer_type()),
                StorageField::new("tags", StorageDataType::List(Box::new(StorageDataType::Utf8))),
            ],
        )
    }

    fn struct_scalar(id: i64, name: &str) -> ScalarValue {
        let struct_array = StructArray::from(vec![
            (
                Arc::new(Field::new("id", DataType::Int64, true)),
                Arc::new(Int64Array::from(vec![Some(id)])) as arrow::array::ArrayRef,
            ),
            (
                Arc::new(Field::new("name", DataType::Utf8, true)),
                Arc::new(StringArray::from(vec![Some(name)])) as arrow::array::ArrayRef,
            ),
        ]);
        ScalarValue::Struct(Arc::new(struct_array))
    }

    fn utf8_list(values: &[&str]) -> ScalarValue {
        let items: Vec<ScalarValue> = values
            .iter()
            .map(|value| ScalarValue::Utf8(Some((*value).to_string())))
            .collect();
        if items.is_empty() {
            let field = Arc::new(Field::new("item", DataType::Utf8, true));
            let list = ListArray::try_new(
                field,
                OffsetBuffer::from_lengths([0]),
                Arc::new(StringArray::from(Vec::<String>::new())),
                None,
            )
            .unwrap();
            return ScalarValue::List(Arc::new(list));
        }
        ScalarValue::List(ScalarValue::new_list(&items, &DataType::Utf8, true))
    }

    fn sample_row() -> UserTableRow {
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(9)));
        values.insert("customer".to_string(), struct_scalar(7, "ada"));
        values.insert("tags".to_string(), utf8_list(&["vip", "west"]));
        UserTableRow {
            user_id: UserId::new("user-1"),
            _version: VersionId::try_from_i64(100).unwrap(),
            _deleted: false,
            fields: Row { values },
        }
    }

    #[test]
    fn nested_struct_and_list_roundtrip_without_string_fallback() {
        let schema = orders_schema();
        let row = sample_row();
        let encoded = encode_user_row(&row, &schema).unwrap();
        assert_eq!(&encoded.as_slice()[0..4], b"KOBJ");
        let raw = encoded.as_slice();
        assert!(
            !raw.windows(b"{\"name\"".len()).any(|w| w == b"{\"name\""),
            "nested struct must not be JSON-encoded"
        );
        let decoded =
            decode_user_row(encoded.as_slice(), &schema, row.user_id.clone(), row._version)
                .unwrap();
        assert_eq!(decoded.user_id, row.user_id);
        assert_eq!(decoded._version, row._version);
        assert_eq!(decoded.fields.values.get("id"), row.fields.values.get("id"));
        assert_eq!(decoded.fields.values.get("customer"), row.fields.values.get("customer"));
        assert_eq!(decoded.fields.values.get("tags"), row.fields.values.get("tags"));
    }

    #[test]
    fn raft_field_payload_roundtrips_without_flexbuffers_names() {
        let schema = orders_schema();
        let row = sample_row();
        let encoded = encode_row_fields(&row.fields, &schema).unwrap();
        assert!(
            !encoded.windows(b"customer".len()).any(|w| w == b"customer"),
            "ordinal field payload must not store column names"
        );
        let decoded = decode_row_fields(&encoded, &schema).unwrap();
        assert_eq!(decoded.values.get("id"), row.fields.values.get("id"));
        assert_eq!(decoded.values.get("customer"), row.fields.values.get("customer"));
        assert_eq!(decoded.values.get("tags"), row.fields.values.get("tags"));
    }

    #[test]
    fn additive_nullable_struct_field_decodes_as_null() {
        let old_schema =
            StorageSchema::new(1, vec![StorageField::new("customer", customer_type())]);
        let mut values = std::collections::BTreeMap::new();
        values.insert("customer".to_string(), struct_scalar(1, "lin"));
        let row = UserTableRow {
            user_id: UserId::new("user-2"),
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row { values },
        };
        let encoded = encode_user_row(&row, &old_schema).unwrap();

        let mut wider_fields = match customer_type() {
            StorageDataType::Struct(fields) => fields,
            _ => unreachable!(),
        };
        wider_fields.push(StorageField::new("username", StorageDataType::Utf8));
        let new_schema = StorageSchema::new(
            1,
            vec![StorageField::new(
                "customer",
                StorageDataType::Struct(wider_fields),
            )],
        );
        let decoded =
            decode_user_row(encoded.as_slice(), &new_schema, row.user_id.clone(), row._version)
                .unwrap();
        let ScalarValue::Struct(struct_array) = decoded.fields.values.get("customer").unwrap()
        else {
            panic!("expected struct");
        };
        assert_eq!(struct_array.num_columns(), 3);
        assert!(struct_array.column(2).is_null(0));
    }

    #[test]
    fn metadata_decode_skips_nested_columns() {
        let encoded = encode_user_row(&sample_row(), &orders_schema()).unwrap();
        let raw = encoded.as_slice();
        assert!(
            !raw.windows(b"user-1".len()).any(|w| w == b"user-1"),
            "user_id must not be stored in the row value"
        );
        let meta = decode_row_metadata(encoded.as_slice()).unwrap();
        assert!(!meta.deleted);
    }

    #[test]
    fn embedding_from_sliced_parquet_row_roundtrips() {
        let schema = StorageSchema::new(
            1,
            vec![StorageField::new(
                "vec",
                StorageDataType::Embedding { dimension: 3 },
            )],
        );
        let child = Arc::new(Field::new("item", DataType::Float32, true));
        let values = arrow::array::Float32Array::from(vec![9.0, 8.0, 7.0, 1.0, 0.0, 0.0]);
        let list = arrow::array::FixedSizeListArray::new(child, 3, Arc::new(values), None);
        let scalar = ScalarValue::try_from_array(&list, 1).unwrap();

        let mut fields = std::collections::BTreeMap::new();
        fields.insert("vec".to_string(), scalar);
        let row = UserTableRow {
            user_id: UserId::new("user-emb"),
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: true,
            fields: Row { values: fields },
        };

        let encoded = encode_user_row(&row, &schema).unwrap();
        let decoded =
            decode_user_row(encoded.as_slice(), &schema, row.user_id.clone(), row._version)
                .unwrap();
        let decoded_vec = decoded.fields.values.get("vec").expect("vec");
        assert!(matches!(decoded_vec, ScalarValue::FixedSizeList(_)));
    }

    #[test]
    fn unsupported_value_does_not_use_string_fallback() {
        let schema = StorageSchema::new(1, vec![StorageField::new("id", StorageDataType::Int64)]);
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::IntervalYearMonth(Some(1)));
        let row = UserTableRow {
            user_id: UserId::new("user-3"),
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row { values },
        };
        let err = encode_user_row(&row, &schema).unwrap_err();
        assert!(matches!(err, SerializationError::Encode(_)));
        assert!(!err.to_string().contains("fallback"));
    }

    #[test]
    fn shared_and_stream_rows_roundtrip_without_identity() {
        let schema = StorageSchema::new(1, vec![StorageField::new("id", StorageDataType::Int64)]);
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(42)));
        let fields = Row { values };

        let version = VersionId::try_from_i64(77).unwrap();
        let shared = SharedTableRow {
            _version: version,
            _deleted: true,
            fields: fields.clone(),
        };
        let encoded = encode_shared_row(&shared, &schema).unwrap();
        let decoded = decode_shared_row(encoded.as_slice(), &schema, version).unwrap();
        assert_eq!(decoded._version, version);
        assert!(decoded._deleted);
        assert_eq!(decoded.fields.values.get("id"), fields.values.get("id"));

        let selected =
            decode_shared_row_selected(encoded.as_slice(), &schema, version, &[0]).unwrap();
        assert_eq!(selected._version, version);
        assert!(selected._deleted);
        assert_eq!(selected.fields.values.get("id"), fields.values.get("id"));

        let stream = StreamTableRow {
            user_id: UserId::new("owner"),
            _version: VersionId::try_from_i64(5).unwrap(),
            _timestamp: 1_700_000_000_000,
            fields: fields.clone(),
        };
        let encoded = encode_stream_row(&stream, &schema).unwrap();
        let decoded = decode_stream_row(
            encoded.as_slice(),
            &schema,
            UserId::new("owner"),
            VersionId::try_from_i64(5).unwrap(),
            stream._timestamp,
        )
        .unwrap();
        assert_eq!(decoded.user_id.as_str(), "owner");
        assert_eq!(decoded._version, VersionId::try_from_i64(5).unwrap());
        assert_eq!(decoded._timestamp, stream._timestamp);
        assert_eq!(decoded.fields.values.get("id"), fields.values.get("id"));
    }

    #[test]
    fn extra_stored_ordinals_are_skipped() {
        let wide = StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("gone", StorageDataType::Utf8),
            ],
        );
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(1)));
        values.insert("gone".to_string(), ScalarValue::Utf8(Some("drop-me".to_string())));
        let row = UserTableRow {
            user_id: UserId::new("user-x"),
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row { values },
        };
        let encoded = encode_user_row(&row, &wide).unwrap();
        let narrow = StorageSchema::new(1, vec![StorageField::new("id", StorageDataType::Int64)]);
        let decoded = decode_user_row(
            encoded.as_slice(),
            &narrow,
            UserId::new("user-x"),
            VersionId::try_from_i64(1).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded.fields.values.get("id"), Some(&ScalarValue::Int64(Some(1))));
        assert!(!decoded.fields.values.contains_key("gone"));
    }

    #[test]
    fn stored_int32_decodes_after_alter_to_int64() {
        let int32_schema = StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("quantity", StorageDataType::Int32),
            ],
        );
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(2001)));
        values.insert("quantity".to_string(), ScalarValue::Int32(Some(5)));
        let row = UserTableRow {
            user_id: UserId::new("user-x"),
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row { values },
        };
        let encoded = encode_user_row(&row, &int32_schema).unwrap();
        let int64_schema = StorageSchema::new(
            2,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("quantity", StorageDataType::Int64),
            ],
        );
        let decoded = decode_user_row(
            encoded.as_slice(),
            &int64_schema,
            UserId::new("user-x"),
            VersionId::try_from_i64(1).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded.fields.values.get("quantity"), Some(&ScalarValue::Int64(Some(5))));
    }

    #[test]
    fn new_rows_set_column_offset_flag() {
        let encoded = encode_user_row(&sample_row(), &orders_schema()).unwrap();
        let flags = u16::from_le_bytes([encoded.as_slice()[9], encoded.as_slice()[10]]);
        assert_eq!(flags, crate::object::FLAG_COLUMN_OFFSETS | crate::object::FLAG_VERSION_IN_KEY);
    }

    #[test]
    fn legacy_commit_seq_header_is_rejected() {
        let schema = orders_schema();
        let row = sample_row();
        let mut payload = Vec::new();
        payload.extend_from_slice(&schema.version.to_le_bytes());
        payload.extend_from_slice(&3u64.to_le_bytes());
        payload.push(u8::from(row._deleted));
        payload.extend_from_slice(&encode_row_fields(&row.fields, &schema).unwrap());
        let encoded = crate::object::encode_envelope(
            crate::object::ObjectKind::Row,
            schema.version,
            &payload,
        )
        .unwrap();
        let err = decode_user_row(encoded.as_slice(), &schema, row.user_id.clone(), row._version)
            .unwrap_err();
        assert!(err.to_string().contains("legacy commit sequence"));
    }

    #[test]
    fn selected_decode_skips_unread_nested_columns() {
        let schema = orders_schema();
        let row = sample_row();
        let encoded = encode_user_row(&row, &schema).unwrap();
        let decoded = decode_user_row_selected(
            encoded.as_slice(),
            &schema,
            row.user_id.clone(),
            row._version,
            &[0, 2],
        )
        .unwrap();
        assert_eq!(decoded.fields.values.get("id"), row.fields.values.get("id"));
        assert_eq!(decoded.fields.values.get("tags"), row.fields.values.get("tags"));
        assert!(!decoded.fields.values.contains_key("customer"));
    }

    #[test]
    fn ordinal_column_encode_matches_named_row() {
        let schema = orders_schema();
        let row = sample_row();
        let columns = vec![
            row.fields.values.get("id").cloned().unwrap(),
            row.fields.values.get("customer").cloned().unwrap(),
            row.fields.values.get("tags").cloned().unwrap(),
        ];
        let from_map = encode_user_row(&row, &schema).unwrap();
        let from_cols = encode_user_row_from_columns(row._deleted, &columns, &schema).unwrap();
        assert_eq!(from_map.as_slice(), from_cols.as_slice());
        let envelope = kalamdb_commons::models::rows::RowEnvelope::new(columns);
        let from_envelope = encode_row_envelope(row._deleted, &envelope, &schema).unwrap();
        assert_eq!(from_map.as_slice(), from_envelope.as_slice());
    }

    #[test]
    fn row_body_v2_round_trips_and_zero_copy_view_accesses_fields() {
        let schema = StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("flag", StorageDataType::Boolean),
                StorageField::new("note", StorageDataType::Utf8),
                StorageField::new("customer", customer_type()),
            ],
        );
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(42)));
        values.insert("flag".to_string(), ScalarValue::Boolean(Some(true)));
        values.insert("note".to_string(), ScalarValue::Utf8(Some("hello v2".to_string())));
        values.insert("customer".to_string(), struct_scalar(99, "Ada"));
        let row = UserTableRow {
            user_id: UserId::new("u1"),
            _version: VersionId::try_from_i64(7).unwrap(),
            _deleted: false,
            fields: Row { values },
        };

        let encoded = encode_user_row_v2(&row, &schema).unwrap();
        let (header, payload) =
            crate::object::decode_envelope(encoded.as_slice(), crate::object::ObjectKind::Row)
                .unwrap();
        assert_ne!(header.flags & crate::object::FLAG_ROW_BODY_V2, 0);

        let view = RowView::parse(payload, &schema, header.flags).unwrap();
        assert!(!view.is_deleted());
        assert!(!view.is_null(0));
        assert_eq!(view.fixed_bytes(0, &schema).unwrap(), Some(42i64.to_le_bytes().as_slice()));
        assert_eq!(view.fixed_bytes(1, &schema).unwrap(), Some(&[1u8][..]));
        assert_eq!(
            view.decode_field(2, &schema).unwrap(),
            ScalarValue::Utf8(Some("hello v2".to_string()))
        );

        let decoded =
            decode_user_row(encoded.as_slice(), &schema, row.user_id.clone(), row._version)
                .unwrap();
        assert_eq!(decoded, row);

        let selected = decode_user_row_selected(
            encoded.as_slice(),
            &schema,
            row.user_id.clone(),
            row._version,
            &[0, 2],
        )
        .unwrap();
        assert_eq!(selected.fields.values.get("id"), Some(&ScalarValue::Int64(Some(42))));
        assert_eq!(
            selected.fields.values.get("note"),
            Some(&ScalarValue::Utf8(Some("hello v2".to_string())))
        );
        assert!(!selected.fields.values.contains_key("customer"));
    }

    #[test]
    fn row_body_v2_tombstone_only_and_mixed_v1_v2_batch_decode() {
        let v1_schema = StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("name", StorageDataType::Utf8),
            ],
        );
        let mut v2_schema = StorageSchema::new(
            2,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("name", StorageDataType::Utf8),
                StorageField::new("score", StorageDataType::Int64)
                    .with_missing_default(Some(ScalarValue::Int64(Some(100)))),
            ],
        );
        v2_schema.register_historical_version(1, &v1_schema.fields);

        let mut v1_values = std::collections::BTreeMap::new();
        v1_values.insert("id".to_string(), ScalarValue::Int64(Some(1)));
        v1_values.insert("name".to_string(), ScalarValue::Utf8(Some("legacy".to_string())));
        let v1_row = UserTableRow {
            user_id: UserId::new("u1"),
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row { values: v1_values },
        };
        let encoded_v1 = encode_user_row(&v1_row, &v1_schema).unwrap();

        let mut v2_values = std::collections::BTreeMap::new();
        v2_values.insert("id".to_string(), ScalarValue::Int64(Some(2)));
        v2_values.insert("name".to_string(), ScalarValue::Utf8(Some("modern".to_string())));
        v2_values.insert("score".to_string(), ScalarValue::Int64(Some(250)));
        let v2_row = UserTableRow {
            user_id: UserId::new("u1"),
            _version: VersionId::try_from_i64(2).unwrap(),
            _deleted: false,
            fields: Row { values: v2_values },
        };
        let encoded_v2 = encode_user_row_v2(&v2_row, &v2_schema).unwrap();

        let tombstone = encode_tombstone_row_v2(v2_schema.version).unwrap();
        assert!(decode_row_metadata(tombstone.as_slice()).unwrap().deleted);
        let decoded_tombstone = decode_user_row(
            tombstone.as_slice(),
            &v2_schema,
            UserId::new("u1"),
            VersionId::try_from_i64(3).unwrap(),
        )
        .unwrap();
        assert!(decoded_tombstone._deleted);
        assert_eq!(decoded_tombstone.fields.values.get("id"), Some(&ScalarValue::Int64(None)));

        let arrow_schema = Arc::new(arrow::datatypes::Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("name", DataType::Utf8, true),
            Field::new("score", DataType::Int64, true),
        ]));
        let batch = decode_payloads_to_arrow_batch(
            &v2_schema,
            arrow_schema,
            &[encoded_v1.as_slice(), encoded_v2.as_slice()],
        )
        .unwrap();
        assert_eq!(batch.num_rows(), 2);
        let scores = batch.column(2).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(scores.value(0), 100);
        assert_eq!(scores.value(1), 250);
    }

    #[test]
    fn unknown_row_envelope_flag_is_rejected() {
        let schema = orders_schema();
        let row = sample_row();
        let mut encoded = encode_user_row(&row, &schema).unwrap().into_bytes();
        // Set an unknown flag bit (0x8000) in the envelope flags at bytes 9..11.
        let flags = u16::from_le_bytes([encoded[9], encoded[10]]) | 0x8000;
        encoded[9..11].copy_from_slice(&flags.to_le_bytes());
        let err = decode_user_row(&encoded, &schema, row.user_id, row._version).unwrap_err();
        assert!(err.to_string().contains("unsupported envelope flags"));
    }

    #[test]
    fn row_body_v2_rejects_non_monotonic_offsets_and_trailing_bytes() {
        let schema = StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("a", StorageDataType::Utf8),
                StorageField::new("b", StorageDataType::Utf8),
            ],
        );
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int64(Some(1)));
        values.insert("a".to_string(), ScalarValue::Utf8(Some("hello".to_string())));
        values.insert("b".to_string(), ScalarValue::Utf8(Some("world".to_string())));
        let row = UserTableRow {
            user_id: UserId::new("u1"),
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row { values },
        };
        let encoded = encode_user_row_v2(&row, &schema).unwrap();
        let (header, payload) =
            crate::object::decode_envelope(encoded.as_slice(), crate::object::ObjectKind::Row)
                .unwrap();

        // Corrupt the first variable end offset so end0 > end1 (non-monotonic).
        let mut bad_offsets = payload.to_vec();
        // Layout: schema_version(2) + row_flags(1) + slot_span(2) + fixed(8) = 13; offsets start at
        // 13.
        bad_offsets[13] = 10;
        bad_offsets[14] = 5;
        let err = RowView::parse(&bad_offsets, &schema, header.flags).unwrap_err();
        assert!(err.to_string().contains("invalid v2 variable offset"));

        // Corrupt the final variable end offset so it under-reads var_data (trailing bytes).
        let mut trailing = payload.to_vec();
        trailing[14] = 8;
        let err = RowView::parse(&trailing, &schema, header.flags).unwrap_err();
        assert!(err.to_string().contains("trailing bytes after v2 variable data"));
    }

    #[test]
    fn two_column_row_decodes_after_add_drop_rename_and_alter_type() {
        let v1 = StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int32),
                StorageField::new("legacy_name", StorageDataType::Utf8),
            ],
        );
        let mut values = std::collections::BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int32(Some(77)));
        values.insert("legacy_name".to_string(), ScalarValue::Utf8(Some("drop-me".to_string())));
        let row = UserTableRow {
            user_id: UserId::new("u1"),
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row { values },
        };
        let encoded_v1 = encode_user_row(&row, &v1).unwrap();

        // v4 after: ALTER id TYPE BIGINT, DROP legacy_name (slot 1 hole), ADD title WITH DEFAULT,
        // RENAME title -> label
        let mut v4 = StorageSchema::new(
            4,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::dropped_slot(),
                StorageField::new("label", StorageDataType::Utf8)
                    .with_missing_default(Some(ScalarValue::Utf8(Some("backfilled".to_string())))),
            ],
        );
        v4.register_historical_version(1, &v1.fields);

        let decoded = decode_user_row(
            encoded_v1.as_slice(),
            &v4,
            UserId::new("u1"),
            VersionId::try_from_i64(1).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded.fields.values.get("id"), Some(&ScalarValue::Int64(Some(77))));
        assert!(!decoded.fields.values.contains_key("legacy_name"));
        assert_eq!(
            decoded.fields.values.get("label"),
            Some(&ScalarValue::Utf8(Some("backfilled".to_string())))
        );
    }

    #[test]
    fn v2_historical_fixed_values_use_the_stored_type_before_coercion() {
        let cases = [
            (
                StorageDataType::Int32,
                StorageDataType::Int64,
                ScalarValue::Int32(Some(-77)),
                ScalarValue::Int64(Some(-77)),
            ),
            (
                StorageDataType::Float32,
                StorageDataType::Float64,
                ScalarValue::Float32(Some(1.5)),
                ScalarValue::Float64(Some(1.5)),
            ),
            (
                StorageDataType::TimestampMillisecond,
                StorageDataType::TimestampMicrosecond,
                ScalarValue::TimestampMillisecond(Some(123), None),
                ScalarValue::TimestampMicrosecond(Some(123_000), None),
            ),
        ];
        for (stored_type, current_type, value, expected) in cases {
            let old = StorageSchema::new(1, vec![StorageField::new("value", stored_type)]);
            let mut current = StorageSchema::new(2, vec![StorageField::new("value", current_type)]);
            current.register_historical_version(1, &old.fields);
            let row = SharedTableRow {
                _version: VersionId::try_from_i64(1).unwrap(),
                _deleted: false,
                fields: Row {
                    values: [("value".to_string(), value)].into_iter().collect(),
                },
            };
            let encoded = encode_shared_row_v2(&row, &old).unwrap();
            let decoded = decode_shared_row(encoded.as_slice(), &current, row._version).unwrap();
            assert_eq!(decoded.fields.values["value"], expected);
            let selected =
                decode_shared_row_selected(encoded.as_slice(), &current, row._version, &[0])
                    .unwrap();
            assert_eq!(selected.fields, decoded.fields);
        }
    }

    #[test]
    fn v2_rejects_missing_historical_layout_instead_of_reinterpreting_bytes() {
        let old = StorageSchema::new(1, vec![StorageField::new("value", StorageDataType::Int64)]);
        let current =
            StorageSchema::new(2, vec![StorageField::new("value", StorageDataType::Float64)]);
        let row = SharedTableRow {
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row {
                values: [("value".to_string(), ScalarValue::Int64(Some(42)))].into_iter().collect(),
            },
        };
        let encoded = encode_shared_row_v2(&row, &old).unwrap();
        assert!(decode_shared_row(encoded.as_slice(), &current, row._version).is_err());
    }

    #[test]
    fn project_row_fields_reads_one_column_from_v1_and_v2() {
        let schema = StorageSchema::new(
            1,
            vec![
                StorageField::new("id", StorageDataType::Int64),
                StorageField::new("note", StorageDataType::Utf8),
            ],
        );
        let row = SharedTableRow {
            _version: VersionId::try_from_i64(1).unwrap(),
            _deleted: false,
            fields: Row {
                values: [
                    ("id".to_string(), ScalarValue::Int64(Some(7))),
                    ("note".to_string(), ScalarValue::Utf8(Some("skip me".to_string()))),
                ]
                .into_iter()
                .collect(),
            },
        };
        let encoded_v1 = encode_shared_row(&row, &schema).unwrap();
        let mut projected = Vec::new();
        let deleted =
            project_row_fields(encoded_v1.as_slice(), &schema, &[0], |_, value| {
                projected.push(value);
                Ok(())
            })
            .unwrap();
        assert!(!deleted);
        assert_eq!(projected, vec![ScalarValue::Int64(Some(7))]);

        let encoded_v2 = encode_shared_row_v2(&row, &schema).unwrap();
        projected.clear();
        let deleted =
            project_row_fields(encoded_v2.as_slice(), &schema, &[0], |_, value| {
                projected.push(value);
                Ok(())
            })
            .unwrap();
        assert!(!deleted);
        assert_eq!(projected, vec![ScalarValue::Int64(Some(7))]);
    }
}
