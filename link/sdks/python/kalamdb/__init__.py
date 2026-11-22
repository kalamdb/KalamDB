"""KalamDB Python SDK.

A SQL-first realtime database client for AI agents and multi-tenant SaaS.
"""

from ._native import (
    Auth,
    FileDownload,
    FileRef,
    FileUpload,
    KalamClient,
    LiveEvents,
    LiveRows,
    Consumer,
    KalamError,
    KalamConnectionError,
    KalamAuthError,
    KalamServerError,
    KalamConfigError,
)
from .agent import run_agent

# The table runtime lives in `kalamdb.schema`, which is what generated bindings
# import. Only the declaration types are re-exported here; the column factories
# and codecs stay in `kalamdb.schema` so they do not crowd the top-level
# namespace (the same split as @kalamdb/orm beside @kalamdb/client).
from .schema import Column, KTable, Table, TableKind, UnsupportedParameterError

__all__ = [
    "Auth",
    "FileDownload",
    "FileRef",
    "FileUpload",
    "KalamClient",
    "LiveEvents",
    "LiveRows",
    "Consumer",
    "KalamError",
    "KalamConnectionError",
    "KalamAuthError",
    "KalamServerError",
    "KalamConfigError",
    "run_agent",
    "Column",
    "KTable",
    "Table",
    "TableKind",
    "UnsupportedParameterError",
]
