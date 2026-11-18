"""Column declarations and the factories generated code calls."""

from __future__ import annotations

import datetime as _dt
from typing import TYPE_CHECKING, Any, Generic, List, TypeVar

if TYPE_CHECKING:  # pragma: no cover - import only for annotations
    from .._native import FileRef

__all__ = [
    "Column",
    "bigint",
    "boolean",
    "bytes_",
    "date",
    "double",
    "embedding",
    "file",
    "integer",
    "json",
    "smallint",
    "text",
    "time",
    "timestamp",
    "uuid",
]

#: Maximum vector width the server accepts for an EMBEDDING column.
MAX_EMBEDDING_DIMENSIONS = 8192

T = TypeVar("T")


class Column(Generic[T]):
    """One column of a table. Modifier methods return self so they chain.

    ``T`` is the Python type the column decodes to, so generated annotations
    carry real types through to the caller.
    """

    def __init__(self, name: str, sql_type: str) -> None:
        if not name:
            raise ValueError("column name must not be empty")
        self.name = name
        self.sql_type = sql_type
        self.is_primary_key = False
        self.pk_ordinal = 0
        self.is_not_null = False
        self.has_server_default = False
        self.is_read_only = False

    def primary_key(self, ordinal: int = 0) -> "Column[T]":
        """Mark this column part of the primary key. ``ordinal`` orders the
        parts of a composite key."""
        if ordinal < 0:
            raise ValueError("primary key ordinal must not be negative")
        self.is_primary_key = True
        self.pk_ordinal = ordinal
        return self

    def not_null(self) -> "Column[T]":
        self.is_not_null = True
        return self

    def server_default(self) -> "Column[T]":
        """The server fills this column, so it is optional on insert and is
        left out of generated INSERTs unless supplied explicitly."""
        self.has_server_default = True
        return self

    def read_only(self) -> "Column[T]":
        """Generated or identity column: never written by the client."""
        self.is_read_only = True
        return self

    def __repr__(self) -> str:
        return f"Column({self.name!r}, {self.sql_type!r})"


def bigint(name: str) -> "Column[int]":
    return Column(name, "BIGINT")


def integer(name: str) -> "Column[int]":
    return Column(name, "INT")


def smallint(name: str) -> "Column[int]":
    return Column(name, "SMALLINT")


def double(name: str) -> "Column[float]":
    return Column(name, "DOUBLE")


def boolean(name: str) -> "Column[bool]":
    return Column(name, "BOOLEAN")


def text(name: str) -> "Column[str]":
    return Column(name, "TEXT")


def uuid(name: str) -> "Column[str]":
    return Column(name, "UUID")


def bytes_(name: str) -> "Column[bytes]":
    return Column(name, "BYTES")


def json(name: str) -> "Column[Any]":
    return Column(name, "JSON")


def timestamp(name: str) -> "Column[_dt.datetime]":
    return Column(name, "TIMESTAMP")


def date(name: str) -> "Column[_dt.date]":
    return Column(name, "DATE")


def time(name: str) -> "Column[_dt.time]":
    return Column(name, "TIME")


def file(name: str) -> "Column[FileRef]":
    return Column(name, "FILE")


def embedding(name: str, dimensions: int) -> "Column[List[float]]":
    """Vector column; ``dimensions`` must match the producing model."""
    if dimensions < 1 or dimensions > MAX_EMBEDDING_DIMENSIONS:
        raise ValueError(
            f"embedding dimensions must be between 1 and {MAX_EMBEDDING_DIMENSIONS}, "
            f"got {dimensions}"
        )
    return Column(name, f"EMBEDDING({dimensions})")
