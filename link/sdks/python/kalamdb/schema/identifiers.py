"""SQL identifier quoting, so table and column names are never interpolated raw."""

from __future__ import annotations

__all__ = ["quote_ident", "quote_table"]


def quote_ident(ident: str) -> str:
    """Quote a single identifier for KalamDB SQL."""
    if not ident:
        raise ValueError("identifier must not be empty")
    if "\x00" in ident:
        raise ValueError(f"identifier must not contain a NUL byte: {ident!r}")
    escaped = ident.replace('"', '""')
    return f'"{escaped}"'


def quote_table(table_id: str) -> str:
    """Quote a ``schema.table`` identifier, each part separately.

    An unqualified name is quoted as a single identifier.
    """
    if not table_id:
        raise ValueError("table id must not be empty")
    namespace, separator, name = table_id.rpartition(".")
    if not name:
        raise ValueError(f"invalid table id: {table_id!r}")
    if not separator:
        return quote_ident(name)
    # A separator with nothing before it (".notes") is malformed, not unqualified.
    if not namespace:
        raise ValueError(f"table id {table_id!r} has an empty namespace")
    return f"{quote_ident(namespace)}.{quote_ident(name)}"
