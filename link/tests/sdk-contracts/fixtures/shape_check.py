"""Shape checks for generated Python bindings, verified with mypy --strict.

Mirrors fixtures/shape-check.ts and fixtures/generated_test.dart: it asserts the
generated row dataclass, enum, composite and column declarations carry the types
the contract promises. The negative boundary is checked separately, by
check_generated.py writing a file mypy must reject.
"""

from __future__ import annotations

import datetime as dt
from typing import Any, Dict, List, Mapping, Optional

from kalamdb.schema import Column, FileRef, Table, TableKind

from kalam import KALAM_TABLES, Address, Inventory, InventoryColumns, Mood, inventory


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
    table: Table[Inventory] = inventory
    assert table.table_id.endswith("inventory")
    assert table.kind is TableKind.UNSPECIFIED
    assert table.key_columns == ("sku",)
    assert table.row is Inventory
    registry: Dict[str, Table[Any]] = KALAM_TABLES
    assert table.table_id in registry


def columns_are_declared_objects() -> None:
    """Generated columns expose the metadata the runtime builds SQL from."""
    sku: Column[str] = InventoryColumns.sku
    assert sku.name == "sku"
    assert sku.is_primary_key
    assert InventoryColumns.id.is_not_null
    assert not InventoryColumns.tags.is_not_null


def builds_parameterised_sql() -> None:
    """The runtime builds SQL and parameters, never interpolated values."""
    sql, params = inventory.insert_sql({"id": "1", "sku": "sku-1"})
    assert sql.startswith("INSERT INTO ")
    assert params == ["1", "sku-1"]

    select, select_params = inventory.select_by_key_sql("sku-1")
    assert "$1" in select
    assert select_params == ["sku-1"]
