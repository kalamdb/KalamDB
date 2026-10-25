//! Ordinal KOBJ row codec vs the previous name-keyed FlexBuffers row.
//!
//! Reports encoded size and throughput. Run:
//! `cargo bench --profile dev -p kalamdb-serialization --bench codec`

use std::{
    collections::{BTreeMap, HashMap},
    hint::black_box,
    sync::Arc,
    time::Duration,
};

use arrow::{
    array::{Int64Array, ListArray, StringArray, StructArray},
    buffer::OffsetBuffer,
    datatypes::{DataType, Field},
};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use datafusion_common::ScalarValue;
use kalamdb_commons::{
    ids::VersionId,
    models::{
        rows::{Row, UserTableRow},
        KalamCellValue, UserId,
    },
    websocket_messages::{ChangeTypeRaw, ServerMessage},
};
use kalamdb_serialization::{
    decode_object, decode_payloads_to_arrow_batch, decode_server_message, decode_user_row,
    decode_user_row_selected, encode_object, encode_server_message, encode_user_row,
    encode_user_row_v2, StorageDataType, StorageField, StorageSchema,
};
use serde::{Deserialize, Serialize};

const BATCH_SIZE: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CatalogObject {
    name: String,
    version: u32,
    tags: Vec<String>,
}

fn customer_type() -> StorageDataType {
    StorageDataType::Struct(vec![
        StorageField::new("id", StorageDataType::Int64),
        StorageField::new("name", StorageDataType::Utf8),
    ])
}

fn nested_schema() -> StorageSchema {
    StorageSchema::new(
        1,
        vec![
            StorageField::new("id", StorageDataType::Int64),
            StorageField::new("customer", customer_type()),
            StorageField::new("tags", StorageDataType::List(Box::new(StorageDataType::Utf8))),
        ],
    )
}

fn scalar_schema() -> StorageSchema {
    StorageSchema::new(
        1,
        vec![
            StorageField::new("id", StorageDataType::Int64),
            StorageField::new("conversation_id", StorageDataType::Int64),
            StorageField::new("body", StorageDataType::Utf8),
        ],
    )
}

fn text_heavy_schema() -> StorageSchema {
    StorageSchema::new(
        1,
        vec![
            StorageField::new("id", StorageDataType::Int64),
            StorageField::new("title", StorageDataType::Utf8),
            StorageField::new("summary", StorageDataType::Utf8),
            StorageField::new("content", StorageDataType::Utf8),
            StorageField::new("author", StorageDataType::Utf8),
            StorageField::new("metadata", StorageDataType::Utf8),
        ],
    )
}

fn text_heavy_row() -> UserTableRow {
    let mut values = BTreeMap::new();
    values.insert("id".to_string(), ScalarValue::Int64(Some(99)));
    values.insert(
        "title".to_string(),
        ScalarValue::Utf8(Some("PostgreSQL 18 Compatibility & Row Storage v2".to_string())),
    );
    values.insert(
        "summary".to_string(),
        ScalarValue::Utf8(Some(
            "Zero-copy fixed region + packed variable offsets for fast selective projections"
                .to_string(),
        )),
    );
    values.insert("content".to_string(), ScalarValue::Utf8(Some("x".repeat(512))));
    values.insert("author".to_string(), ScalarValue::Utf8(Some("kalamdb-team".to_string())));
    values.insert(
        "metadata".to_string(),
        ScalarValue::Utf8(Some("{\"env\":\"prod\",\"region\":\"us-east-1\"}".to_string())),
    );
    UserTableRow {
        user_id: UserId::new("user-1"),
        _version: VersionId::try_from_i64(100).unwrap(),
        _deleted: false,
        fields: Row { values },
    }
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
        .expect("empty list");
        return ScalarValue::List(Arc::new(list));
    }
    ScalarValue::List(ScalarValue::new_list(&items, &DataType::Utf8, true))
}

