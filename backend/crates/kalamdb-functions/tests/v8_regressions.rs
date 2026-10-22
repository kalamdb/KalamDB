use datafusion_common::ScalarValue;
use kalamdb_commons::RoutineId;
use kalamdb_functions::{ModuleRevision, RoutineValue, RuntimeLimits, V8Session};
use tokio_util::sync::CancellationToken;

#[test]
fn settled_promise_returns_its_value() {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(
            "async function kalamInvoke(name, args) { return await Promise.resolve(args[0] + 1); }",
        ),
        RuntimeLimits::default(),
    )
    .unwrap();
    let result = session
        .invoke(
            &RoutineId::new("increment"),
            &[RoutineValue::new(ScalarValue::Int32(Some(6)))],
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(result.value, ScalarValue::Int32(Some(7)));
}

#[test]
fn transfer_safe_int64_is_js_number_so_plus_one_is_arithmetic() {
    let encoded = kalamdb_commons::conversions::arrow_json_conversion::scalar_value_to_js_json(
        &ScalarValue::Int64(Some(41)),
    )
    .expect("js json");
    let bytes = kalamdb_serialization::encode_function_value("", &encoded.0).unwrap();
    let mut arg = RoutineValue::new(ScalarValue::Int64(Some(41)));
    arg.transfer = Some(bytes.into());
    arg.contract_hash = Some(String::new());
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(kalamdb_functions::wrap_procedure_source(
            "return input + 1;",
        )),
        RuntimeLimits::default(),
    )
    .unwrap();
    let result = session
        .invoke(&RoutineId::new("inc"), &[arg], &CancellationToken::new())
        .unwrap();
    assert_eq!(
        result.value,
        ScalarValue::Int64(Some(42)),
        "Int64 transfer must be a JS number; string concat would yield 411"
    );
}

#[test]
fn reusable_session_does_not_retain_user_globals() {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(
            "function kalamInvoke() { globalThis.count = (globalThis.count || 0) + 1; return \
             count; }",
        ),
        RuntimeLimits::default(),
    )
    .unwrap();
    for _ in 0..3 {
        let value = session
            .invoke(&RoutineId::new("count"), &[], &CancellationToken::new())
            .unwrap();
        assert_eq!(value.value, ScalarValue::Int32(Some(1)));
    }
}

#[test]
fn native_conversion_rejects_sparse_arrays_before_allocating() {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture("function kalamInvoke() { return new Array(1000000); }"),
        RuntimeLimits::default(),
    )
    .unwrap();
    assert!(session
        .invoke(&RoutineId::new("sparse"), &[], &CancellationToken::new())
        .is_err());
}

#[test]
fn native_conversion_rejects_deep_values() {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(
            "function kalamInvoke() { let value = 1; for(let i=0;i<80;i++) value={child:value}; \
             return value; }",
        ),
        RuntimeLimits::default(),
    )
    .unwrap();
    assert!(session.invoke(&RoutineId::new("deep"), &[], &CancellationToken::new()).is_err());
}

#[test]
fn array_buffer_allocation_is_bounded() {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(
            "function kalamInvoke() { return new ArrayBuffer(128 * 1024 * 1024).byteLength; }",
        ),
        RuntimeLimits::default(),
    )
    .unwrap();
    assert!(session
        .invoke(&RoutineId::new("buffer"), &[], &CancellationToken::new())
        .is_err());
}

#[test]
fn native_conversion_rejects_cycles_and_mixed_arrays_without_panicking() {
    for expression in [
        "(() => { const v = {}; v.self = v; return v; })()",
        "[1, 'mixed']",
        "new WebAssembly.Memory({initial: 2048})",
    ] {
        let mut session = V8Session::load(
            ModuleRevision::typescript_fixture(format!(
                "function kalamInvoke() {{ return {expression}; }}"
            )),
            RuntimeLimits::default(),
        )
        .unwrap();
        assert!(session
            .invoke(&RoutineId::new("invalid"), &[], &CancellationToken::new())
            .is_err());
    }
}

