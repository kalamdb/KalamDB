"""Unit tests for table declarations and the SQL they build. No server needed."""

from dataclasses import dataclass
from typing import Any, Mapping, Optional

import pytest

from kalamdb.schema import (
    MAX_EMBEDDING_DIMENSIONS,
    Column,
    KTable,
    TableKind,
    bigint,
    embedding,
    file,
    json,
    quote_ident,
    quote_table,
    text,
    timestamp,
)


@dataclass(frozen=True)
class Note:
    """Stands in for a generated row class; value codecs are tested separately."""

    id: int
    author: str
    body: str

    @classmethod
    def from_row(cls, row: Mapping[str, Any]) -> "Note":
        return cls(id=int(row["id"]), author=str(row["author"]), body=str(row["body"]))

    def to_row(self):
        return {"id": self.id, "author": self.author, "body": self.body}


class NoteColumns:
    id = bigint("id").primary_key().server_default()
    author = text("author").not_null()
    body = text("body").not_null()
    created_at = timestamp("created_at").not_null().server_default()


def notes_table():
    return KTable.shared("playground.notes", NoteColumns, row=Note)


# -- identifiers --------------------------------------------------------------


def test_quote_ident_escapes_embedded_quotes():
    assert quote_ident("body") == '"body"'
    assert quote_ident('we"ird') == '"we""ird"'


def test_quote_ident_rejects_empty_and_nul():
    with pytest.raises(ValueError):
        quote_ident("")
    with pytest.raises(ValueError):
        quote_ident("bo\x00dy")


def test_quote_table_quotes_each_part():
    assert quote_table("chat.profiles") == '"chat"."profiles"'
    assert quote_table("inbox") == '"inbox"'
    with pytest.raises(ValueError):
        quote_table("")


# -- declaration --------------------------------------------------------------


def test_table_id_splits_namespace():
    notes = notes_table()
    assert notes.table_id == "playground.notes"
    assert notes.namespace == "playground"
    assert notes.name == "notes"
    assert notes.kind is TableKind.SHARED


def test_unqualified_name_has_no_namespace():
    t = KTable.user("inbox", {"id": bigint("id").primary_key()})
    assert t.namespace is None
    assert t.table_id == "inbox"


def test_each_factory_records_its_kind():
    cols = {"id": bigint("id").primary_key()}
    assert KTable.table("a.b", cols).kind is TableKind.UNSPECIFIED
    assert KTable.user("a.b", cols).kind is TableKind.USER
    assert KTable.shared("a.b", cols).kind is TableKind.SHARED
    assert KTable.stream("a.b", cols).kind is TableKind.STREAM


def test_columns_keep_declaration_order():
    assert [c.name for c in notes_table()] == ["id", "author", "body", "created_at"]


def test_columns_class_is_reachable_from_the_table():
    assert notes_table().c is NoteColumns


def test_key_columns_follow_primary_key_ordinals():
    t = KTable.stream(
        "chat.events",
        {
            "seq": bigint("seq").primary_key(2),
            "room": text("room").primary_key(1),
            "payload": json("payload"),
        },
    )
    assert t.key_columns == ("room", "seq")


def test_declaration_rejects_bad_input():
    with pytest.raises(ValueError, match="no columns"):
        KTable.shared("app.empty", {})
    with pytest.raises(TypeError, match="not a Column"):
        KTable.shared("app.t", {"id": "nope"})
    with pytest.raises(TypeError, match="columns must be"):
        KTable.shared("app.t", ["id"])
    with pytest.raises(ValueError, match="twice"):
        KTable.shared("app.t", {"a": bigint("id"), "b": Column("id", "BIGINT")})


def test_system_columns_are_not_dropped():
    """_version, _deleted and _timestamp are real column names, not privates."""
    class Columns:
        id = bigint("id").primary_key()
        _version = bigint("_version").not_null()
        _deleted = text("_deleted")

    t = KTable.shared("app.rows", Columns)
    assert [c.name for c in t] == ["id", "_version", "_deleted"]
    assert t.column("_version").is_not_null


