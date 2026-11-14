"""Unit tests for tables bound to a client. No server needed — the client is a
stub that records what it was asked to run."""

import pytest

from kalamdb.binding import BoundTable
from kalamdb.schema import (
    KTable,
    bigint,
    boolean,
    double,
    embedding,
    file,
    json,
    text,
    timestamp,
)


class FakeClient:
    """Stands in for KalamClient, recording every call it receives."""

    def __init__(self, rows=None, response=None):
        self.calls = []
        self.rows = [] if rows is None else rows
        self.response = {"status": "success"} if response is None else response

    async def query(self, sql, params=None):
        self.calls.append({"method": "query", "sql": sql, "params": params})
        return self.response

    async def query_rows(self, sql, params=None):
        self.calls.append({"method": "query_rows", "sql": sql, "params": params})
        return self.rows

    async def query_with_files(self, sql, files, params=None):
        self.calls.append(
            {
                "method": "query_with_files",
                "sql": sql,
                "files": files,
                "params": params,
            }
        )
        return self.response

    async def live(self, sql, **options):
        self.calls.append({"method": "live", "sql": sql, "options": options})
        return "live-rows"

    async def live_events(self, sql, **options):
        self.calls.append({"method": "live_events", "sql": sql, "options": options})
        return "live-events"

    @property
    def last(self):
        return self.calls[-1]


def notes_table():
    return KTable.shared(
        "playground.notes",
        id=bigint("id").primary_key().server_default("SNOWFLAKE_ID()"),
        author=text("author").not_null(),
        body=text("body").not_null(),
        created_at=timestamp("created_at").not_null().server_default("NOW()"),
    ).compression("zstd")


def profiles_table():
    return KTable.user(
        "playground.profiles",
        id=bigint("id").primary_key().server_default("SNOWFLAKE_ID()"),
        display_name=text("display_name").not_null(),
        avatar=file("avatar"),
    )


def bind(table, **client_kwargs):
    client = FakeClient(**client_kwargs)
    return table.bind(client), client


def test_bind_returns_a_bound_table():
    table = notes_table()
    bound = table.bind(FakeClient())
    assert isinstance(bound, BoundTable)
    assert bound.table is table
    assert bound.qualified_name == "playground.notes"
    assert repr(bound) == "BoundTable('playground.notes', kind=SHARED)"


@pytest.mark.asyncio
async def test_create_runs_the_declared_ddl():
    db, client = bind(notes_table())
    await db.create()
    sql = client.last["sql"]
    assert sql.startswith("CREATE TABLE IF NOT EXISTS playground.notes (")
    assert sql.endswith("WITH (TYPE = 'SHARED', COMPRESSION = 'zstd')")


@pytest.mark.asyncio
async def test_create_can_drop_the_existence_guard():
    db, client = bind(notes_table())
    await db.create(if_not_exists=False)
    assert client.last["sql"].startswith("CREATE TABLE playground.notes (")


@pytest.mark.asyncio
async def test_drop_guards_on_existence_by_default():
    db, client = bind(notes_table())
    await db.drop()
    assert client.last["sql"] == "DROP TABLE IF EXISTS playground.notes"
    await db.drop(if_exists=False)
    assert client.last["sql"] == "DROP TABLE playground.notes"


@pytest.mark.asyncio
async def test_insert_binds_params_and_skips_server_defaults():
    db, client = bind(notes_table())
    await db.insert({"author": "alice", "body": "hello"})
    assert client.last["method"] == "query"
    assert client.last["sql"] == (
        "INSERT INTO playground.notes (author, body) VALUES ($1, $2)"
    )
    assert client.last["params"] == ["alice", "hello"]


@pytest.mark.asyncio
async def test_insert_rejects_a_missing_not_null_column():
    db, _ = bind(notes_table())
    with pytest.raises(ValueError, match="body is NOT NULL"):
        await db.insert({"author": "alice"})