fn nested_row() -> UserTableRow {
    let mut values = BTreeMap::new();
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

fn scalar_row() -> UserTableRow {
    let mut values = BTreeMap::new();
    values.insert("id".to_string(), ScalarValue::Int64(Some(42)));
    values.insert("conversation_id".to_string(), ScalarValue::Int64(Some(7)));
    values.insert(
        "body".to_string(),
        ScalarValue::Utf8(Some("hello from kalamdb 0.7".to_string())),
    );
    UserTableRow {
        user_id: UserId::new("user-1"),
        _version: VersionId::try_from_i64(100).unwrap(),
        _deleted: false,
        fields: Row { values },
    }
}

fn encode_name_keyed(row: &UserTableRow) -> Vec<u8> {
    flexbuffers::to_vec(row).expect("name-keyed flexbuffers encode")
}

fn catalog_object() -> CatalogObject {
    CatalogObject {
        name: "app.messages".to_string(),
        version: 4,
        tags: vec!["chat".to_string(), "rls".to_string()],
    }
}

fn print_sizes() {
    let nested = nested_row();
    let scalar = scalar_row();
    let nested_kobj = encode_user_row(&nested, &nested_schema()).unwrap();
    let scalar_kobj = encode_user_row(&scalar, &scalar_schema()).unwrap();
    let nested_legacy = encode_name_keyed(&nested);
    let scalar_legacy = encode_name_keyed(&scalar);
    let object = encode_object(&catalog_object()).unwrap();
    eprintln!(
        "codec sizes: nested_kobj={} nested_name_keyed={} scalar_kobj={} scalar_name_keyed={} \
         object={}",
        nested_kobj.len(),
        nested_legacy.len(),
        scalar_kobj.len(),
        scalar_legacy.len(),
        object.len()
    );
}

fn bench_codec(c: &mut Criterion) {
    print_sizes();

    let nested = nested_row();
    let scalar = scalar_row();
    let nested_schema = nested_schema();
    let scalar_schema = scalar_schema();
    let nested_bytes = encode_user_row(&nested, &nested_schema).unwrap().into_bytes();
    let nested_v2_bytes = encode_user_row_v2(&nested, &nested_schema).unwrap().into_bytes();
    let scalar_bytes = encode_user_row(&scalar, &scalar_schema).unwrap().into_bytes();
    let scalar_v2_bytes = encode_user_row_v2(&scalar, &scalar_schema).unwrap().into_bytes();
    let nested_legacy = encode_name_keyed(&nested);
    let scalar_legacy = encode_name_keyed(&scalar);
    let catalog = catalog_object();
    let catalog_bytes = encode_object(&catalog).unwrap().into_bytes();

    let mut group = c.benchmark_group("serialization_codec");
    group.measurement_time(Duration::from_secs(8));
    group.warm_up_time(Duration::from_secs(2));

    group.throughput(Throughput::Bytes(nested_bytes.len() as u64));
    group.bench_function("nested_row_encode_kobj", |b| {
        b.iter(|| encode_user_row(black_box(&nested), black_box(&nested_schema)).unwrap())
    });
    group.bench_function("nested_row_decode_kobj", |b| {
        b.iter(|| {
            decode_user_row(
                black_box(&nested_bytes),
                black_box(&nested_schema),
                UserId::new("user-1"),
                VersionId::try_from_i64(100).unwrap(),
            )
            .unwrap()
        })
    });
    group.throughput(Throughput::Bytes(nested_v2_bytes.len() as u64));
    group.bench_function("nested_row_encode_v2", |b| {
        b.iter(|| encode_user_row_v2(black_box(&nested), black_box(&nested_schema)).unwrap())
    });
    group.bench_function("nested_row_decode_v2", |b| {
        b.iter(|| {
            decode_user_row(
                black_box(&nested_v2_bytes),
                black_box(&nested_schema),
                UserId::new("user-1"),
                VersionId::try_from_i64(100).unwrap(),
            )
            .unwrap()
        })
    });
    group.bench_function("nested_row_decode_selected_v2", |b| {
        b.iter(|| {
            decode_user_row_selected(
                black_box(&nested_v2_bytes),
                black_box(&nested_schema),
                UserId::new("user-1"),
                VersionId::try_from_i64(100).unwrap(),
                black_box(&[0]),
            )
            .unwrap()
        })
    });
    group.bench_function("scalar_row_decode_selected_v2", |b| {
        b.iter(|| {
            decode_user_row_selected(
                black_box(&scalar_v2_bytes),
                black_box(&scalar_schema),
                UserId::new("user-1"),
                VersionId::try_from_i64(100).unwrap(),
                black_box(&[0, 1]),
            )
            .unwrap()
        })
    });
    group.throughput(Throughput::Bytes(nested_legacy.len() as u64));
    group.bench_function("nested_row_encode_name_keyed", |b| {
        b.iter(|| encode_name_keyed(black_box(&nested)))
    });
    group.bench_function("nested_row_decode_name_keyed", |b| {
        b.iter(|| {
            let decoded: UserTableRow =
                flexbuffers::from_slice(black_box(&nested_legacy)).expect("name-keyed decode");
            decoded
        })
    });

    group.throughput(Throughput::Bytes(scalar_bytes.len() as u64));
    group.bench_function("scalar_row_encode_kobj", |b| {
        b.iter(|| encode_user_row(black_box(&scalar), black_box(&scalar_schema)).unwrap())
    });
    group.bench_function("scalar_row_decode_kobj", |b| {
        b.iter(|| {
            decode_user_row(
                black_box(&scalar_bytes),
                black_box(&scalar_schema),
                UserId::new("user-1"),
                VersionId::try_from_i64(100).unwrap(),
            )
            .unwrap()
        })
    });
    group.throughput(Throughput::Bytes(scalar_legacy.len() as u64));
    group.bench_function("scalar_row_encode_name_keyed", |b| {
        b.iter(|| encode_name_keyed(black_box(&scalar)))
    });
    group.bench_function("scalar_row_decode_name_keyed", |b| {
        b.iter(|| {
            let decoded: UserTableRow =
                flexbuffers::from_slice(black_box(&scalar_legacy)).expect("name-keyed decode");
            decoded
        })
    });

    group.throughput(Throughput::Bytes((scalar_bytes.len() * BATCH_SIZE) as u64));
    group.bench_function("batch_scalar_encode_decode_kobj", |b| {
        b.iter_batched(
            || Vec::with_capacity(BATCH_SIZE),
            |mut encoded| {
                encoded.clear();
                for _ in 0..BATCH_SIZE {
                    encoded.push(encode_user_row(&scalar, &scalar_schema).unwrap().into_bytes());
                }
                for bytes in &encoded {
                    decode_user_row(
                        bytes,
                        &scalar_schema,
                        UserId::new("user-1"),
                        VersionId::try_from_i64(100).unwrap(),
                    )
                    .unwrap();
                }
                encoded
            },
            BatchSize::SmallInput,
        )
    });

    group.throughput(Throughput::Bytes(catalog_bytes.len() as u64));
    group.bench_function("object_encode", |b| {
        b.iter(|| encode_object(black_box(&catalog)).unwrap())
    });
    group.bench_function("object_decode", |b| {
        b.iter(|| {
            let _: CatalogObject = decode_object(black_box(&catalog_bytes)).unwrap();
        })
    });

    let text_heavy = text_heavy_row();
    let text_schema = text_heavy_schema();
    let text_v1_bytes = encode_user_row(&text_heavy, &text_schema).unwrap().into_bytes();
    let text_v2_bytes = encode_user_row_v2(&text_heavy, &text_schema).unwrap().into_bytes();
    let text_arrow_schema = Arc::new(arrow::datatypes::Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("title", DataType::Utf8, true),
        Field::new("summary", DataType::Utf8, true),
        Field::new("content", DataType::Utf8, true),
        Field::new("author", DataType::Utf8, true),
        Field::new("metadata", DataType::Utf8, true),
    ]));
    let text_batch_v1_slices: Vec<&[u8]> = vec![text_v1_bytes.as_slice(); BATCH_SIZE];
    let text_batch_v2_slices: Vec<&[u8]> = vec![text_v2_bytes.as_slice(); BATCH_SIZE];

    group.throughput(Throughput::Bytes(text_v1_bytes.len() as u64));
    group.bench_function("text_heavy_encode_v1", |b| {
        b.iter(|| encode_user_row(black_box(&text_heavy), black_box(&text_schema)).unwrap())
    });
    group.bench_function("text_heavy_decode_v1", |b| {
        b.iter(|| {
            decode_user_row(
                black_box(&text_v1_bytes),
                black_box(&text_schema),
                UserId::new("user-1"),
                VersionId::try_from_i64(100).unwrap(),
            )
            .unwrap()
        })
    });
    group.throughput(Throughput::Bytes(text_v2_bytes.len() as u64));
    group.bench_function("text_heavy_encode_v2", |b| {
        b.iter(|| encode_user_row_v2(black_box(&text_heavy), black_box(&text_schema)).unwrap())
    });
    group.bench_function("text_heavy_decode_v2", |b| {
        b.iter(|| {
            decode_user_row(
                black_box(&text_v2_bytes),
                black_box(&text_schema),
                UserId::new("user-1"),
                VersionId::try_from_i64(100).unwrap(),
            )
            .unwrap()
        })
    });
    group.throughput(Throughput::Bytes((text_v1_bytes.len() * BATCH_SIZE) as u64));
    group.bench_function("payloads_to_record_batch_v1", |b| {
        b.iter(|| {
            decode_payloads_to_arrow_batch(
                black_box(&text_schema),
                black_box(Arc::clone(&text_arrow_schema)),
                black_box(&text_batch_v1_slices),
            )
            .unwrap()
        })
    });
    group.throughput(Throughput::Bytes((text_v2_bytes.len() * BATCH_SIZE) as u64));
    group.bench_function("payloads_to_record_batch_v2", |b| {
        b.iter(|| {
            decode_payloads_to_arrow_batch(
                black_box(&text_schema),
                black_box(Arc::clone(&text_arrow_schema)),
                black_box(&text_batch_v2_slices),
            )
            .unwrap()
        })
    });

    group.finish();
}

