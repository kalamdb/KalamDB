#!/usr/bin/env python3
"""Keep server storage and execution engines out of the CLI build graph."""

import subprocess
import sys
from pathlib import Path


def main():
    root = Path(__file__).resolve().parent.parent
    result = subprocess.run(
        [
            "cargo", "tree", "--locked", "-p", "kalam-cli",
            "--edges", "normal,build", "--prefix", "none", "--format", "{p}",
        ],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        print(result.stderr, file=sys.stderr, end="")
        return result.returncode
    packages = {line.split()[0] for line in result.stdout.splitlines() if line.strip()}
    forbidden = packages & {
        "rocksdb", "librocksdb-sys", "datafusion", "kalamdb-store", "kalamdb-system",
    }
    if forbidden:
        print("CLI pulls in server implementation: " + ", ".join(sorted(forbidden)), file=sys.stderr)
        return 1
    print("CLI dependency boundary passed: no storage or query execution engine")
    return 0


if __name__ == "__main__":
    sys.exit(main())