@pytest.mark.asyncio
async def test_insert_rejects_an_undeclared_column():
    db, _ = bind(notes_table())
    with pytest.raises(KeyError, match="subject"):
        await db.insert({"author": "alice", "body": "hi", "subject": "nope"})


@pytest.mark.asyncio
async def test_insert_uploads_a_file_column_tuple():
    db, client = bind(profiles_table())
    await db.insert(
        {"display_name": "alice", "avatar": ("alice.png", b"bytes", "image/png")}
    )
    call = client.last
    assert call["method"] == "query_with_files"
    assert call["sql"] == (
        "INSERT INTO playground.profiles (display_name, avatar) "
        "VALUES ($1, FILE('avatar'))"
    )
    assert call["params"] == ["alice"]
    assert call["files"] == {"avatar": ("alice.png", b"bytes", "image/png")}


@pytest.mark.asyncio
async def test_insert_guesses_the_mime_type_of_a_2_tuple():
    db, client = bind(profiles_table())
    await db.insert({"display_name": "alice", "avatar": ("alice.png", b"bytes")})
    assert client.last["files"]["avatar"] == ("alice.png", b"bytes", "image/png")


@pytest.mark.asyncio
async def test_insert_reads_a_path_off_disk(tmp_path):
    avatar = tmp_path / "alice.png"
    avatar.write_bytes(b"png-bytes")
    db, client = bind(profiles_table())
    await db.insert({"display_name": "alice", "avatar": avatar})
    assert client.last["files"]["avatar"] == ("alice.png", b"png-bytes", "image/png")


@pytest.mark.asyncio
async def test_insert_rejects_bare_bytes_for_a_file_column():
    db, _ = bind(profiles_table())
    with pytest.raises(ValueError, match="needs a filename"):
        await db.insert({"display_name": "alice", "avatar": b"png-bytes"})


@pytest.mark.asyncio
async def test_insert_treats_a_file_string_as_a_reference():
    db, client = bind(profiles_table())
    await db.insert({"display_name": "alice", "avatar": "files/alice.png"})
    assert client.last["method"] == "query"
    assert client.last["sql"].endswith("VALUES ($1, $2)")
    assert client.last["params"] == ["alice", "files/alice.png"]


@pytest.mark.asyncio
async def test_insert_many_numbers_params_across_rows():
    db, client = bind(notes_table())
    await db.insert_many(
        [{"author": "alice", "body": "one"}, {"author": "bob", "body": "two"}]
    )
    assert client.last["sql"] == (
        "INSERT INTO playground.notes (author, body) VALUES ($1, $2), ($3, $4)"
    )
    assert client.last["params"] == ["alice", "one", "bob", "two"]


@pytest.mark.asyncio
async def test_insert_many_rejects_rows_filling_different_columns():
    db, _ = bind(notes_table())
    with pytest.raises(ValueError, match="same columns"):
        await db.insert_many(
            [
                {"author": "alice", "body": "one"},
                {"id": 7, "author": "bob", "body": "two"},
            ]
        )


@pytest.mark.asyncio
async def test_insert_many_rejects_an_empty_batch():
    db, _ = bind(notes_table())
    with pytest.raises(ValueError, match="at least one row"):
        await db.insert_many([])


@pytest.mark.asyncio
async def test_select_renders_the_whole_clause_set():
    db, client = bind(notes_table())
    await db.select(
        "author = $1",
        ["alice"],
        columns=["id", "body"],
        order_by="created_at DESC",
        limit=10,
        offset=20,
    )
    assert client.last["method"] == "query_rows"
    assert client.last["sql"] == (
        "SELECT id, body FROM playground.notes WHERE author = $1 "
        "ORDER BY created_at DESC LIMIT 10 OFFSET 20"
    )
    assert client.last["params"] == ["alice"]


@pytest.mark.asyncio
async def test_select_without_arguments_reads_everything():
    db, client = bind(notes_table())
    await db.select()
    assert client.last["sql"] == "SELECT * FROM playground.notes"
    assert client.last["params"] is None


