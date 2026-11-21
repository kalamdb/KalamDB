"""Table kinds declared in schema.sql."""

from __future__ import annotations

from enum import Enum

__all__ = ["TableKind"]


class TableKind(Enum):
    """How the server partitions a table's rows.

    USER rows are partitioned per user, SHARED rows share row-level security,
    STREAM rows are append-only events that expire. UNSPECIFIED is a table
    declared without a kind, which the server treats as shared.
    """

    UNSPECIFIED = "UNSPECIFIED"
    USER = "USER"
    SHARED = "SHARED"
    STREAM = "STREAM"
