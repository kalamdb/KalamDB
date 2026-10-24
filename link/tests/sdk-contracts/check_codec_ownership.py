#!/usr/bin/env python3
"""Gate C0 repository-wide codec-ownership and lean-dependency check (T071, FR-048/050/056)."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]

FORBIDDEN_LEAN_CRATES = {
    "arrow",
    "arrow-array",
    "arrow-buffer",
    "arrow-data",
    "arrow-schema",
    "datafusion",
    "datafusion-common",
    "rocksdb",
    "librocksdb-sys",
    "v8",
    "actix-web",
}

ALLOWED_EXTERNAL_BOUNDARIES = {
    "kalamdb-postgres-wire": "PostgreSQL wire protocol (pgwire)",
    "kalamdb-filestore": "Cold storage Parquet/Arrow segments",
    "kalamdb-store": "RocksDB key-value engine & storekey",
    "kalamdb-raft": "Outer tonic/prost gRPC transport for cluster consensus",
    "kalamdb-api": "External HTTP JSON/OpenAPI adapter",
}


def check_lean_serialization_tree() -> None:
    cmd = ["cargo", "tree", "-p", "kalamdb-serialization", "--no-default-features", "--edges", "normal"]
    res = subprocess.run(cmd, cwd=ROOT, text=True, capture_output=True, check=True)
    for line in res.stdout.splitlines():
        tokens = line.replace("├──", " ").replace("└──", " ").replace("│", " ").split()
        if not tokens:
            continue
        crate_name = tokens[0]
        if crate_name in FORBIDDEN_LEAN_CRATES:
            raise RuntimeError(
                f"Forbidden server dependency '{crate_name}' found in lean kalamdb-serialization tree:\n{line}"
            )


def check_link_common_uses_shared_serialization() -> None:
    cargo_toml = (ROOT / "link/link-common/Cargo.toml").read_text(encoding="utf-8")
    if "kalamdb-serialization" not in cargo_toml:
        raise RuntimeError("link-common must depend on kalamdb-serialization for shared domain codec ownership")


if __name__ == "__main__":
    check_lean_serialization_tree()
    check_link_common_uses_shared_serialization()
    print(
        "Gate C0 codec-ownership check passed: lean kalamdb-serialization has 0 server dependencies "
        f"and {len(ALLOWED_EXTERNAL_BOUNDARIES)} named external boundary exemptions."
    )
