"""Table definitions for the KalamDB Python SDK."""

from __future__ import annotations

from enum import Enum
from typing import Any, Dict, Iterator, List, Mapping, Tuple

__all__ = [
    "TableKind",
    "Column",
    "Table",
    "KTable",
    "bigint",
    "boolean",
    "bytes_",
    "date",
    "double",
    "embedding",
    "file",
    "integer",
    "json",
    "text",
    "time",
    "timestamp",
    "uuid",
]


class TableKind(Enum):
    """USER rows are partitioned per user, SHARED rows share row-level
    security, STREAM rows are append-only events that expire."""

    USER = "USER"
    SHARED = "SHARED"
    STREAM = "STREAM"
    SYSTEM = "SYSTEM"


class Column:
    """One column. Modifier methods return self so they chain."""

    def __init__(self, name: str, sql_type: str) -> None:
        if not name:
            raise ValueError("column name must not be empty")
        self.name = name
        self.sql_type = sql_type
        self.is_primary_key = False
        self.is_not_null = False
        self.has_server_default = False
        self.default_expr: str | None = None

    def primary_key(self) -> "Column":
        self.is_primary_key = True
        return self

    def not_null(self) -> "Column":
        self.is_not_null = True
        return self

    def server_default(self, expr: str | None = None) -> "Column":
        """Let the server fill this column: it becomes optional on insert and
        is dropped from generated INSERTs. Pass ``expr`` ("SNOWFLAKE_ID()",
        "NOW()") to emit it in the DDL too."""
        self.has_server_default = True
        self.default_expr = expr
        return self

    def ddl(self) -> str:
        parts = [self.name, self.sql_type]
        if self.is_not_null:
            parts.append("NOT NULL")
        if self.default_expr:
            parts.append(f"DEFAULT {self.default_expr}")
        if self.is_primary_key:
            parts.append("PRIMARY KEY")
        return " ".join(parts)

    def __repr__(self) -> str:
        return f"Column({self.name!r}, {self.sql_type!r})"


def _column(sql_type: str):
    def factory(name: str) -> Column:
        return Column(name, sql_type)

    return factory


bigint = _column("BIGINT")
boolean = _column("BOOLEAN")
bytes_ = _column("BYTES")
date = _column("DATE")
double = _column("DOUBLE")
file = _column("FILE")
integer = _column("INT")
json = _column("JSON")
text = _column("TEXT")
time = _column("TIME")
timestamp = _column("TIMESTAMP")
uuid = _column("UUID")


def embedding(name: str, dimensions: int) -> Column:
    """Vector column; ``dimensions`` must match the producing model."""
    if dimensions <= 0:
        raise ValueError("embedding dimensions must be positive")
    return Column(name, f"EMBEDDING({dimensions})")


class Table:
    """A declared table. Build one through :class:`KTable`."""

    def __init__(self, qualified_name: str, kind: TableKind, columns: List[Column]) -> None:
        if not columns:
            raise ValueError(f"table {qualified_name!r} has no columns")

        namespace, _, name = qualified_name.rpartition(".")
        if not name:
            raise ValueError(f"invalid table name: {qualified_name!r}")

        self.namespace = namespace or None
        self.name = name
        self.qualified_name = qualified_name
        self.kind = kind
        self.columns = columns

    def column(self, name: str) -> Column:
        for col in self.columns:
            if col.name == name:
                return col
        raise KeyError(f"{self.qualified_name} has no column {name!r}")

    def create_table_sql(self, if_not_exists: bool = True) -> str:
        kind = "" if self.kind is TableKind.SYSTEM else f"{self.kind.value} "
        guard = "IF NOT EXISTS " if if_not_exists else ""
        body = ",\n  ".join(col.ddl() for col in self.columns)
        return f"CREATE {kind}TABLE {guard}{self.qualified_name} (\n  {body}\n)"

    def insert_sql(self, data: Mapping[str, Any]) -> Tuple[str, List[Any]]:
        """Parameterised INSERT, skipping server-filled columns."""
        unknown = set(data) - {col.name for col in self.columns}
        if unknown:
            raise KeyError(f"{self.qualified_name} has no column(s): {sorted(unknown)}")

        names: List[str] = []
        params: List[Any] = []
        for col in self.columns:
            if col.name in data:
                names.append(col.name)
                params.append(data[col.name])
            elif col.has_server_default:
                continue
            elif col.is_not_null:
                raise ValueError(
                    f"{self.qualified_name}.{col.name} is NOT NULL with no server "
                    "default, so it must be supplied"
                )

        placeholders = ", ".join(f"${i}" for i in range(1, len(params) + 1))
        return (
            f"INSERT INTO {self.qualified_name} ({', '.join(names)}) VALUES ({placeholders})",
            params,
        )

    def select_sql(self, where: str = "") -> str:
        sql = f"SELECT * FROM {self.qualified_name}"
        return f"{sql} WHERE {where}" if where else sql

    def __iter__(self) -> Iterator[Column]:
        return iter(self.columns)

    def __repr__(self) -> str:
        return f"Table({self.qualified_name!r}, kind={self.kind.value}, columns={len(self.columns)})"


class KTable:
    """Factories for each table kind."""

    @staticmethod
    def _build(qualified_name: str, kind: TableKind, columns: Dict[str, Column]) -> Table:
        resolved = []
        for attr, col in columns.items():
            if not isinstance(col, Column):
                raise TypeError(f"{attr!r} is not a Column")
            resolved.append(col)
        return Table(qualified_name, kind, resolved)

    @staticmethod
    def user(qualified_name: str, **columns: Column) -> Table:
        return KTable._build(qualified_name, TableKind.USER, columns)

    @staticmethod
    def shared(qualified_name: str, **columns: Column) -> Table:
        return KTable._build(qualified_name, TableKind.SHARED, columns)

    @staticmethod
    def stream(qualified_name: str, **columns: Column) -> Table:
        return KTable._build(qualified_name, TableKind.STREAM, columns)

    @staticmethod
    def system(qualified_name: str, **columns: Column) -> Table:
        return KTable._build(qualified_name, TableKind.SYSTEM, columns)