def test_declaration_still_rejects_non_columns_beside_dunders():
    class Columns:
        """A docstring and annotations add dunders, which are skipped."""

        id = bigint("id").primary_key()
        oops = "not a column"

    with pytest.raises(TypeError, match="not a Column"):
        KTable.shared("app.rows", Columns)


def test_quote_table_rejects_an_empty_namespace():
    # ".notes" is malformed, not unqualified.
    with pytest.raises(ValueError, match="empty namespace"):
        quote_table(".notes")


def test_key_columns_follow_a_column_marked_after_construction():
    class Columns:
        id = bigint("id")
        label = text("label").not_null()

    t = KTable.shared("app.rows", Columns)
    assert t.key_columns == ()
    t.column("id").primary_key()
    assert t.key_columns == ("id",)
    assert t.select_by_key_sql(1)[0].endswith('WHERE "id" = $1')


def test_composite_key_accepts_any_sequence():
    t = KTable.stream(
        "chat.events",
        {
            "room": text("room").primary_key(1),
            "seq": bigint("seq").primary_key(2),
        },
    )
    expected = 'SELECT * FROM "chat"."events" WHERE "room" = $1 AND "seq" = $2'
    assert t.select_by_key_sql(["a", 1]) == (expected, ["a", 1])
    assert t.select_by_key_sql(range(2)) == (expected, [0, 1])
    # A string is a sequence but is never a composite key.
    with pytest.raises(ValueError, match="composite key"):
        t.select_by_key_sql("ab")


def test_column_lookup():
    notes = notes_table()
    assert notes.column("body").sql_type == "TEXT"
    with pytest.raises(KeyError):
        notes.column("missing")


# -- what the review asked us to drop ----------------------------------------


def test_ddl_and_option_builders_are_gone():
    notes = notes_table()
    assert not hasattr(KTable, "system")
    assert not hasattr(notes, "create_table_sql")
    assert not hasattr(notes, "options_sql")
    assert not hasattr(notes.column("body"), "ddl")
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


def test_table_kind_has_no_system_member():
    assert [k.name for k in TableKind] == ["UNSPECIFIED", "USER", "SHARED", "STREAM"]


# -- insert -------------------------------------------------------------------


def test_insert_omits_server_filled_columns_and_quotes_identifiers():
    sql, params = notes_table().insert_sql({"author": "manik", "body": "hello"})
    assert sql == (
        'INSERT INTO "playground"."notes" ("author", "body") VALUES ($1, $2)'
    )
    assert params == ["manik", "hello"]


def test_insert_keeps_a_server_default_column_when_given_explicitly():
    sql, params = notes_table().insert_sql({"id": 7, "author": "a", "body": "b"})
    assert sql.startswith('INSERT INTO "playground"."notes" ("id", "author", "body")')
    assert params == [7, "a", "b"]


def test_insert_rejects_a_missing_required_column():
    with pytest.raises(ValueError, match="body"):
        notes_table().insert_sql({"author": "manik"})


def test_insert_rejects_an_unknown_column():
    with pytest.raises(KeyError, match="nickname"):
        notes_table().insert_sql({"author": "a", "body": "b", "nickname": "x"})


def test_insert_skips_read_only_columns_and_rejects_writing_them():
    t = KTable.shared(
        "app.t",
        {
            "id": bigint("id").primary_key().server_default(),
            "version": bigint("version").read_only(),
            "label": text("label").not_null(),
        },
    )
    sql, params = t.insert_sql({"label": "x"})
    assert "version" not in sql
    assert params == ["x"]
    with pytest.raises(ValueError, match="read-only"):
        t.insert_sql({"label": "x", "version": 2})


def test_insert_rejects_an_empty_payload():
    # A required column is reported first, since that is the more useful error.
    with pytest.raises(ValueError, match="NOT NULL"):
        notes_table().insert_sql({})

    # When every column is server-filled there is nothing to bind at all.
    t = KTable.shared("app.t", {"id": bigint("id").primary_key().server_default()})
    with pytest.raises(ValueError, match="supplied no columns"):
        t.insert_sql({})


# -- update and delete --------------------------------------------------------


def test_update_sets_columns_then_binds_the_key():
    sql, params = notes_table().update_sql(7, {"body": "edited"})
    assert sql == 'UPDATE "playground"."notes" SET "body" = $1 WHERE "id" = $2'
    assert params == ["edited", 7]


