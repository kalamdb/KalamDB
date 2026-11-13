"""Unit tests for table definitions. No server needed."""

import pytest

from kalamdb.schema import (
    KTable,
    TableKind,
    bigint,
    embedding,
    file,
    json,
    text,
    timestamp,
)


def notes_table():
    return KTable.shared(
        "playground.notes",
        id=bigint("id").primary_key().server_default("SNOWFLAKE_ID()"),
        author=text("author").not_null(),
        body=text("body").not_null(),
        created_at=timestamp("created_at").not_null().server_default("NOW()"),
    )


def test_qualified_name_splits_namespace():
    notes = notes_table()
    assert notes.namespace == "playground"
    assert notes.name == "notes"
    assert notes.kind is TableKind.SHARED


def test_unqualified_name_has_no_namespace():
    t = KTable.user("inbox", id=bigint("id").primary_key())
    assert t.namespace is None
    assert t.qualified_name == "inbox"


def events_table():
    return KTable.stream(
        "playground.events",
        event_id=bigint("event_id").primary_key().server_default("SNOWFLAKE_ID()"),
        kind=text("kind").not_null(),
        payload=json("payload"),
    )


def test_create_table_sql_includes_kind_and_constraints():
    sql = notes_table().create_table_sql()
    assert sql.startswith("CREATE TABLE IF NOT EXISTS playground.notes (")
    assert "id BIGINT DEFAULT SNOWFLAKE_ID() PRIMARY KEY" in sql
    assert "author TEXT NOT NULL" in sql
    assert "created_at TIMESTAMP NOT NULL DEFAULT NOW()" in sql
    assert sql.endswith("WITH (TYPE = 'SHARED')")


def test_create_table_sql_without_the_existence_guard():
    sql = notes_table().create_table_sql(if_not_exists=False)
    assert sql.startswith("CREATE TABLE playground.notes (")


def test_user_tables_declare_their_kind_as_an_option():
    profiles = KTable.user("playground.profiles", id=bigint("id").primary_key())
    sql = profiles.create_table_sql()
    assert sql.startswith("CREATE TABLE IF NOT EXISTS playground.profiles (")
    assert sql.endswith("WITH (TYPE = 'USER')")


def test_stream_tables_require_a_ttl():
    with pytest.raises(ValueError, match="ttl_seconds"):
        events_table().create_table_sql()


def test_stream_table_options_render_in_a_stable_order():
    events = (
        events_table()
        .max_stream_size_bytes(1048576)
        .eviction_strategy("hybrid")
        .ttl_seconds(3600)
    )
    assert events.create_table_sql().endswith(
        "WITH (TYPE = 'STREAM', TTL_SECONDS = 3600, EVICTION_STRATEGY = 'hybrid', "
        "MAX_STREAM_SIZE_BYTES = 1048576)"
    )


def test_storage_options_render_for_user_and_shared_tables():
    notes = notes_table().storage_id("local-ssd").flush_policy(rows=1000).compression("ZSTD")
    assert notes.create_table_sql().endswith(
        "WITH (TYPE = 'SHARED', STORAGE_ID = 'local-ssd', FLUSH_POLICY = 'rows:1000', "
        "COMPRESSION = 'zstd')"
    )

    profiles = KTable.user(
        "playground.profiles", id=bigint("id").primary_key()
    ).use_user_storage()
    assert profiles.create_table_sql().endswith(
        "WITH (TYPE = 'USER', USE_USER_STORAGE = true)"
    )


def test_flush_policy_accepts_both_bounds():
    notes = notes_table().flush_policy(rows=1000, interval_seconds=60)
    assert "FLUSH_POLICY = 'rows:1000,interval:60'" in notes.create_table_sql()


def test_options_are_rejected_for_the_wrong_table_kind():
    with pytest.raises(ValueError, match="TTL_SECONDS is only supported for STREAM"):
        notes_table().ttl_seconds(60)
    with pytest.raises(ValueError, match="STORAGE_ID is only supported"):
        events_table().storage_id("local-ssd")
    with pytest.raises(ValueError, match="COMPRESSION is only supported"):
        events_table().compression("zstd")
    with pytest.raises(ValueError, match="USE_USER_STORAGE is only supported for USER"):
        notes_table().use_user_storage()
    with pytest.raises(ValueError, match="EVICTION_STRATEGY is only supported for STREAM"):
        notes_table().eviction_strategy("hybrid")


def test_option_values_are_validated():
    with pytest.raises(ValueError, match="invalid compression"):
        notes_table().compression("lz4")
    with pytest.raises(ValueError, match="invalid eviction strategy"):
        events_table().eviction_strategy("random")
    with pytest.raises(ValueError, match="invalid storage id"):
        notes_table().storage_id("local ssd")
    with pytest.raises(ValueError, match="ttl seconds must be positive"):
        events_table().ttl_seconds(0)
    with pytest.raises(ValueError, match="rows or interval_seconds"):
        notes_table().flush_policy()


def test_server_default_without_an_expression_emits_no_ddl_default():
    t = KTable.shared("app.t", id=bigint("id").primary_key().server_default())
    assert "DEFAULT" not in t.create_table_sql()
    sql, params = t.insert_sql({})
    assert params == []


def test_system_tables_carry_no_options():
    users = KTable.system("system.users", user_id=text("user_id").primary_key())
    sql = users.create_table_sql()
    assert sql.startswith("CREATE TABLE IF NOT EXISTS system.users")
    assert "WITH" not in sql
    with pytest.raises(ValueError, match="only supported"):
        users.compression("zstd")


def test_insert_omits_server_filled_columns():
    sql, params = notes_table().insert_sql({"author": "manik", "body": "hello"})
    assert sql == (
        "INSERT INTO playground.notes (author, body) VALUES ($1, $2)"
    )
    assert params == ["manik", "hello"]


def test_insert_keeps_a_server_default_column_when_given_explicitly():
    sql, params = notes_table().insert_sql({"id": 7, "author": "a", "body": "b"})
    assert sql.startswith("INSERT INTO playground.notes (id, author, body)")
    assert params == [7, "a", "b"]


def test_insert_rejects_a_missing_required_column():
    with pytest.raises(ValueError, match="body"):
        notes_table().insert_sql({"author": "manik"})


def test_insert_rejects_an_unknown_column():
    with pytest.raises(KeyError, match="nickname"):
        notes_table().insert_sql({"author": "a", "body": "b", "nickname": "x"})


def test_select_sql_with_and_without_a_predicate():
    notes = notes_table()
    assert notes.select_sql() == "SELECT * FROM playground.notes"
    assert notes.select_sql("author = $1") == "SELECT * FROM playground.notes WHERE author = $1"


def test_embedding_carries_its_dimensions():
    assert embedding("vec", 384).sql_type == "EMBEDDING(384)"
    with pytest.raises(ValueError):
        embedding("vec", 0)


def test_kalamdb_specific_column_types():
    assert file("attachment").sql_type == "FILE"


def test_table_rejects_an_empty_column_set():
    with pytest.raises(ValueError, match="no columns"):
        KTable.shared("app.empty")


def test_column_lookup():
    notes = notes_table()
    assert notes.column("body").sql_type == "TEXT"
    with pytest.raises(KeyError):
        notes.column("missing")
