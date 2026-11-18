"""Factories generated code calls to declare a table, one per table kind."""

from __future__ import annotations

from typing import Any, List, Mapping, Optional, Type, TypeVar

from .column import Column
from .table import Table
from .types import TableKind

__all__ = ["KTable"]

Row = TypeVar("Row")


def _resolve_columns(table_id: str, columns: Any) -> List[Column[Any]]:
    """Collect the columns of a generated columns class or a plain mapping.

    A class is read in definition order, so the declaration order in
    schema.sql survives into the generated table.

    Only dunder attributes are skipped. A single leading underscore is a real
    column name here -- ``_version``, ``_deleted`` and ``_timestamp`` are
    KalamDB system columns -- so filtering those out would silently lose them.
    """
    if isinstance(columns, Mapping):
        entries = list(columns.items())
    elif isinstance(columns, type):
        entries = [
            (attr, value)
            for attr, value in vars(columns).items()
            if not (attr.startswith("__") and attr.endswith("__"))
        ]
    else:
        raise TypeError(
            f"{table_id}: columns must be a generated columns class or a mapping, "
            f"got {type(columns).__name__}"
        )

    resolved: List[Column[Any]] = []
    for attr, value in entries:
        if not isinstance(value, Column):
            raise TypeError(f"{table_id}: {attr!r} is not a Column")
        resolved.append(value)

    if not resolved:
        raise ValueError(f"table {table_id!r} has no columns")
    return resolved


class KTable:
    """Declare a table. The factory name carries the table's kind, the same way
    TypeScript's ``kTable.user`` / ``kTable.shared`` / ``kTable.stream`` do."""

    @staticmethod
    def _build(
        table_id: str,
        kind: TableKind,
        columns: Any,
        row: Optional[Type[Row]],
    ) -> Table[Row]:
        return Table(
            table_id,
            kind,
            _resolve_columns(table_id, columns),
            row=row,
            columns_class=columns if isinstance(columns, type) else None,
        )

    @staticmethod
    def table(table_id: str, columns: Any, *, row: Optional[Type[Row]] = None) -> Table[Row]:
        """A table declared without a kind. The server treats it as shared."""
        return KTable._build(table_id, TableKind.UNSPECIFIED, columns, row)

    @staticmethod
    def user(table_id: str, columns: Any, *, row: Optional[Type[Row]] = None) -> Table[Row]:
        return KTable._build(table_id, TableKind.USER, columns, row)

    @staticmethod
    def shared(table_id: str, columns: Any, *, row: Optional[Type[Row]] = None) -> Table[Row]:
        return KTable._build(table_id, TableKind.SHARED, columns, row)

    @staticmethod
    def stream(table_id: str, columns: Any, *, row: Optional[Type[Row]] = None) -> Table[Row]:
        return KTable._build(table_id, TableKind.STREAM, columns, row)