#[test]
fn resizable_backing_stores_cannot_bypass_memory_budget() {
    for expression in [
        "new ArrayBuffer(128 * 1024 * 1024, {maxByteLength: 256 * 1024 * 1024}).byteLength",
        "new SharedArrayBuffer(128 * 1024 * 1024, {maxByteLength: 256 * 1024 * 1024}).byteLength",
    ] {
        let mut session = V8Session::load(
            ModuleRevision::typescript_fixture(format!(
                "function kalamInvoke() {{ return {expression}; }}"
            )),
            RuntimeLimits::default(),
        )
        .unwrap();
        assert!(
            session
                .invoke(&RoutineId::new("buffer"), &[], &CancellationToken::new())
                .is_err(),
            "{expression}"
        );
    }
}

#[test]
fn throwing_result_accessors_fail_conversion() {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(
            "function kalamInvoke() { return { get value() { throw new Error('accessor failure'); \
             } }; }",
        ),
        RuntimeLimits::default(),
    )
    .unwrap();
    assert!(session
        .invoke(&RoutineId::new("getter"), &[], &CancellationToken::new())
        .is_err());
}

#[test]
fn unsafe_int64_uses_bigint_and_numeric_uses_decimal_string() {
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(kalamdb_functions::wrap_procedure_source(
            "return { big: typeof input.big === 'bigint' ? input.big + 1n : 0n, price: \
             input.price };",
        )),
        RuntimeLimits::default(),
    )
    .unwrap();
    let big_val = (1i64 << 53) + 7;
    let arg = RoutineValue::named_object(vec![
        ("big".to_string(), RoutineValue::new(ScalarValue::Int64(Some(big_val)))),
        (
            "price".to_string(),
            RoutineValue::new(ScalarValue::Decimal128(Some(12345), 10, 2)),
        ),
    ]);
    let result = session
        .invoke(&RoutineId::new("num"), &[arg], &CancellationToken::new())
        .unwrap();
    let ScalarValue::Struct(struct_arr) = result.value else {
        panic!("expected struct");
    };
    let big_col = struct_arr.column_by_name("big").expect("big col");
    let price_col = struct_arr.column_by_name("price").expect("price col");
    assert_eq!(
        ScalarValue::try_from_array(big_col, 0).unwrap(),
        ScalarValue::Int64(Some(big_val + 1))
    );
    assert_eq!(
        ScalarValue::try_from_array(price_col, 0).unwrap(),
        ScalarValue::Utf8(Some("123.45".to_string()))
    );
}

#[test]
fn read_view_supports_lazy_projection_and_materialization() {
    use std::sync::Arc;

    use arrow::{
        array::{Int64Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema},
    };

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(vec![10, 20, 30])),
            Arc::new(StringArray::from(vec!["alice", "bob", "carol"])),
        ],
    )
    .unwrap();
    let view_arg = RoutineValue::from_read_view_batches(vec![batch]);
    let mut session = V8Session::load(
        ModuleRevision::typescript_fixture(kalamdb_functions::wrap_procedure_source(
            "const second = input.at(1); const all = input.materialize(); return { len: \
             input.length, secondName: second.name, allCount: all.length, firstId: all[0].id };",
        )),
        RuntimeLimits::default(),
    )
    .unwrap();
    let result = session
        .invoke(&RoutineId::new("view"), &[view_arg], &CancellationToken::new())
        .unwrap();
    let ScalarValue::Struct(struct_arr) = result.value else {
        panic!("expected struct");
    };
    assert_eq!(
        ScalarValue::try_from_array(struct_arr.column_by_name("len").unwrap(), 0).unwrap(),
        ScalarValue::Int32(Some(3))
    );
    assert_eq!(
        ScalarValue::try_from_array(struct_arr.column_by_name("secondName").unwrap(), 0).unwrap(),
        ScalarValue::Utf8(Some("bob".to_string()))
    );
    assert_eq!(
        ScalarValue::try_from_array(struct_arr.column_by_name("allCount").unwrap(), 0).unwrap(),
        ScalarValue::Int32(Some(3))
    );
    assert_eq!(
        ScalarValue::try_from_array(struct_arr.column_by_name("firstId").unwrap(), 0).unwrap(),
        ScalarValue::Int32(Some(10))
    );
}
