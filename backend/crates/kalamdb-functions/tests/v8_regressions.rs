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
