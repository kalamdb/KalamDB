//! Scenario 15: KalamDB 0.7 row storage + scalar indexes.
//!
//! Contract-level coverage for the 0.7 storage/index work:
//! - USER rows survive schema evolution and keep key-derived identity.
//! - SHARED rows survive schema evolution before and after hot -> cold flush.
//! - STREAM rows round-trip with key-derived `_seq`.
//! - Scalar CREATE/DROP INDEX is reflected in `system.schemas`.
//! - Indexed equality remains correct before and after flush.

use std::time::Duration;

use kalamdb_commons::Role;

use super::helpers::*;

#[tokio::test]
async fn test_v07_user_rows_survive_additive_schema_change() -> anyhow::Result<()> {
    let server = crate::test_support::http_server::get_global_server().await;
    let ns = unique_ns("v07_user_row");

    let resp = server.execute_sql(&format!("CREATE NAMESPACE {ns}")).await?;
    assert_success(&resp, "CREATE NAMESPACE");

    let resp = server
        .execute_sql(&format!(
            r#"CREATE TABLE {ns}.items (
                id BIGINT PRIMARY KEY,
                name TEXT NOT NULL
            ) WITH (TYPE = 'USER')"#
        ))
        .await?;
    assert_success(&resp, "CREATE USER table");

    let user = format!("{ns}_user");
    let client = create_user_and_client(server, &user, &Role::User).await?;

    let resp = client
        .execute_query(
            &format!("INSERT INTO {ns}.items (id, name) VALUES (1, 'before')"),
            None,
            None,
            None,
        )
        .await?;
    assert!(resp.success(), "insert before ALTER must succeed: {resp:?}");

    let resp = server
        .execute_sql(&format!("ALTER TABLE {ns}.items ADD COLUMN note TEXT"))
        .await?;
    assert_success(&resp, "ALTER TABLE ADD COLUMN");

    let resp = client
        .execute_query(
            &format!(
                "INSERT INTO {ns}.items (id, name, note) VALUES (2, 'after', 'new-slot')"
            ),
            None,
            None,
            None,
        )
        .await?;
    assert!(resp.success(), "insert after ALTER must succeed: {resp:?}");

    let resp = client
        .execute_query(
            &format!("SELECT id, name, note, user_id, _seq FROM {ns}.items ORDER BY id"),
            None,
            None,
            None,
        )
        .await?;
    assert!(resp.success(), "read after schema change must succeed: {resp:?}");

    let rows = resp.rows_as_maps();
    assert_eq!(rows.len(), 2);

    assert_eq!(rows[0].get("id").and_then(json_to_i64), Some(1));
    assert!(
        rows[0].get("note").map(|v| v.is_null()).unwrap_or(false),
        "missing trailing ordinal on an old row must decode as NULL: {:?}",
        rows[0]
    );
    assert_eq!(rows[0].get("user_id").and_then(|v| v.as_str()), Some(user.as_str()));
    assert!(rows[0].get("_seq").and_then(json_to_i64).unwrap_or_default() > 0);

    assert_eq!(rows[1].get("id").and_then(json_to_i64), Some(2));
    assert_eq!(rows[1].get("note").and_then(|v| v.as_str()), Some("new-slot"));
    assert_eq!(rows[1].get("user_id").and_then(|v| v.as_str()), Some(user.as_str()));
    assert!(rows[1].get("_seq").and_then(json_to_i64).unwrap_or_default() > 0);

    let _ = server.execute_sql(&format!("DROP NAMESPACE {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
async fn test_v07_shared_rows_survive_schema_change_and_flush() -> anyhow::Result<()> {
    let server = crate::test_support::http_server::get_global_server().await;
    let ns = unique_ns("v07_shared_row");

    let resp = server.execute_sql(&format!("CREATE NAMESPACE {ns}")).await?;
    assert_success(&resp, "CREATE NAMESPACE");

    let resp = server
        .execute_sql(&format!(
            r#"CREATE TABLE {ns}.messages (
                id BIGINT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                body TEXT NOT NULL
            ) WITH (
                TYPE = 'SHARED',
                STORAGE_ID = 'local'
            )"#
        ))
        .await?;
    assert_success(&resp, "CREATE SHARED table");

    let resp = server
        .execute_sql(&format!(
            "INSERT INTO {ns}.messages (id, conversation_id, body) VALUES (1, 'room-a', 'before')"
        ))
        .await?;
    assert_success(&resp, "insert before ALTER");

    let resp = server
        .execute_sql(&format!("ALTER TABLE {ns}.messages ADD COLUMN metadata JSONB"))
        .await?;
    assert_success(&resp, "ALTER TABLE ADD COLUMN");

    let resp = server
        .execute_sql(&format!(
            r#"INSERT INTO {ns}.messages (id, conversation_id, body, metadata)
               VALUES (2, 'room-a', 'after', '{"source":"e2e","nested":{"ok":true}}')"#
        ))
        .await?;
    assert_success(&resp, "insert after ALTER");

    let before_flush = server
        .execute_sql(&format!(
            "SELECT id, body, metadata FROM {ns}.messages WHERE conversation_id = 'room-a' ORDER BY id"
        ))
        .await?;
    assert_success(&before_flush, "read before flush");
    assert_eq!(before_flush.rows().len(), 2);

    let rows = before_flush.rows_as_maps();
    assert!(rows[0].get("metadata").map(|v| v.is_null()).unwrap_or(false));
    let metadata = rows[1].get("metadata").expect("metadata must be present");
    assert!(
        metadata.to_string().contains("nested") && metadata.to_string().contains("true"),
        "nested JSONB value must round-trip: {metadata:?}"
    );

    let flush = server
        .execute_sql(&format!("STORAGE FLUSH TABLE {ns}.messages"))
        .await?;
    assert_success(&flush, "STORAGE FLUSH TABLE");
    wait_for_flush_complete(server, &ns, "messages", Duration::from_secs(20)).await?;

    let after_flush = server
        .execute_sql(&format!(
            "SELECT id, body, metadata FROM {ns}.messages WHERE conversation_id = 'room-a' ORDER BY id"
        ))
        .await?;
    assert_success(&after_flush, "read after flush");
    assert_eq!(after_flush.rows().len(), 2);

    let rows = after_flush.rows_as_maps();
    assert!(rows[0].get("metadata").map(|v| v.is_null()).unwrap_or(false));
    let metadata = rows[1].get("metadata").expect("metadata must be present after flush");
    assert!(metadata.to_string().contains("nested"));

    let _ = server.execute_sql(&format!("DROP NAMESPACE {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
async fn test_v07_stream_rows_reconstruct_seq_from_storage_key() -> anyhow::Result<()> {
    let server = crate::test_support::http_server::get_global_server().await;
    let ns = unique_ns("v07_stream_row");

    let resp = server.execute_sql(&format!("CREATE NAMESPACE {ns}")).await?;
    assert_success(&resp, "CREATE NAMESPACE");

    let resp = server
        .execute_sql(&format!(
            r#"CREATE TABLE {ns}.events (
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL
            ) WITH (TYPE = 'STREAM', TTL_SECONDS = 3600)"#
        ))
        .await?;
    assert_success(&resp, "CREATE STREAM table");

    for (event_type, payload) in [("created", "one"), ("updated", "two")] {
        let resp = server
            .execute_sql(&format!(
                "INSERT INTO {ns}.events (event_type, payload) VALUES ('{event_type}', '{payload}')"
            ))
            .await?;
        assert_success(&resp, "INSERT STREAM row");
    }

    let resp = server
        .execute_sql(&format!(
            "SELECT _seq, event_type, payload FROM {ns}.events ORDER BY _seq"
        ))
        .await?;
    assert_success(&resp, "SELECT STREAM rows");

    let rows = resp.rows_as_maps();
    assert_eq!(rows.len(), 2);

    let seq1 = rows[0].get("_seq").and_then(json_to_i64).unwrap_or_default();
    let seq2 = rows[1].get("_seq").and_then(json_to_i64).unwrap_or_default();
    assert!(seq1 > 0, "first stream row must have a reconstructed _seq");
    assert!(seq2 > seq1, "stream _seq values must preserve storage-key order");
    assert_eq!(rows[0].get("payload").and_then(|v| v.as_str()), Some("one"));
    assert_eq!(rows[1].get("payload").and_then(|v| v.as_str()), Some("two"));

    let _ = server.execute_sql(&format!("DROP NAMESPACE {ns} CASCADE")).await;
    Ok(())
}

#[tokio::test]
async fn test_v07_scalar_index_catalog_hot_and_cold_equality() -> anyhow::Result<()> {
    let server = crate::test_support::http_server::get_global_server().await;
    let ns = unique_ns("v07_scalar_index");
    let index_name = format!("idx_{ns}_conversation");

    let resp = server.execute_sql(&format!("CREATE NAMESPACE {ns}")).await?;
    assert_success(&resp, "CREATE NAMESPACE");

    let resp = server
        .execute_sql(&format!(
            r#"CREATE TABLE {ns}.messages (
                id BIGINT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                body TEXT NOT NULL
            ) WITH (
                TYPE = 'SHARED',
                STORAGE_ID = 'local'
            )"#
        ))
        .await?;
    assert_success(&resp, "CREATE SHARED table");

    let resp = server
        .execute_sql(&format!(
            "CREATE INDEX {index_name} ON {ns}.messages(conversation_id)"
        ))
        .await?;
    assert_success(&resp, "CREATE INDEX");

    for i in 0..60_i64 {
        let room = if i == 17 || i == 43 { "target-room" } else { "noise-room" };
        let resp = server
            .execute_sql(&format!(
                "INSERT INTO {ns}.messages (id, conversation_id, body) VALUES ({i}, '{room}', 'body-{i}')"
            ))
            .await?;
        assert_success(&resp, "INSERT indexed row");
    }

    let catalog = server
        .execute_sql(&format!(
            "SELECT indexes FROM system.schemas WHERE namespace_id = '{ns}' AND table_name = 'messages' AND is_latest = true"
        ))
        .await?;
    assert_success(&catalog, "read index catalog");
    let indexes = get_first_string(&catalog, "indexes")
        .unwrap_or_else(|| format!("{:?}", catalog.rows_as_maps()));
    assert!(
        indexes.contains(&index_name) && indexes.contains("conversation_id"),
        "system.schemas.indexes must expose the scalar index; got {indexes}"
    );

    let hot = server
        .execute_sql(&format!(
            "SELECT id, body FROM {ns}.messages WHERE conversation_id = 'target-room' ORDER BY id"
        ))
        .await?;
    assert_success(&hot, "indexed equality on hot rows");
    assert_eq!(hot.rows().len(), 2);
    let hot_rows = hot.rows_as_maps();
    assert_eq!(hot_rows[0].get("id").and_then(json_to_i64), Some(17));
    assert_eq!(hot_rows[1].get("id").and_then(json_to_i64), Some(43));

    let flush = server
        .execute_sql(&format!("STORAGE FLUSH TABLE {ns}.messages"))
        .await?;
    assert_success(&flush, "STORAGE FLUSH TABLE");
    wait_for_flush_complete(server, &ns, "messages", Duration::from_secs(20)).await?;

    let cold = server
        .execute_sql(&format!(
            "SELECT id, body FROM {ns}.messages WHERE conversation_id = 'target-room' ORDER BY id"
        ))
        .await?;
    assert_success(&cold, "indexed equality after flush");
    assert_eq!(cold.rows().len(), 2);
    let cold_rows = cold.rows_as_maps();
    assert_eq!(cold_rows[0].get("id").and_then(json_to_i64), Some(17));
    assert_eq!(cold_rows[1].get("id").and_then(json_to_i64), Some(43));

    let resp = server
        .execute_sql(&format!("DROP INDEX {index_name}"))
        .await?;
    assert_success(&resp, "DROP INDEX");

    let catalog = server
        .execute_sql(&format!(
            "SELECT indexes FROM system.schemas WHERE namespace_id = '{ns}' AND table_name = 'messages' AND is_latest = true"
        ))
        .await?;
    assert_success(&catalog, "read index catalog after DROP INDEX");
    let indexes = get_first_string(&catalog, "indexes")
        .unwrap_or_else(|| format!("{:?}", catalog.rows_as_maps()));
    assert!(
        !indexes.contains(&index_name),
        "dropped index must disappear from system.schemas.indexes; got {indexes}"
    );

    // Dropping the acceleration structure must not change SQL semantics.
    let after_drop = server
        .execute_sql(&format!(
            "SELECT id FROM {ns}.messages WHERE conversation_id = 'target-room' ORDER BY id"
        ))
        .await?;
    assert_success(&after_drop, "equality after DROP INDEX");
    assert_eq!(after_drop.rows().len(), 2);

    let _ = server.execute_sql(&format!("DROP NAMESPACE {ns} CASCADE")).await;
    Ok(())
}
