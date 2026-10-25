#!/usr/bin/env python3
"""Gate C0 repository-wide codec-ownership and lean-dependency check (T071, FR-048/050/056)."""
from pathlib import Path
import json
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]

FORBIDDEN_PRODUCTION_TOKENS = (
    "rmp_serde",
    "to_msgpack",
    "SerializationType::MessagePack",
)

PRODUCTION_SCAN_DIRS = (
    "backend/crates/kalamdb-api/src/ws",
    "link/link-common/src",
    "link/kalam-link-wasm/src",
    "link/kalam-link-dart/src",
)

REQUIRED_GUARANTEE_SURFACES = (
    "single_sql_statement",
    "http_sql_batch",
    "explicit_transaction",
    "native_function_call",
    "pgwire",
    "live_subscription",
    "topic_consumer",
    "file_attachment",
    "backup_restore",
)

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


def check_sdks_do_not_reimplement_kdom() -> None:
    """Function and WebSocket frames are encoded in Rust. SDK sources must not carry KDOM."""
    roots = (
        ROOT / "link/sdks/typescript/client/src",
        ROOT / "link/sdks/dart/link/lib/src",
    )
    violations: list[str] = []
    for root in roots:
        for path in root.rglob("*"):
            if not path.is_file() or path.suffix not in {".ts", ".dart"}:
                continue
            if "generated" in path.parts:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            if "KDOM" in text or "0x4B, 0x44, 0x4F, 0x4D" in text or "encode_domain_frame" in text:
                violations.append(str(path.relative_to(ROOT)))
    if violations:
        raise RuntimeError(
            "SDK sources reimplement the domain frame codec:\n" + "\n".join(violations)
        )


def check_link_common_uses_shared_serialization() -> None:
    cargo_toml = (ROOT / "link/link-common/Cargo.toml").read_text(encoding="utf-8")
    if "kalamdb-serialization" not in cargo_toml:
        raise RuntimeError("link-common must depend on kalamdb-serialization for shared domain codec ownership")


def forbidden_tokens(text: str) -> list[str]:
    return [token for token in FORBIDDEN_PRODUCTION_TOKENS if token in text]


def production_sources() -> list[Path]:
    files: list[Path] = []
    for relative in PRODUCTION_SCAN_DIRS:
        root = ROOT / relative
        if not root.is_dir():
            raise RuntimeError(f"production codec scan directory is missing: {relative}")
        files.extend(path for path in root.rglob("*") if path.is_file() and path.suffix in {".rs", ".dart", ".ts"})
    commons = ROOT / "backend/crates/kalamdb-commons/src"
    files.extend(path for path in commons.glob("websocket*") if path.is_file())
    return files


def check_production_codec_tokens() -> None:
    violations: list[str] = []
    for path in production_sources():
        text = path.read_text(encoding="utf-8", errors="replace")
        hits = forbidden_tokens(text)
        if hits:
            violations.append(f"{path.relative_to(ROOT)}: {', '.join(hits)}")
    if violations:
        raise RuntimeError(
            "production WebSocket/SDK paths still reference a removed codec:\n" + "\n".join(violations)
        )


def check_negative_control() -> None:
    sample = "\n".join(
        [
            "let bytes = rmp_serde::to_vec_named(&value);",
            "notif.to_msgpack();",
            "SerializationType::MessagePack",
        ]
    )
    found = forbidden_tokens(sample)
    missing = [token for token in FORBIDDEN_PRODUCTION_TOKENS if token not in found]
    if missing:
        raise RuntimeError(f"codec negative control failed to reject: {', '.join(missing)}")
    if forbidden_tokens("SerializationType::Domain\nencode_change_frame()"):
        raise RuntimeError("codec negative control rejected a domain-codec snippet")


def check_guarantee_matrix() -> None:
    matrix_path = ROOT / "link/tests/sdk-contracts/guarantee-matrix.json"
    prose_path = ROOT / "docs/architecture/guarantees.md"
    if not matrix_path.is_file():
        raise RuntimeError(f"missing guarantee matrix: {matrix_path}")
    if not prose_path.is_file():
        raise RuntimeError(f"missing guarantee prose: {prose_path}")
    matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
    surfaces = [row.get("surface") for row in matrix.get("surfaces", [])]
    missing = [name for name in REQUIRED_GUARANTEE_SURFACES if name not in surfaces]
    if missing:
        raise RuntimeError(f"guarantee matrix is missing surfaces: {', '.join(missing)}")
    prose = prose_path.read_text(encoding="utf-8")
    missing_prose = [name for name in REQUIRED_GUARANTEE_SURFACES if f"`{name}`" not in prose]
    if missing_prose:
        raise RuntimeError(f"guarantees.md is missing surfaces: {', '.join(missing_prose)}")


if __name__ == "__main__":
    check_lean_serialization_tree()
    check_link_common_uses_shared_serialization()
    check_sdks_do_not_reimplement_kdom()
    check_production_codec_tokens()
    check_negative_control()
    check_guarantee_matrix()
    print(
        "Gate C0 codec-ownership check passed: lean kalamdb-serialization has 0 server dependencies, "
        f"{len(ALLOWED_EXTERNAL_BOUNDARIES)} named external boundary exemptions, "
        "production paths have no removed codec tokens, and the guarantee matrix lists "
        f"{len(REQUIRED_GUARANTEE_SURFACES)} surfaces."
    )
