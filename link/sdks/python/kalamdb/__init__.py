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
]