@pytest.mark.asyncio
async def test_select_rejects_an_undeclared_projection():
    db, _ = bind(notes_table())
    with pytest.raises(KeyError, match="subject"):
        await db.select(columns=["subject"])


@pytest.mark.asyncio
async def test_select_parses_rows_into_declared_types():
    table = KTable.user(
        "playground.metrics",
        id=bigint("id").primary_key(),
        ratio=double("ratio"),
        active=boolean("active"),
        payload=json("payload"),
        vector=embedding("vector", 3),
    )
    raw = {
        "id": "12",
        "ratio": "0.25",
        "active": "true",
        "payload": '{"a": 1}',
        "vector": "[0.1, 0.2, 0.3]",
        "_version": "7",
    }
    db, _ = bind(table, rows=[raw])
    rows = await db.select()
    assert rows[0] == {
        "id": 12,
        "ratio": 0.25,
        "active": True,
        "payload": {"a": 1},
        "vector": [0.1, 0.2, 0.3],
        "_version": "7",
    }


@pytest.mark.asyncio
async def test_select_can_return_rows_exactly_as_sent():
    table = KTable.user("playground.metrics", id=bigint("id").primary_key())
    db, _ = bind(table, rows=[{"id": "12"}])
    assert await db.select(parse=False) == [{"id": "12"}]


@pytest.mark.asyncio
async def test_get_matches_the_declared_primary_key():
    db, client = bind(notes_table(), rows=[{"id": "7", "body": "hello"}])
    row = await db.get(7)
    assert client.last["sql"] == (
        "SELECT * FROM playground.notes WHERE id = $1 LIMIT 1"
    )
    assert client.last["params"] == [7]
    assert row == {"id": 7, "body": "hello"}


@pytest.mark.asyncio
async def test_get_returns_none_when_nothing_matches():
    db, _ = bind(notes_table(), rows=[])
    assert await db.get(7) is None


@pytest.mark.asyncio
async def test_get_needs_a_declared_primary_key():
    table = KTable.user("playground.loose", body=text("body"))
    db, _ = bind(table)
    with pytest.raises(ValueError, match="no primary_key"):
        await db.get(7)


@pytest.mark.asyncio
async def test_delete_matches_the_declared_primary_key():
    db, client = bind(notes_table())
    await db.delete(7)
    assert client.last["sql"] == "DELETE FROM playground.notes WHERE id = $1"
    assert client.last["params"] == [7]


@pytest.mark.asyncio
async def test_count_reads_the_first_value_of_the_first_row():
    db, client = bind(notes_table(), rows=[{"count(*)": "3"}])
    assert await db.count("author = $1", ["alice"]) == 3
    assert client.last["sql"] == "SELECT COUNT(*) FROM playground.notes WHERE author = $1"
    assert client.last["params"] == ["alice"]


@pytest.mark.asyncio
async def test_count_is_zero_when_the_server_returns_no_rows():
    db, _ = bind(notes_table(), rows=[])
    assert await db.count() == 0


@pytest.mark.asyncio
async def test_live_keys_on_the_primary_key_by_default():
    db, client = bind(notes_table())
    assert await db.live("author = 'alice'") == "live-rows"
    assert client.last["sql"] == (
        "SELECT * FROM playground.notes WHERE author = 'alice'"
    )
    assert client.last["options"] == {"key_columns": ["id"]}


@pytest.mark.asyncio
async def test_live_respects_explicit_options():
    db, client = bind(notes_table())
    await db.live(limit=50, key_columns=["author"])
    assert client.last["options"] == {"limit": 50, "key_columns": ["author"]}


@pytest.mark.asyncio
async def test_live_events_passes_the_select_through():
    db, client = bind(notes_table())
    assert await db.live_events(last_rows=5) == "live-events"
    assert client.last["sql"] == "SELECT * FROM playground.notes"
    assert client.last["options"] == {"last_rows": 5}
