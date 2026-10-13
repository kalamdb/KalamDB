#!/usr/bin/env bash
# One-batch PostgreSQL compatibility rewriter.
# Rewrites exact allowlisted tokens outside comments, strings, and dollar quotes.
# A second run on the same batch is a no-op.
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: scripts/pg18-rewrite/rewrite.sh --batch <crate-or-directory> --exceptions <file> [--dry-run|--apply]" >&2
  exit 2
fi

exec python3 - "$@" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

DENY = (
    "pg_namespace",
    "nspname",
    "nspowner",
    "nspacl",
    "typnamespace",
    "pronamespace",
    "relnamespace",
)
IDENT_REPLACEMENTS = (("NamespaceId", "SchemaId"),)
SQL_PAIRS = (
    ("CREATE", "NAMESPACE", "CREATE", "SCHEMA"),
    ("create", "namespace", "create", "schema"),
    ("DROP", "NAMESPACE", "DROP", "SCHEMA"),
    ("drop", "namespace", "drop", "schema"),
    ("ALTER", "NAMESPACE", "ALTER", "SCHEMA"),
    ("alter", "namespace", "alter", "schema"),
    ("SHOW", "NAMESPACES", "SHOW", "SCHEMAS"),
    ("show", "namespaces", "show", "schemas"),
    ("SET", "NAMESPACE", "SET", "SCHEMA"),
    ("set", "namespace", "set", "schema"),
    ("USE", "NAMESPACE", "SET", "SCHEMA"),
    ("use", "namespace", "set", "schema"),
)
SOURCE_SUFFIXES = {".rs", ".sql"}
SKIP_DIR_NAMES = {"target", "generated", "node_modules", ".git"}


def usage():
    print(
        "usage: scripts/pg18-rewrite/rewrite.sh --batch <crate-or-directory> "
        "--exceptions <file> [--dry-run|--apply]",
        file=sys.stderr,
    )
    return 2


def repo_root():
    return Path(__file__).resolve().parents[2] if Path(__file__).exists() else Path.cwd()


def parse_args(argv):
    batch = None
    exceptions = None
    mode = "apply"
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg == "--batch" and i + 1 < len(argv):
            batch = argv[i + 1]
            i += 2
        elif arg == "--exceptions" and i + 1 < len(argv):
            exceptions = argv[i + 1]
            i += 2
        elif arg == "--dry-run":
            mode = "dry-run"
            i += 1
        elif arg == "--apply":
            mode = "apply"
            i += 1
        else:
            print(f"unexpected argument: {arg}", file=sys.stderr)
            return None
    if not batch or not exceptions:
        return None
    return batch, exceptions, mode


def is_ident_char(ch):
    return ch.isalnum() or ch == "_"


def raw_string_at(text, i):
    """Index of the `r` in a raw string, when `i` is at that string's prefix."""
    if i >= len(text):
        return None
    if text[i] in "bc" and (i == 0 or not is_ident_char(text[i - 1])):
        if i + 1 < len(text) and text[i + 1] == "r":
            return i + 1
        return None
    if text[i] == "r" and (i == 0 or not is_ident_char(text[i - 1])):
        return i
    return None


def rust_quote_end(text, i):
    """End index of a character literal, or one past a lifetime quote."""
    n = len(text)
    if i + 1 >= n:
        return i + 1
    nxt = text[i + 1]
    if nxt == "\\":
        j = i + 2
        if j < n and text[j] == "u" and j + 1 < n and text[j + 1] == "{":
            close = text.find("}'", j)
            return n if close < 0 else close + 2
        if j < n:
            j += 1
        if j < n and text[j] == "'":
            j += 1
        return j
    if i + 2 < n and text[i + 2] == "'":
        return i + 3
    return i + 1


def find_raw_string_end(text, i):
    hashes = 0
    j = i + 1
    while j < len(text) and text[j] == "#":
        hashes += 1
        j += 1
    if j >= len(text) or text[j] != '"':
        return None
    closer = '"' + ("#" * hashes)
    end = text.find(closer, j + 1)
    if end < 0:
        return len(text)
    return end + len(closer)


