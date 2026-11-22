"""Shape checks for generated Python bindings, verified with mypy --strict.

Mirrors fixtures/shape-check.ts and fixtures/generated_test.dart: it asserts the
generated row dataclass, enum, composite and column declarations carry the types
the contract promises, across all three table kinds. The negative boundary is
checked separately, by check_generated.py writing files mypy must reject.
"""

from __future__ import annotations

import datetime as dt
from typing import Any, Dict, List, Mapping, Optional

from kalamdb.schema import (
    Column,
    FileRef,
    Table,
    TableKind,
    UnsupportedParameterError,
)

from kalam import (
    KALAM_TABLES,
    Address,
    Events,
    Inventory,
    InventoryColumns,
    Mood,
    Profiles,
    ProfilesColumns,
    Rooms,
    events,
    inventory,
    profiles,
    rooms,
)


def decodes_a_wire_row(row: Mapping[str, Any]) -> Inventory:
    """Generated from_row accepts the mapping shape query_rows returns."""
    return Inventory.from_row(row)


def declared_field_types() -> Dict[str, Any]:
    """Every column keeps its declared type, including through arrays."""
    record = Inventory(
        id="1",
        sku="sku-1",
        tags=["a"],
        dates=[dt.datetime(2026, 1, 1)],
        moods=[Mood.CALM],
        addresses=[Address(city="Jammu")],
        attachment=None,
    )
    id_: str = record.id
    sku: str = record.sku
    tags: Optional[List[str]] = record.tags
    dates: Optional[List[dt.datetime]] = record.dates
    moods: Optional[List[Mood]] = record.moods
    addresses: Optional[List[Address]] = record.addresses
    attachment: Optional[FileRef] = record.attachment
    assert id_ and sku and tags is not None
    assert dates is not None and moods is not None and addresses is not None
    assert attachment is None
    return record.to_row()


def enum_carries_its_wire_value() -> str:
    """A generated enum is a str enum, so it compares and serialises as text."""
    value: str = Mood.CALM.value
    assert Mood("calm") is Mood.CALM
    return value


def table_declaration_is_typed() -> None:
    """The table is declared with its kind, key and row class."""
    table: Table[Inventory, Any, Any] = inventory
    assert table.table_id.endswith("inventory")
    assert table.kind is TableKind.UNSPECIFIED
    assert table.key_columns == ("sku",)
    assert table.row is Inventory
    registry: Dict[str, Table[Any, Any, Any]] = KALAM_TABLES
    assert table.table_id in registry


def every_table_kind_is_declared() -> None:
    """USER, SHARED and STREAM each reach the right factory and row class."""
    user: Table[Profiles, Any, Any] = profiles
    shared: Table[Rooms, Any, Any] = rooms
    stream: Table[Events, Any, Any] = events
    assert user.kind is TableKind.USER
    assert shared.kind is TableKind.SHARED
    assert stream.kind is TableKind.STREAM
    assert user.row is Profiles and shared.row is Rooms and stream.row is Events
    assert user.key_columns == ("id",)
    assert shared.key_columns == ("id",)
    assert stream.key_columns == ("seq",)
    for table in (user, shared, stream):
        assert table.table_id in KALAM_TABLES


def columns_are_declared_objects() -> None:
    """Generated columns expose the metadata the runtime builds SQL from."""
    sku: Column[str] = InventoryColumns.sku
    assert sku.name == "sku"
    assert sku.is_primary_key
    assert InventoryColumns.id.is_not_null
    assert not InventoryColumns.tags.is_not_null
    # A server-filled column is optional on insert and never emitted unasked.
    assert ProfilesColumns.id.has_server_default
    assert ProfilesColumns.seen_at.has_server_default


def temporal_columns_keep_their_python_types() -> None:
    """DATE, TIME and BYTES decode to the stdlib types, not to strings."""
    born_on: Column[dt.date] = ProfilesColumns.born_on
    wakes_at: Column[dt.time] = ProfilesColumns.wakes_at
    avatar: Column[bytes] = ProfilesColumns.avatar
    assert (born_on.sql_type, wakes_at.sql_type, avatar.sql_type) == (
        "DATE",
        "TIME",
        "BYTES",
    )
    # The wire forms the server actually sends: Date32 days, microseconds since
    # midnight, and an array of byte values.
    row = Profiles.from_row(
        {
            "id": "1",
            "email": "a@b.c",
            "born_on": 20454,
            "wakes_at": 45_296_000_000,
            "avatar": [1, 2, 3],
            "seen_at": 86_400_000_000,
        }
    )
    assert row.born_on == dt.date(2026, 1, 1)
    assert row.wakes_at == dt.time(12, 34, 56)
    assert row.avatar == b"\x01\x02\x03"
    # A microsecond timestamp close to the epoch stays in 1970.
    assert row.seen_at == dt.datetime(1970, 1, 2, tzinfo=dt.timezone.utc)


def builds_parameterised_sql() -> None:
    """The runtime builds SQL and parameters, never interpolated values."""
    sql, params = inventory.insert_sql({"id": "1", "sku": "sku-1"})
    assert sql.startswith("INSERT INTO ")
    assert params == ["1", "sku-1"]

    select, select_params = inventory.select_by_key_sql("sku-1")
    assert "$1" in select
    assert select_params == ["sku-1"]


def typed_writes_encode_their_values() -> None:
    """Declared Python types are encoded on the way out, not sent raw."""
    sql, params = profiles.insert_sql(
        {"email": "a@b.c", "born_on": dt.date(2026, 1, 1)}
    )
    assert "$1" in sql and "$2" in sql
    assert params == ["a@b.c", "2026-01-01"]

    _, update_params = rooms.update_sql(1, {"mood": Mood.BUSY})
    assert update_params == ["busy", 1]


def array_parameters_are_refused_before_they_are_sent() -> None:
    """The server binds scalars only, so an array is rejected client-side."""
    try:
        inventory.insert_sql({"id": "1", "sku": "sku-1", "tags": ["a"]})
    except UnsupportedParameterError as error:
        assert "tags" in str(error)
    else:  # pragma: no cover - the raise above is the contract
        raise AssertionError("an array parameter should have been refused")


def main() -> None:
    """Run every check, so the harness exercises them and not just mypy."""
    row: Dict[str, Any] = {
        "id": "1",
        "sku": "sku-1",
        "tags": ["a"],
        "dates": ["2026-01-01T00:00:00Z"],
        "moods": ["calm"],
        "addresses": [{"city": "Jammu"}],
        "attachment": None,
    }
    decoded = decodes_a_wire_row(row)
    assert decoded.sku == "sku-1"
    assert decoded.moods == [Mood.CALM]
    assert decoded.addresses == [Address(city="Jammu")]
    assert decoded.dates == [dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc)]

    assert declared_field_types()["sku"] == "sku-1"
    assert enum_carries_its_wire_value() == "calm"
    table_declaration_is_typed()
    every_table_kind_is_declared()
    columns_are_declared_objects()
    temporal_columns_keep_their_python_types()
    builds_parameterised_sql()
    typed_writes_encode_their_values()
    array_parameters_are_refused_before_they_are_sent()


if __name__ == "__main__":
    main()
