//! Offline API surface tests — no running server required.

use kalam_client::{
    AuthProvider, AutoOffsetReset, KalamCellValue, LiveRowsConfig, SeqId, SubscriptionConfig,
    SubscriptionOptions, TopicConsumer,
};

#[test]
fn auth_provider_constructors_are_available() {
    let _ = AuthProvider::none();
    let _ = AuthProvider::jwt_token("token".to_string());
    let _ = AuthProvider::basic_auth("alice".into(), "secret".into());
    let _ = AuthProvider::system_user_auth("password".into());
}

#[test]
fn subscription_options_builder_sets_fields() {
    let options = SubscriptionOptions::new()
        .with_batch_size(100)
        .with_last_rows(50)
        .with_from(SeqId::from(42_i64));

    assert_eq!(options.batch_size, Some(100));
    assert_eq!(options.last_rows, Some(50));
    assert_eq!(options.from, Some(SeqId::from(42_i64)));
}

#[test]
fn subscription_config_carries_sql_and_options() {
    let mut config = SubscriptionConfig::new("sub-1", "SELECT * FROM app.messages");
    config.options = Some(SubscriptionOptions::new().with_last_rows(10));

    assert_eq!(config.id, "sub-1");
    assert!(config.sql.contains("app.messages"));
    assert_eq!(config.options.as_ref().unwrap().last_rows, Some(10));
}

#[test]
fn live_rows_config_defaults_to_id_key() {
    let config = LiveRowsConfig::default();
    assert!(config.limit.is_none());
    assert!(config.key_columns.is_none());
}

#[test]
fn kalam_cell_value_accessors_work() {
    let text = KalamCellValue::text("hello");
    assert_eq!(text.as_text(), Some("hello"));

    let integer = KalamCellValue::int(7);
    assert_eq!(integer.as_int(), Some(7));
}

#[test]
fn consumer_builder_requires_group_id() {
    let result = TopicConsumer::builder()
        .base_url("http://localhost:2900")
        .topic("demo.topic")
        .build();

    let err = match result {
        Ok(_) => panic!("group_id should be required"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("group_id"));
}

#[test]
fn consumer_builder_requires_topic() {
    let result = TopicConsumer::builder()
        .base_url("http://localhost:2900")
        .group_id("workers")
        .build();

    let err = match result {
        Ok(_) => panic!("topic should be required"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("topic"));
}

#[test]
fn consumer_builder_accepts_auto_offset_reset() {
    let built = TopicConsumer::builder()
        .base_url("http://localhost:2900")
        .group_id("workers")
        .topic("demo.topic")
        .auto_offset_reset(AutoOffsetReset::Earliest)
        .build();

    assert!(built.is_ok(), "expected valid consumer builder");
}