def protected_ranges(text, kind):
    ranges = []
    i = 0
    n = len(text)
    while i < n:
        if kind == "rust" and text.startswith("//", i):
            end = text.find("\n", i)
            end = n if end < 0 else end
            ranges.append((i, end))
            i = end
            continue
        if kind == "sql" and text.startswith("--", i):
            end = text.find("\n", i)
            end = n if end < 0 else end
            ranges.append((i, end))
            i = end
            continue
        if text.startswith("/*", i):
            end = text.find("*/", i + 2)
            end = n if end < 0 else end + 2
            ranges.append((i, end))
            i = end
            continue
        if kind == "rust":
            raw_at = raw_string_at(text, i)
            if raw_at is not None:
                raw_end = find_raw_string_end(text, raw_at)
                if raw_end is not None:
                    ranges.append((i, raw_end))
                    i = raw_end
                    continue
        if kind == "rust" and text[i] == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            ranges.append((i, j))
            i = j
            continue
        if kind == "rust" and text[i] == "'":
            char_end = rust_quote_end(text, i)
            if char_end > i + 1:
                ranges.append((i, char_end))
            i = max(char_end, i + 1)
            continue
        if kind == "sql" and text[i] == "'":
            j = i + 1
            while j < n:
                if text.startswith("''", j):
                    j += 2
                    continue
                if text[j] == "'":
                    j += 1
                    break
                j += 1
            ranges.append((i, j))
            i = j
            continue
        if kind == "sql" and text[i] == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 1
            j = min(n, j + 1)
            ranges.append((i, j))
            i = j
            continue
        if kind == "sql" and text[i] == "$":
            j = i + 1
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            if j < n and text[j] == "$":
                tag = text[i : j + 1]
                end = text.find(tag, j + 1)
                end = n if end < 0 else end + len(tag)
                ranges.append((i, end))
                i = end
                continue
        i += 1
    return ranges


def overlaps(ranges, start, end):
    for left, right in ranges:
        if start < right and end > left:
            return True
    return False


def token_at(text, start, token):
    end = start + len(token)
    if text[start:end] != token:
        return False
    before = text[start - 1] if start else ""
    after = text[end] if end < len(text) else ""
    if before and is_ident_char(before):
        return False
    if after and is_ident_char(after):
        return False
    return True


def count_token(text, token):
    count = 0
    start = 0
    while True:
        found = text.find(token, start)
        if found < 0:
            return count
        if token_at(text, found, token):
            count += 1
        start = found + 1


def collect_edits(text, kind):
    protected = protected_ranges(text, kind)
    edits = []
    protected_hits = 0
    for old, new in IDENT_REPLACEMENTS:
        start = 0
        while True:
            found = text.find(old, start)
            if found < 0:
                break
            end = found + len(old)
            if token_at(text, found, old):
                if overlaps(protected, found, end):
                    protected_hits += 1
                else:
                    edits.append((found, end, new))
            start = found + 1
    if kind == "sql":
        for first, second, new_first, new_second in SQL_PAIRS:
            start = 0
            while True:
                found = text.find(first, start)
                if found < 0:
                    break
                if not token_at(text, found, first):
                    start = found + 1
                    continue
                cursor = found + len(first)
                while cursor < len(text) and text[cursor] in " \t\r\n":
                    cursor += 1
                if token_at(text, cursor, second):
                    end = cursor + len(second)
                    if overlaps(protected, found, end):
                        protected_hits += 1
                    else:
                        gap = text[found + len(first) : cursor]
                        edits.append((found, end, new_first + gap + new_second))
                    start = end
                else:
                    start = found + 1
    edits.sort()
    # Drop overlapping edits. Identifier and keyword spans should not overlap.
    chosen = []
    last_end = -1
    for edit in edits:
        if edit[0] < last_end:
            return None, protected_hits
        chosen.append(edit)
        last_end = edit[1]
    return chosen, protected_hits


def apply_edits(text, edits):
    parts = []
    cursor = 0
    for start, end, replacement in edits:
        parts.append(text[cursor:start])
        parts.append(replacement)
        cursor = end
    parts.append(text[cursor:])
    return "".join(parts)


def load_negative_tests(exceptions_file, root):
    paths = []
    for line in exceptions_file.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        marker = None
        for prefix in ("negative-test:", "skip:"):
            if stripped.startswith(prefix):
                marker = stripped[len(prefix) :].strip()
                break
        if not marker:
            continue
        candidate = (root / marker).resolve()
        paths.append(candidate)
    return paths


def is_negative(path, negative_paths):
    for item in negative_paths:
        if path == item or item in path.parents:
            return True
    return False


def source_files(batch, negative_paths):
    files = []
    skipped = []
    for path in sorted(batch.rglob("*")):
        if not path.is_file():
            continue
        if any(part in SKIP_DIR_NAMES for part in path.parts):
            continue
        if path.suffix not in SOURCE_SUFFIXES:
            continue
        if is_negative(path.resolve(), negative_paths):
            skipped.append(path)
            continue
        files.append(path)
    return files, skipped


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def read_source(path):
    return path.read_bytes().decode("utf-8")