def test_update_refuses_keys_read_only_and_empty_patches():
    notes = notes_table()
    with pytest.raises(ValueError, match="primary key"):
        notes.update_sql(7, {"id": 8})
    with pytest.raises(ValueError, match="no columns"):
        notes.update_sql(7, {})
    with pytest.raises(KeyError):
        notes.update_sql(7, {"nope": 1})


def test_delete_binds_the_key():
    sql, params = notes_table().delete_sql(7)
    assert sql == 'DELETE FROM "playground"."notes" WHERE "id" = $1'
    assert params == [7]


# -- select -------------------------------------------------------------------


def test_select_without_a_predicate():
    sql, params = notes_table().select_sql()
    assert sql == 'SELECT * FROM "playground"."notes"'
    assert params == []


def test_select_passes_params_through_rather_than_interpolating():
    sql, params = notes_table().select_sql("author = $1", ["manik"])
    assert sql == 'SELECT * FROM "playground"."notes" WHERE author = $1'
    assert params == ["manik"]


def test_select_params_without_a_where_fragment_is_rejected():
    with pytest.raises(ValueError, match="where fragment"):
        notes_table().select_sql(None, ["manik"])


def test_select_by_key():
    sql, params = notes_table().select_by_key_sql(7)
    assert sql == 'SELECT * FROM "playground"."notes" WHERE "id" = $1'
    assert params == [7]


# -- composite and missing keys ----------------------------------------------


def composite_table():
    return KTable.stream(
        "chat.events",
        {
            "room": text("room").primary_key(1),
            "seq": bigint("seq").primary_key(2),
            "payload": json("payload"),
        },
    )


def test_composite_key_accepts_a_sequence_or_a_mapping():
    t = composite_table()
    expected = 'SELECT * FROM "chat"."events" WHERE "room" = $1 AND "seq" = $2'
    assert t.select_by_key_sql(("general", 4)) == (expected, ["general", 4])
    assert t.select_by_key_sql({"seq": 4, "room": "general"}) == (expected, ["general", 4])


def test_composite_key_rejects_bad_shapes():
    t = composite_table()
    with pytest.raises(ValueError, match="composite key"):
        t.select_by_key_sql(4)
    with pytest.raises(ValueError, match="composite key of 2 columns"):
        t.select_by_key_sql(("general",))
    with pytest.raises(ValueError, match="missing"):
        t.select_by_key_sql({"room": "general"})


def test_keyed_operations_need_a_primary_key():
    t = KTable.stream("chat.log", {"line": text("line").not_null()})
    assert t.key_columns == ()
    with pytest.raises(ValueError, match="no primary key"):
        t.delete_sql(1)
    with pytest.raises(ValueError, match="no primary key"):
        t.select_by_key_sql(1)
    # a plain select still works
    assert t.select_sql()[0] == 'SELECT * FROM "chat"."log"'


# -- decoding -----------------------------------------------------------------


def test_decode_without_a_row_class_is_rejected():
    t = KTable.shared("app.t", {"id": bigint("id").primary_key()})
    with pytest.raises(TypeError, match="without a row class"):
        t.decode({"id": 1})


# -- columns ------------------------------------------------------------------


def test_embedding_bounds_match_the_server():
    assert embedding("vec", 384).sql_type == "EMBEDDING(384)"
    assert embedding("vec", MAX_EMBEDDING_DIMENSIONS).sql_type.endswith(
        f"({MAX_EMBEDDING_DIMENSIONS})"
    )
    with pytest.raises(ValueError, match="between 1 and"):
        embedding("vec", 0)
    with pytest.raises(ValueError, match="between 1 and"):
        embedding("vec", MAX_EMBEDDING_DIMENSIONS + 1)


def test_kalamdb_specific_column_types():
    assert file("attachment").sql_type == "FILE"
    assert json("payload").sql_type == "JSON"


def test_column_rejects_an_empty_name_and_negative_ordinal():
    with pytest.raises(ValueError, match="must not be empty"):
        bigint("")
    with pytest.raises(ValueError, match="ordinal"):
        bigint("id").primary_key(-1)
