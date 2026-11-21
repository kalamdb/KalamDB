"""Smoke tests for the generated-table runtime against a real server.

The unit tests in test_schema.py and test_codecs.py check the SQL builders and
the codecs in isolation. These check the pair against a live server, which is
where the wire shapes actually come from: DATE arrives as a day count, TIME and
TIMESTAMP as microseconds, BYTES as an array of byte values. Reading those back
as strings, or reading a timestamp near the epoch as milliseconds, is invisible
to a unit test that supplies its own fixtures.

One table of each kind is exercised, so USER, SHARED and STREAM are all known
to work with the declarations `kalam schema gen --languages python` emits.

The DDL here uses the server's runtime form, `WITH (TYPE = ...)`. The
`CREATE USER TABLE` spelling the generated bindings are declared from belongs
to schema.sql, which the contract compiler reads; the SQL API does not accept
it.
"""

import datetime as dt

import pytest

from kalamdb.schema import (
    Column,
    KTable,
    TableKind,
    UnsupportedParameterError,
    bigint,
    bytes_,
    date,
    text,
    time,
    timestamp,
)


def shared_notes(namespace):
    """A SHARED table, declared the way generated bindings declare one."""

    class Columns:
        id = bigint("id").primary_key().not_null().server_default()
        author = text("author").not_null()
        body = text("body").not_null()
        tags = Column("tags", "TEXT[]")

    return KTable.shared(f"{namespace}.notes", Columns)


def user_profiles(namespace):
    """A USER table carrying every column type that decodes from a number."""

    class Columns:
        id = bigint("id").primary_key().not_null().server_default()
        email = text("email").not_null()
        born_on = date("born_on")
        wakes_at = time("wakes_at")
        avatar = bytes_("avatar")
        seen_at = timestamp("seen_at")

    return KTable.user(f"{namespace}.profiles", Columns)


def stream_events(namespace):
    """A STREAM table: append-only events."""

    class Columns:
        seq = bigint("seq").primary_key().not_null()
        kind = text("kind").not_null()
        at = timestamp("at")

    return KTable.stream(f"{namespace}.events", Columns)


@pytest.mark.asyncio
async def test_shared_table_round_trips_through_the_builders(client, temp_namespace):
    notes = shared_notes(temp_namespace)
    assert notes.kind is TableKind.SHARED
    await client.query(f"""
        CREATE TABLE {temp_namespace}.notes (
            id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
            author TEXT NOT NULL,
            body TEXT NOT NULL,
            tags TEXT[]
        ) WITH (TYPE = 'SHARED')
    """)

    sql, params = notes.insert_sql({"author": "manik", "body": "hello"})
    await client.query(sql, params)

    sql, params = notes.select_sql("author = $1", ["manik"])
    rows = await client.query_rows(sql, params)
    assert len(rows) == 1
    key = int(rows[0]["id"])

    sql, params = notes.update_sql(key, {"body": "edited"})
    await client.query(sql, params)
    sql, params = notes.select_by_key_sql(key)
    assert (await client.query_rows(sql, params))[0]["body"] == "edited"

    sql, params = notes.delete_sql(key)
    await client.query(sql, params)
    sql, params = notes.select_sql()
    assert await client.query_rows(sql, params) == []


@pytest.mark.asyncio
async def test_user_table_decodes_date_time_bytes_and_timestamp(
    client, temp_namespace
):
    """The wire shapes the server really sends, not the ones a fixture guesses."""
    profiles = user_profiles(temp_namespace)
    assert profiles.kind is TableKind.USER
    await client.query(f"""
        CREATE TABLE {temp_namespace}.profiles (
            id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
            email TEXT NOT NULL,
            born_on DATE,
            wakes_at TIME,
            avatar BYTES,
            seen_at TIMESTAMP
        ) WITH (TYPE = 'USER')
    """)

    # DATE, TIME and BYTES cannot be bound as parameters, so they are written
    # as literals; the point of the test is what comes back, not how it got in.
    await client.query(f"""
        INSERT INTO {temp_namespace}.profiles (email, born_on, wakes_at, avatar, seen_at)
        VALUES ('a@b.c', DATE '2026-01-01', TIME '12:34:56',
                X'010203', TIMESTAMP '1970-01-02 00:00:00')
    """)

    sql, params = profiles.select_sql("email = $1", ["a@b.c"])
    row = (await client.query_rows(sql, params))[0]

    from kalamdb.schema import as_bytes, as_date, as_datetime, as_time

    assert as_date(row["born_on"]) == dt.date(2026, 1, 1)
    assert as_time(row["wakes_at"]) == dt.time(12, 34, 56)
    assert as_bytes(row["avatar"]) == b"\x01\x02\x03"
    # A timestamp one day after the epoch stays in 1970: read as milliseconds
    # it became 1972.
    seen_at = as_datetime(row["seen_at"])
    assert (seen_at.year, seen_at.month, seen_at.day) == (1970, 1, 2)


@pytest.mark.asyncio
async def test_stream_table_appends_and_reads_back(client, temp_namespace):
    events = stream_events(temp_namespace)
    assert events.kind is TableKind.STREAM
    await client.query(f"""
        CREATE TABLE {temp_namespace}.events (
            seq BIGINT PRIMARY KEY,
            kind TEXT NOT NULL,
            at TIMESTAMP
        ) WITH (TYPE = 'STREAM', TTL_SECONDS = 3600)
    """)

    for seq, kind in ((1, "created"), (2, "updated")):
        sql, params = events.insert_sql({"seq": seq, "kind": kind})
        await client.query(sql, params)

    sql, params = events.select_sql()
    rows = await client.query_rows(sql, params)
    assert sorted(int(row["seq"]) for row in rows) == [1, 2]

    sql, params = events.select_by_key_sql(2)
    assert (await client.query_rows(sql, params))[0]["kind"] == "updated"


@pytest.mark.asyncio
async def test_array_parameters_are_refused_with_the_server_s_reason(
    client, temp_namespace
):
    """The SDK refuses the bind, and the server agrees when asked directly."""
    notes = shared_notes(temp_namespace)
    await client.query(f"""
        CREATE TABLE {temp_namespace}.notes (
            id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
            author TEXT NOT NULL,
            body TEXT NOT NULL,
            tags TEXT[]
        ) WITH (TYPE = 'SHARED')
    """)

    with pytest.raises(UnsupportedParameterError, match="tags"):
        notes.insert_sql({"author": "a", "body": "b", "tags": ["x"]})

    # Sent anyway, the server is the one that rejects it. If this ever starts
    # succeeding, array binding has landed and the SDK guard should go.
    with pytest.raises(Exception) as caught:
        await client.query(
            f"INSERT INTO {temp_namespace}.notes (author, body, tags) "
            "VALUES ($1, $2, $3)",
            ["a", "b", ["x"]],
        )
    assert "Array parameters not yet supported" in str(caught.value)
