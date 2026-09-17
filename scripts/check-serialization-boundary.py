#!/usr/bin/env python3
"""Report direct persistence-codec usage outside kalamdb-serialization.

`--fail` exits 1 when FlatBuffers, FlexBuffers, bincode, or MessagePack remain
outside `kalamdb-serialization`. JSON `to_vec`/`from_slice` is printed as notes
(HTTP, manifests, and topic payloads are not RocksDB object persistence).
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

PERSISTENCE_ROOTS = [
    "backend/crates/kalamdb-commons",
    "backend/crates/kalamdb-store",
    "backend/crates/kalamdb-tables",
    "backend/crates/kalamdb-system",
    "backend/crates/kalamdb-vector",
    "backend/crates/kalamdb-raft",
    "backend/crates/kalamdb-streams",
    "backend/crates/kalamdb-publisher",
    "backend/crates/kalamdb-core",
    "backend/crates/kalamdb-flush",
    "backend/crates/kalamdb-filestore",
]

ALLOWED_PATH_PREFIXES = (
    "backend/crates/kalamdb-serialization/",
)

ALLOWED_PATH_SUBSTRINGS = (
    "/generated/",
    "/tests/",
)

ALLOWED_FILE_SUFFIXES = (
    "_tests.rs",
    "/benches/",
)

# External/public wire formats, not RocksDB object persistence.
ALLOWED_PATH_EXACT_SUBSTRINGS = (
    "kalamdb-commons/src/websocket.rs",
    "kalamdb-api/src/",
    "kalamdb-auth/src/",
)

FORBIDDEN = [
    ("flatbuffers::", "direct FlatBuffers"),
    ("flexbuffers::", "direct FlexBuffers"),
    ("bincode::", "direct bincode"),
    ("rmp_serde::", "direct MessagePack"),
]

JSON_PATTERNS = [
    ("serde_json::to_vec", "JSON to_vec"),
    ("serde_json::from_slice", "JSON from_slice"),
]


def is_allowed(path: str) -> bool:
    rel = path.replace("\\", "/")
    if any(rel.startswith(prefix) for prefix in ALLOWED_PATH_PREFIXES):
        return True
    if any(part in rel for part in ALLOWED_PATH_SUBSTRINGS):
        return True
    if any(rel.endswith(suffix) or suffix in rel for suffix in ALLOWED_FILE_SUFFIXES):
        return True
    if any(part in rel for part in ALLOWED_PATH_EXACT_SUBSTRINGS):
        return True
    return False


def scan_rs(pattern: str, roots: list[str]) -> list[tuple[str, int, str]]:
    """Scan `*.rs` files without requiring ripgrep (CI runners may not have `rg`)."""
    hits: list[tuple[str, int, str]] = []
    for root in roots:
        root_path = Path(root)
        if not root_path.exists():
            continue
        for path in root_path.rglob("*.rs"):
            if "target" in path.parts:
                continue
            try:
                lines = path.read_text(encoding="utf-8").splitlines()
            except (OSError, UnicodeDecodeError):
                continue
            for line_no, text in enumerate(lines, start=1):
                if pattern in text:
                    hits.append((str(path), line_no, text.strip()))
    return hits


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--fail",
        action="store_true",
        help="exit 1 when FlatBuffers/FlexBuffers/bincode/MessagePack remain outside kalamdb-serialization",
    )
    args = parser.parse_args()

    roots = [str(ROOT / path) for path in PERSISTENCE_ROOTS if (ROOT / path).exists()]
    forbidden: list[str] = []
    json_notes: list[str] = []

    for pattern, label in FORBIDDEN:
        for path, line_no, text in scan_rs(pattern, roots):
            rel = os.path.relpath(path, ROOT)
            if is_allowed(rel):
                continue
            forbidden.append(f"{rel}:{line_no}: {label}: {text}")

    for pattern, label in JSON_PATTERNS:
        for path, line_no, text in scan_rs(pattern, roots):
            rel = os.path.relpath(path, ROOT)
            if is_allowed(rel):
                continue
            json_notes.append(f"{rel}:{line_no}: {label}: {text}")

    print("Serialization boundary scan")
    print(f"roots: {', '.join(PERSISTENCE_ROOTS)}")
    print(f"forbidden codec findings: {len(forbidden)}")
    for item in forbidden:
        print(f"  {item}")
    print(f"JSON wire/helper notes (not blocking): {len(json_notes)}")
    for item in json_notes:
        print(f"  {item}")

    if args.fail and forbidden:
        print(
            f"\n{len(forbidden)} persistence codec callsite(s) remain outside "
            "kalamdb-serialization. See docs/plans/2026-02-14-flatbuffers-flexbuffers-vortex-migration-plan.md"
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
