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
        id=bigint("id").primary_key().server_default(),
        author=text("author").not_null(),
        body=text("body").not_null(),
        created_at=timestamp("created_at").not_null().server_default(),
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


def test_each_factory_records_its_kind():
    assert KTable.user("a.b", id=bigint("id")).kind is TableKind.USER
    assert KTable.shared("a.b", id=bigint("id")).kind is TableKind.SHARED
    assert KTable.stream("a.b", id=bigint("id")).kind is TableKind.STREAM


def test_system_factory_is_gone():
    assert not hasattr(KTable, "system")


def test_ddl_generation_is_gone():
    notes = notes_table()
    assert not hasattr(notes, "create_table_sql")
    assert not hasattr(notes, "options_sql")
    assert not hasattr(notes.column("body"), "ddl")


def test_table_option_builders_are_gone():
    notes = notes_table()
    for option in (
        "storage_id",
        "use_user_storage",
        "flush_policy",
        "compression",
        "deleted_retention_hours",
        "ttl_seconds",
        "eviction_strategy",
        "max_stream_size_bytes",
    ):
        assert not hasattr(notes, option), option


def test_insert_omits_server_filled_columns():
    sql, params = notes_table().insert_sql({"author": "manik", "body": "hello"})
    assert sql == "INSERT INTO playground.notes (author, body) VALUES ($1, $2)"
    assert params == ["manik", "hello"]


def test_insert_keeps_a_server_default_column_when_given_explicitly():
    sql, params = notes_table().insert_sql({"id": 7, "author": "a", "body": "b"})
    assert sql.startswith("INSERT INTO playground.notes (id, author, body)")
    assert params == [7, "a", "b"]


def test_insert_with_only_server_filled_columns_binds_nothing():
    t = KTable.shared("app.t", id=bigint("id").primary_key().server_default())
    sql, params = t.insert_sql({})
    assert params == []
    assert sql == "INSERT INTO app.t () VALUES ()"


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
    assert json("payload").sql_type == "JSON"


def test_table_rejects_an_empty_column_set():
    with pytest.raises(ValueError, match="no columns"):
        KTable.shared("app.empty")


def test_column_lookup():
    notes = notes_table()
    assert notes.column("body").sql_type == "TEXT"
    with pytest.raises(KeyError):
        notes.column("missing")
