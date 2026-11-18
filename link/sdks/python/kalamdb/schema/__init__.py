"""Runtime for generated KalamDB schema bindings.

Tables are declared in ``schema.sql`` and the typed bindings are written by
``kalam schema gen --languages python``. This package holds the hand-written
runtime those bindings call into: columns, tables, and the SQL they build.
"""

from __future__ import annotations

from .column import (
    MAX_EMBEDDING_DIMENSIONS,
    Column,
    bigint,
    boolean,
    bytes_,
    date,
    double,
    embedding,
    file,
    integer,
    json,
    smallint,
    text,
    time,
    timestamp,
    uuid,
)
from .identifiers import quote_ident, quote_table
from .ktable import KTable
from .table import Table
from .types import TableKind

__all__ = [
    # declarations
    "Column",
    "KTable",
    "Table",
    "TableKind",
    "MAX_EMBEDDING_DIMENSIONS",
    # column factories
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
    # identifiers
    "quote_ident",
    "quote_table",
]
