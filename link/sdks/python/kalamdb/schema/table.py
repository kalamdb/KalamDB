"""A declared table and the SQL it builds.

Every method here is pure: it returns a SQL string plus its parameters and
touches no connection, so callers pass the pair to ``KalamClient.query``.
"""

from __future__ import annotations

from typing import (
    Any,
    Dict,
    Generic,
    Iterator,
    List,
    Mapping,
    Optional,
    Sequence,
    Tuple,
    Type,
    TypeVar,
    cast,
)

from .column import Column
from .identifiers import quote_ident, quote_table
from .types import TableKind

__all__ = ["Table"]

Row = TypeVar("Row")


class Table(Generic[Row]):
    """One table from schema.sql. Build one through :class:`~kalamdb.schema.KTable`."""

    def __init__(
        self,
        table_id: str,
        kind: TableKind,
        columns: Sequence[Column[Any]],
        row: Optional[Type[Row]] = None,
        columns_class: Optional[type] = None,
    ) -> None:
        if not columns:
            raise ValueError(f"table {table_id!r} has no columns")

        namespace, _, name = table_id.rpartition(".")
        if not name:
            raise ValueError(f"invalid table name: {table_id!r}")

        seen = set()
        for col in columns:
            if col.name in seen:
                raise ValueError(f"table {table_id!r} declares {col.name!r} twice")
            seen.add(col.name)

        self.table_id = table_id
        self.namespace = namespace or None
        self.name = name
        self.kind = kind
        self.columns: Tuple[Column[Any], ...] = tuple(columns)
        self.row = row
        #: The generated columns class, kept for discoverability from the table.
        self.c = columns_class

    @property
    def key_columns(self) -> Tuple[str, ...]:
        """Primary key column names, in key order.

        Derived on access rather than cached: a caller holding a reference to a
        Column can still mark it a key after the table is built, and a stale
        tuple would disagree with `column()`.
        """
        keys = [
            (column.pk_ordinal, index, column.name)
            for index, column in enumerate(self.columns)
            if column.is_primary_key
        ]
        keys.sort()
        return tuple(name for _, _, name in keys)

    # -- lookup ---------------------------------------------------------------

    def column(self, name: str) -> Column[Any]:
        for col in self.columns:
            if col.name == name:
                return col
        raise KeyError(f"{self.table_id} has no column {name!r}")

    def __iter__(self) -> Iterator[Column[Any]]:
        return iter(self.columns)

    def __repr__(self) -> str:
        return (
            f"Table({self.table_id!r}, kind={self.kind.value}, "
            f"columns={len(self.columns)})"
        )

    # -- decoding -------------------------------------------------------------

    def decode(self, row: Mapping[str, Any]) -> Row:
        """Turn one wire row into this table's row class."""
        if self.row is None:
            raise TypeError(
                f"{self.table_id} was declared without a row class, so rows cannot "
                "be decoded; use the mapping from query_rows directly"
            )
        # Generated row classes provide from_row; cast keeps that visible.
        return cast(Row, self.row.from_row(row))  # type: ignore[attr-defined]

    # -- writes ---------------------------------------------------------------

    def insert_sql(self, values: Mapping[str, Any]) -> Tuple[str, List[Any]]:
        """Parameterised INSERT, leaving out server-filled columns."""
        self._reject_unknown(values)

        names: List[str] = []
        params: List[Any] = []
        for col in self.columns:
            if col.name in values:
                if col.is_read_only:
                    raise ValueError(
                        f"{self.table_id}.{col.name} is read-only and is filled by "
                        "the server, so it must not be supplied"
                    )
                names.append(quote_ident(col.name))
                params.append(values[col.name])
            elif col.has_server_default or col.is_read_only:
                continue
            elif col.is_not_null:
                raise ValueError(
                    f"{self.table_id}.{col.name} is NOT NULL with no server "
                    "default, so it must be supplied"
                )

        if not names:
            # The dialect has no INSERT ... DEFAULT VALUES form, so there is no
            # statement to emit for an all-defaults insert.
            raise ValueError(
                f"insert into {self.table_id} supplied no columns, and the server "
                "has no DEFAULT VALUES form; supply at least one column"
            )

        placeholders = ", ".join(f"${i}" for i in range(1, len(params) + 1))
        sql = (
            f"INSERT INTO {quote_table(self.table_id)} "
            f"({', '.join(names)}) VALUES ({placeholders})"
        )
        return sql, params

    def update_sql(self, key: Any, patch: Mapping[str, Any]) -> Tuple[str, List[Any]]:
        """Parameterised UPDATE of one row, addressed by primary key."""
        self._reject_unknown(patch)
        if not patch:
            raise ValueError(f"update of {self.table_id} supplied no columns")

        assignments: List[str] = []
        params: List[Any] = []
        for col in self.columns:
            if col.name not in patch:
                continue
            if col.is_read_only:
                raise ValueError(
                    f"{self.table_id}.{col.name} is read-only, so it cannot be updated"
                )
            if col.name in self.key_columns:
                raise ValueError(
                    f"{self.table_id}.{col.name} is part of the primary key, so it "
                    "cannot be updated"
                )
            params.append(patch[col.name])
            assignments.append(f"{quote_ident(col.name)} = ${len(params)}")

        if not assignments:
            raise ValueError(f"update of {self.table_id} supplied no writable columns")

        where = self._key_predicate(key, params)
        sql = (
            f"UPDATE {quote_table(self.table_id)} SET {', '.join(assignments)} "
            f"WHERE {where}"
        )
        return sql, params

    def delete_sql(self, key: Any) -> Tuple[str, List[Any]]:
        """Parameterised DELETE of one row, addressed by primary key."""
        params: List[Any] = []
        where = self._key_predicate(key, params)
        return f"DELETE FROM {quote_table(self.table_id)} WHERE {where}", params

    # -- reads ----------------------------------------------------------------

    def select_sql(
        self,
        where: Optional[str] = None,
        params: Optional[Sequence[Any]] = None,
    ) -> Tuple[str, List[Any]]:
        """SELECT every column, optionally filtered.

        ``where`` is a SQL fragment whose values are supplied as ``params``
        (``"email = $1"``, ``["a@b.c"]``), so values are never interpolated
        into the statement.
        """
        bound = list(params or [])
        if params and not where:
            raise ValueError("select params were given without a where fragment")

        sql = f"SELECT * FROM {quote_table(self.table_id)}"
        if where:
            sql = f"{sql} WHERE {where}"
        return sql, bound

    def select_by_key_sql(self, key: Any) -> Tuple[str, List[Any]]:
        """SELECT the single row with this primary key."""
        params: List[Any] = []
        where = self._key_predicate(key, params)
        return f"SELECT * FROM {quote_table(self.table_id)} WHERE {where}", params

    # -- internals ------------------------------------------------------------

    def _reject_unknown(self, values: Mapping[str, Any]) -> None:
        unknown = set(values) - {col.name for col in self.columns}
        if unknown:
            raise KeyError(f"{self.table_id} has no column(s): {sorted(unknown)}")

    def _key_predicate(self, key: Any, params: List[Any]) -> str:
        """Append key values to ``params`` and return the WHERE fragment."""
        # Read the key order once. It is derived on access, and the branches
        # below run caller code, so re-reading could collect the values in one
        # order and bind them in another.
        key_columns = self.key_columns
        if not key_columns:
            raise ValueError(
                f"{self.table_id} has no primary key, so rows cannot be addressed "
                "by key; use select_sql with an explicit where fragment"
            )

        if len(key_columns) == 1:
            values: Sequence[Any] = [key]
        elif isinstance(key, Mapping):
            missing = [name for name in key_columns if name not in key]
            if missing:
                raise ValueError(f"{self.table_id} key is missing {missing}")
            values = [key[name] for name in key_columns]
        elif isinstance(key, Sequence) and not isinstance(key, (str, bytes)):
            # Width first: a wrong-length lazy sequence such as range(1_000_000_000)
            # would otherwise be materialised in full only to be rejected.
            self._require_key_width(len(key), key_columns)
            values = list(key)
            # The sequence may have resized while it was being copied.
            self._require_key_width(len(values), key_columns)
        else:
            raise ValueError(
                f"{self.table_id} has a composite key {key_columns}, so pass a "
                "mapping or a sequence of values"
            )

        parts = []
        for name, value in zip(key_columns, values):
            params.append(value)
            parts.append(f"{quote_ident(name)} = ${len(params)}")
        return " AND ".join(parts)

    def _require_key_width(self, width: int, key_columns: Tuple[str, ...]) -> None:
        """Reject a composite key that does not cover every key column."""
        if width != len(key_columns):
            raise ValueError(
                f"{self.table_id} has a composite key of "
                f"{len(key_columns)} columns, got {width} values"
            )