fn bench_websocket(c: &mut Criterion) {
    let mut group = c.benchmark_group("serialization_websocket");
    group.measurement_time(Duration::from_secs(3));
    group.warm_up_time(Duration::from_secs(1));
    group.sample_size(30);
    for count in [1, 256] {
        let rows = (0..count)
            .map(|index| {
                HashMap::from([
                    ("id".to_string(), KalamCellValue::int(index)),
                    (
                        "body".to_string(),
                        KalamCellValue::from_json(serde_json::json!("hello from KalamDB")),
                    ),
                    (
                        "tags".to_string(),
                        KalamCellValue::from_json(serde_json::json!(["one", "two"])),
                    ),
                    (
                        "metadata".to_string(),
                        KalamCellValue::from_json(serde_json::json!({"active": true, "score": 42})),
                    ),
                    ("optional".to_string(), KalamCellValue::null()),
                ])
            })
            .collect();
        let message = ServerMessage::Change {
            subscription_id: "sub-1".to_string(),
            change_type: ChangeTypeRaw::Insert,
            rows: Some(rows),
            old_values: None,
        };
        let bytes = encode_server_message(&message).unwrap();
        assert_eq!(decode_server_message(&bytes).unwrap(), message);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_function(format!("change_decode_{count}"), |b| {
            b.iter(|| decode_server_message(black_box(&bytes)).unwrap())
        });
        group.bench_function(format!("change_encode_{count}"), |b| {
            b.iter(|| encode_server_message(black_box(&message)).unwrap())
        });
    }
    group.finish();
}

criterion_group!(benches, bench_codec, bench_websocket);
criterion_main!(benches);