def manifest_path(root, batch):
    relative = batch.resolve().relative_to(root.resolve())
    name = str(relative).replace("/", "__") + ".json"
    return root / "scripts" / "pg18-rewrite" / "state" / name


def scan_file(text, path):
    kind = "sql" if path.suffix == ".sql" else "rust"
    edits, protected_hits = collect_edits(text, kind)
    if edits is None:
        print(f"overlapping edits in {path}", file=sys.stderr)
        return None
    rewritten = apply_edits(text, edits) if edits else text
    for token in DENY:
        if count_token(text, token) != count_token(rewritten, token):
            print(f"rewrite would change denied identifier {token} in {path}", file=sys.stderr)
            return None
    return edits, protected_hits, rewritten


def main(argv):
    parsed = parse_args(argv)
    if parsed is None:
        return usage()
    batch_arg, exceptions_arg, mode = parsed
    root = Path.cwd()
    batch = (root / batch_arg).resolve()
    exceptions = (root / exceptions_arg).resolve()
    root_resolved = root.resolve()
    if root_resolved not in batch.parents and batch != root_resolved:
        print("batch must stay inside the repository", file=sys.stderr)
        return 3
    if batch == root_resolved:
        print("refusing to rewrite the repository root; pass one crate or directory", file=sys.stderr)
        return 3
    if not batch.is_dir():
        print(f"batch is not a directory: {batch_arg}", file=sys.stderr)
        return 2
    if not exceptions.is_file():
        print(f"exceptions file not found: {exceptions_arg}", file=sys.stderr)
        return 2

    negative = load_negative_tests(exceptions, root)
    files, skipped_files = source_files(batch, negative)
    planned = {}
    protected_total = 0
    for path in files:
        raw = path.read_bytes()
        if b"\0" in raw:
            continue
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            print(f"apply refused: {path} is not utf-8", file=sys.stderr)
            return 3
        scanned = scan_file(text, path)
        if scanned is None:
            return 3
        edits, protected_hits, rewritten = scanned
        protected_total += protected_hits
        if not edits:
            continue
        relative = str(path.resolve().relative_to(root_resolved))
        planned[relative] = {
            "sha256": sha256_bytes(raw),
            "matches": len(edits),
            "rewritten": rewritten,
        }
        print(f"{mode} {relative} matches={len(edits)}")

    for path in skipped_files:
        kind = "sql" if path.suffix == ".sql" else "rust"
        text = read_source(path)
        edits, _protected = collect_edits(text, kind)
        hits = len(edits or [])
        if hits:
            relative = str(path.resolve().relative_to(root_resolved))
            print(f"skipped exception {relative} hits={hits}")

    if protected_total:
        print(f"protected hits left unchanged: {protected_total}")

    state_path = manifest_path(root, batch)
    record = {
        "batch": str(batch.resolve().relative_to(root_resolved)),
        "exceptions": str(exceptions.resolve().relative_to(root_resolved)),
        "files": {key: {"sha256": value["sha256"], "matches": value["matches"]} for key, value in planned.items()},
    }

    if mode == "dry-run":
        state_path.parent.mkdir(parents=True, exist_ok=True)
        state_path.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"dry-run files={len(planned)} matches={sum(item['matches'] for item in planned.values())}")
        print(f"manifest {state_path.relative_to(root_resolved)}")
        return 0

    if not planned:
        print("no-op")
        return 0

    if not state_path.is_file():
        print("apply refused: run --dry-run on this batch first", file=sys.stderr)
        return 3
    previous = json.loads(state_path.read_text(encoding="utf-8"))
    if previous.get("batch") != record["batch"] or previous.get("exceptions") != record["exceptions"]:
        print("apply refused: dry-run manifest is for a different batch", file=sys.stderr)
        return 3
    if previous.get("files") != record["files"]:
        print("apply refused: match counts or file hashes drifted since dry-run", file=sys.stderr)
        return 3

    for relative, item in planned.items():
        path = root_resolved / relative
        if sha256_bytes(path.read_bytes()) != item["sha256"]:
            print(f"apply refused: {relative} changed since dry-run", file=sys.stderr)
            return 3
    for relative, item in planned.items():
        path = root_resolved / relative
        path.write_bytes(item["rewritten"].encode("utf-8"))
        print(f"changed {relative}")
    print(f"apply files={len(planned)} matches={sum(item['matches'] for item in planned.values())}")
    return 0


if __name__ == "__main__":
    # Path(__file__) is not the shell script when reading from stdin.
    sys.exit(main(sys.argv[1:]))
PY
