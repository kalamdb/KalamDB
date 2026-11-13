"""Table definitions for the KalamDB Python SDK."""

from __future__ import annotations

import re
from enum import Enum
from typing import Any, Dict, Iterator, List, Mapping, Tuple

__all__ = [
    "TableKind",
    "Column",
    "Table",
    "KTable",
    "COMPRESSION_CODECS",
    "EVICTION_STRATEGIES",
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


COMPRESSION_CODECS = ("none", "snappy", "zstd")
EVICTION_STRATEGIES = ("time_based", "size_based", "hybrid")

_STORAGE_ID_RE = re.compile(r"^[A-Za-z0-9_-]+$")
_OPTION_ORDER = (
    "STORAGE_ID",
    "USE_USER_STORAGE",
    "FLUSH_POLICY",
    "COMPRESSION",
    "DELETED_RETENTION_HOURS",
    "TTL_SECONDS",
    "EVICTION_STRATEGY",
    "MAX_STREAM_SIZE_BYTES",
)


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
        self.options: Dict[str, str] = {}

    def column(self, name: str) -> Column:
        for col in self.columns:
            if col.name == name:
                return col
        raise KeyError(f"{self.qualified_name} has no column {name!r}")

    def storage_id(self, storage_id: str) -> "Table":
        """Named storage backend holding this table's segments."""
        self._require_kind("STORAGE_ID", TableKind.USER, TableKind.SHARED)
        if not _STORAGE_ID_RE.match(storage_id):
            raise ValueError(
                f"invalid storage id {storage_id!r}: letters, digits, underscore and "
                "hyphen only"
            )
        return self._set("STORAGE_ID", f"'{storage_id}'")

    def use_user_storage(self, enabled: bool = True) -> "Table":
        """Write each user's rows to that user's own storage backend."""
        self._require_kind("USE_USER_STORAGE", TableKind.USER)
        return self._set("USE_USER_STORAGE", "true" if enabled else "false")

    def flush_policy(self, rows: int = 0, interval_seconds: int = 0) -> "Table":
        """When buffered rows move to cold storage. Set either bound or both."""
        self._require_kind("FLUSH_POLICY", TableKind.USER, TableKind.SHARED)
        if rows < 0 or interval_seconds < 0:
            raise ValueError("flush policy bounds must not be negative")
        parts = []
        if rows:
            parts.append(f"rows:{rows}")
        if interval_seconds:
            parts.append(f"interval:{interval_seconds}")
        if not parts:
            raise ValueError("flush policy needs rows or interval_seconds above zero")
        return self._set("FLUSH_POLICY", f"'{','.join(parts)}'")

    def compression(self, codec: str) -> "Table":
        """Parquet compression codec for cold segments."""
        self._require_kind("COMPRESSION", TableKind.USER, TableKind.SHARED)
        normalized = codec.strip().lower()
        if normalized not in COMPRESSION_CODECS:
            raise ValueError(
                f"invalid compression {codec!r}: expected one of "
                f"{', '.join(COMPRESSION_CODECS)}"
            )
        return self._set("COMPRESSION", f"'{normalized}'")

    def deleted_retention_hours(self, hours: int) -> "Table":
        """How long deleted rows stay readable before cleanup drops them."""
        self._require_kind(
            "DELETED_RETENTION_HOURS", TableKind.USER, TableKind.SHARED, TableKind.STREAM
        )
        if hours < 0:
            raise ValueError("deleted retention hours must not be negative")
        return self._set("DELETED_RETENTION_HOURS", str(hours))

    def ttl_seconds(self, seconds: int) -> "Table":
        """Row lifetime. Required on every STREAM table."""
        self._require_kind("TTL_SECONDS", TableKind.STREAM)
        if seconds <= 0:
            raise ValueError("ttl seconds must be positive")
        return self._set("TTL_SECONDS", str(seconds))

    def eviction_strategy(self, strategy: str) -> "Table":
        """Which bound expires stream rows: age, size, or both."""
        self._require_kind("EVICTION_STRATEGY", TableKind.STREAM)
        normalized = strategy.strip().lower()
        if normalized not in EVICTION_STRATEGIES:
            raise ValueError(
                f"invalid eviction strategy {strategy!r}: expected one of "
                f"{', '.join(EVICTION_STRATEGIES)}"
            )
        return self._set("EVICTION_STRATEGY", f"'{normalized}'")

    def max_stream_size_bytes(self, size: int) -> "Table":
        """Size bound used by the size_based and hybrid eviction strategies."""
        self._require_kind("MAX_STREAM_SIZE_BYTES", TableKind.STREAM)
        if size <= 0:
            raise ValueError("max stream size bytes must be positive")
        return self._set("MAX_STREAM_SIZE_BYTES", str(size))

    def create_table_sql(self, if_not_exists: bool = True) -> str:
        guard = "IF NOT EXISTS " if if_not_exists else ""
        body = ",\n  ".join(col.ddl() for col in self.columns)
        sql = f"CREATE TABLE {guard}{self.qualified_name} (\n  {body}\n)"
        options = self.options_sql()
        return f"{sql}\nWITH ({options})" if options else sql

    def options_sql(self) -> str:
        """The WITH clause body. Empty for SYSTEM tables, which take no options."""
        if self.kind is TableKind.SYSTEM:
            return ""
        if self.kind is TableKind.STREAM and "TTL_SECONDS" not in self.options:
            raise ValueError(
                f"{self.qualified_name} is a STREAM table, so it needs ttl_seconds(...)"
            )
        rendered = [f"TYPE = '{self.kind.value}'"]
        rendered.extend(
            f"{name} = {self.options[name]}" for name in _OPTION_ORDER if name in self.options
        )
        return ", ".join(rendered)

    def _set(self, option: str, literal: str) -> "Table":
        self.options[option] = literal
        return self

    def _require_kind(self, option: str, *kinds: TableKind) -> None:
        if self.kind not in kinds:
            allowed = " and ".join(kind.value for kind in kinds)
            raise ValueError(
                f"{option} is only supported for {allowed} tables, and "
                f"{self.qualified_name} is {self.kind.value}"
            )

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
