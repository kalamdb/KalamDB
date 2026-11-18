"""Runtime for generated KalamDB schema bindings.

Tables are declared in ``schema.sql`` and the typed bindings are written by
``kalam schema gen --languages python``. This package holds the hand-written
runtime those bindings call into: columns, tables, and the SQL they build.
"""

from __future__ import annotations

# Re-exported so generated bindings annotate FILE columns from one import line.
from .._native import FileRef
from .codecs import (
    as_bool,
    as_bool_or_none,
    as_bytes,
    as_bytes_or_none,
    as_date,
    as_date_or_none,
    as_datetime,
    as_datetime_or_none,
    as_enum,
    as_enum_or_none,
    as_file_ref,
    as_file_ref_or_none,
    as_float,
    as_float_list,
    as_float_or_none,
    as_int,
    as_int_or_none,
    as_json,
    as_list,
    as_list_or_none,
    as_str,
    as_str_or_none,
    as_struct,
    as_struct_or_none,
    as_time,
    as_time_or_none,
    encode_date,
    encode_datetime,
    encode_enum,
    encode_file,
    encode_list,
    encode_row,
    encode_time,
)
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
    "FileRef",
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
    # decoders
    "as_bool",
    "as_bool_or_none",
    "as_bytes",
    "as_bytes_or_none",
    "as_date",
    "as_date_or_none",
    "as_datetime",
    "as_datetime_or_none",
    "as_enum",
    "as_enum_or_none",
    "as_file_ref",
    "as_file_ref_or_none",
    "as_float",
    "as_float_list",
    "as_float_or_none",
    "as_int",
    "as_int_or_none",
    "as_json",
    "as_list",
    "as_list_or_none",
    "as_str",
    "as_str_or_none",
    "as_struct",
    "as_struct_or_none",
    "as_time",
    "as_time_or_none",
    # encoders
    "encode_date",
    "encode_datetime",
    "encode_enum",
    "encode_file",
    "encode_list",
    "encode_row",
    "encode_time",
]
