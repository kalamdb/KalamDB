"""Unit tests for table definitions. No server needed."""

import pytest

from kalamdb.schema import (
    KTable,
    TableKind,
    bigint,
    boolean,
    double,
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


def test_drop_table_sql_guards_on_existence_by_default():
    notes = notes_table()
    assert notes.drop_table_sql() == "DROP TABLE IF EXISTS playground.notes"
    assert notes.drop_table_sql(if_exists=False) == "DROP TABLE playground.notes"


def test_select_sql_renders_projection_order_limit_and_offset():
    notes = notes_table()
    assert notes.select_sql(
        "author = $1",
        columns=["id", "body"],
        order_by="created_at DESC",
        limit=10,
        offset=20,
    ) == (
        "SELECT id, body FROM playground.notes WHERE author = $1 "
        "ORDER BY created_at DESC LIMIT 10 OFFSET 20"
    )


def test_select_sql_rejects_an_undeclared_projection():
    with pytest.raises(KeyError, match="subject"):
        notes_table().select_sql(columns=["subject"])


def test_select_sql_rejects_nonsense_bounds():
    notes = notes_table()
    with pytest.raises(ValueError, match="limit"):
        notes.select_sql(limit=0)
    with pytest.raises(ValueError, match="offset"):
        notes.select_sql(offset=-1)


def test_count_sql_with_and_without_a_predicate():
    notes = notes_table()
    assert notes.count_sql() == "SELECT COUNT(*) FROM playground.notes"
    assert notes.count_sql("author = $1") == (
        "SELECT COUNT(*) FROM playground.notes WHERE author = $1"
    )


def test_primary_key_column_is_the_declared_one():
    assert notes_table().primary_key_column.name == "id"
    assert KTable.user("playground.loose", body=text("body")).primary_key_column is None


def test_delete_by_pk_sql_binds_the_value():
    sql, params = notes_table().delete_by_pk_sql(7)
    assert sql == "DELETE FROM playground.notes WHERE id = $1"
    assert params == [7]


def test_delete_by_pk_sql_needs_a_primary_key():
    loose = KTable.user("playground.loose", body=text("body"))
    with pytest.raises(ValueError, match="no primary_key"):
        loose.delete_by_pk_sql(7)


def test_insert_many_sql_shares_one_column_list():
    sql, params = notes_table().insert_many_sql(
        [{"author": "alice", "body": "one"}, {"author": "bob", "body": "two"}]
    )
    assert sql == (
        "INSERT INTO playground.notes (author, body) VALUES ($1, $2), ($3, $4)"
    )
    assert params == ["alice", "one", "bob", "two"]


def test_insert_many_sql_rejects_rows_filling_different_columns():
    with pytest.raises(ValueError, match="same columns"):
        notes_table().insert_many_sql(
            [{"author": "a", "body": "b"}, {"id": 1, "author": "c", "body": "d"}]
        )


def test_insert_many_sql_rejects_an_empty_batch():
    with pytest.raises(ValueError, match="at least one row"):
        notes_table().insert_many_sql([])


def uploads_table():
    return KTable.user(
        "playground.uploads",
        id=bigint("id").primary_key().server_default("SNOWFLAKE_ID()"),
        label=text("label").not_null(),
        attachment=file("attachment"),
    )


def test_insert_with_files_sql_turns_uploads_into_file_placeholders():
    sql, params, files = uploads_table().insert_with_files_sql(
        {"label": "demo", "attachment": ("demo.txt", b"body", "text/plain")}
    )
    assert sql == (
        "INSERT INTO playground.uploads (label, attachment) "
        "VALUES ($1, FILE('attachment'))"
    )
    assert params == ["demo"]
    assert files == {"attachment": ("demo.txt", b"body", "text/plain")}


def test_insert_with_files_sql_leaves_ordinary_values_as_params():
    sql, params, files = uploads_table().insert_with_files_sql(
        {"label": "demo", "attachment": "files/demo.txt"}
    )
    assert sql.endswith("VALUES ($1, $2)")
    assert params == ["demo", "files/demo.txt"]
    assert files == {}


def test_insert_sql_keeps_treating_tuples_as_plain_params():
    _, params = uploads_table().insert_sql(
        {"label": "demo", "attachment": ("demo.txt", b"body")}
    )
    assert params == ["demo", ("demo.txt", b"body")]


def test_column_parse_coerces_declared_types():
    assert bigint("id").parse("12") == 12
    assert double("ratio").parse("0.25") == 0.25
    assert boolean("active").parse("true") is True
    assert boolean("active").parse("false") is False
    assert json("payload").parse('{"a": 1}') == {"a": 1}
    assert embedding("vec", 2).parse("[0.5, 0.25]") == [0.5, 0.25]
    assert text("body").parse("hello") == "hello"


def test_column_parse_keeps_values_it_cannot_coerce():
    assert bigint("id").parse("not-a-number") == "not-a-number"
    assert json("payload").parse("{nope") == "{nope"
    assert timestamp("created_at").parse("2026-11-14T10:00:00Z") == (
        "2026-11-14T10:00:00Z"
    )
    assert bigint("id").parse(None) is None


def test_parse_row_leaves_undeclared_keys_alone():
    parsed = notes_table().parse_row({"id": "7", "body": "hi", "_version": "3"})
    assert parsed == {"id": 7, "body": "hi", "_version": "3"}
