#!/usr/bin/env python3
"""Time the PGlite SQLite speed scripts on local SQLite and one KalamDB server.

SQLite uses one connection and runs each script with executescript, which is
the same shape as PGlite's baseline (better-sqlite3 db.exec on one database).
KalamDB sends each script as one HTTP request to a server that is already up.
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import time
import urllib.error
import urllib.request
from pathlib import Path

PHASES: list[tuple[str, str]] = [
    ("1", "1000 INSERTs"),
    ("2", "25000 INSERTs in a transaction"),
    ("3", "25000 INSERTs into an indexed table"),
    ("4", "100 SELECTs without an index"),
    ("5", "100 SELECTs on a string comparison"),
    ("6", "Creating an index"),
    ("7", "5000 SELECTs with an index"),
    ("8", "1000 UPDATEs without an index"),
    ("9", "25000 UPDATEs with an index"),
    ("10", "25000 text UPDATEs with an index"),
    ("11", "INSERTs from a SELECT"),
    ("12", "DELETE without an index"),
    ("13", "DELETE with an index"),
    ("14", "A big INSERT after a big DELETE"),
    ("15", "A big DELETE followed by many small INSERTs"),
    ("16", "DROP TABLE"),
]


def script_path(sql_dir: Path, phase_id: str) -> Path:
    return sql_dir / f"benchmark{phase_id}.sql"


def load_scripts(sql_dir: Path) -> list[str]:
    scripts: list[str] = []
    for phase_id, _name in PHASES:
        path = script_path(sql_dir, phase_id)
        if not path.is_file():
            raise SystemExit(f"missing {path}; run.sh downloads the PGlite scripts first")
        text = path.read_text()
        if not text.strip():
            raise SystemExit(f"{path} is empty")
        scripts.append(text)
    return scripts


def time_sqlite(database: str, scripts: list[str]) -> list[float]:
    if database != ":memory:":
        Path(database).unlink(missing_ok=True)
    connection = sqlite3.connect(database)
    connection.isolation_level = None
    seconds: list[float] = []
    try:
        for index, sql in enumerate(scripts):
            phase_id = PHASES[index][0]
            started = time.perf_counter()
            try:
                connection.executescript(sql)
            except sqlite3.Error as error:
                raise SystemExit(f"SQLite failed on test {phase_id}: {error}") from error
            seconds.append(time.perf_counter() - started)
            label = "memory" if database == ":memory:" else "disk"
            print(f"sqlite {label:<8} {phase_id:>2}  {seconds[-1]:8.3f}s", flush=True)
    finally:
        connection.close()
    return seconds


def request_json(url: str, payload: dict, headers: dict[str, str], timeout: float) -> tuple[float, dict]:
    body = json.dumps(payload).encode()
    request = urllib.request.Request(url, data=body, headers=headers, method="POST")
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read()
    except urllib.error.HTTPError as error:
        detail = error.read().decode(errors="replace")[:800]
        raise SystemExit(f"HTTP {error.code} from {url}: {detail}") from error
    elapsed = time.perf_counter() - started
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as error:
        raise SystemExit(f"response from {url} was not JSON: {raw[:400]!r}") from error
    return elapsed, parsed


def login(base_url: str, user: str, password: str) -> str:
    _elapsed, payload = request_json(
        f"{base_url}/v1/api/auth/login",
        {"user": user, "password": password},
        {"content-type": "application/json"},
        30,
    )
    token = payload.get("access_token")
    if not isinstance(token, str) or not token:
        raise SystemExit(f"login did not return an access token: {payload}")
    return token


def execute_sql(base_url: str, token: str, sql: str, namespace: str | None) -> tuple[float, float]:
    payload: dict = {"sql": sql}
    if namespace is not None:
        payload["namespace_id"] = namespace
    elapsed, body = request_json(
        f"{base_url}/v1/api/sql",
        payload,
        {
            "content-type": "application/json",
            "authorization": f"Bearer {token}",
        },
        600,
    )
    if body.get("status") != "success":
        message = body.get("error") or body
        raise SystemExit(f"SQL failed after {elapsed:.3f}s: {message}")
    took_ms = float(body.get("took") or 0.0)
    return elapsed, took_ms / 1000.0


def run_kalam(base_url: str, user: str, password: str, scripts: list[str]) -> tuple[list[float], list[float]]:
    token = login(base_url.rstrip("/"), user, password)
    base_url = base_url.rstrip("/")
    execute_sql(base_url, token, "CREATE SCHEMA speed", None)
    client_seconds: list[float] = []
    server_seconds: list[float] = []
    for index, sql in enumerate(scripts):
        phase_id, name = PHASES[index]
        client, server = execute_sql(base_url, token, sql, "speed")
        client_seconds.append(client)
        server_seconds.append(server)
        print(f"kalam release  {phase_id:>2} {name:<46} client {client:8.3f}s  server {server:8.3f}s", flush=True)
    return client_seconds, server_seconds


def write_json(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2) + "\n")


def print_report(sqlite_path: Path, kalam_path: Path) -> None:
    sqlite_result = json.loads(sqlite_path.read_text())
    kalam_result = json.loads(kalam_path.read_text())
    memory = sqlite_result["memory"]
    disk = sqlite_result["disk"]
    client = kalam_result["client"]
    server = kalam_result["server"]
    print()
    print("Same machine, same SQL scripts, seconds.")
    print("SQLite is one process and one connection. KalamDB is one release server, one HTTP request per test.")
    print(
        f"{'#':<4} {'Test':<46} {'SQLite':>8} {'SQLite':>8} {'KalamDB':>8} {'server':>8} {'vs mem':>8}"
    )
    print(f"{'':<4} {'':<46} {'memory':>8} {'disk':>8} {'client':>8} {'took':>8} {'':>8}")
    totals = [0.0, 0.0, 0.0, 0.0]
    for index, (phase_id, name) in enumerate(PHASES):
        ratio = client[index] / memory[index] if memory[index] > 0 else 0.0
        print(
            f"{phase_id:<4} {name:<46} {memory[index]:8.3f} {disk[index]:8.3f} "
            f"{client[index]:8.3f} {server[index]:8.3f} {ratio:7.1f}x"
        )
        totals[0] += memory[index]
        totals[1] += disk[index]
        totals[2] += client[index]
        totals[3] += server[index]
    total_ratio = totals[2] / totals[0] if totals[0] > 0 else 0.0
    print(
        f"{'':<4} {'total':<46} {totals[0]:8.3f} {totals[1]:8.3f} "
        f"{totals[2]:8.3f} {totals[3]:8.3f} {total_ratio:7.1f}x"
    )
    print()


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)

    sqlite_cmd = sub.add_parser("sqlite")
    sqlite_cmd.add_argument("--sql-dir", type=Path, required=True)
    sqlite_cmd.add_argument("--disk", type=Path, required=True)
    sqlite_cmd.add_argument("--out", type=Path, required=True)

    kalam_cmd = sub.add_parser("kalam")
    kalam_cmd.add_argument("--sql-dir", type=Path, required=True)
    kalam_cmd.add_argument("--url", required=True)
    kalam_cmd.add_argument("--user", required=True)
    kalam_cmd.add_argument("--password", required=True)
    kalam_cmd.add_argument("--out", type=Path, required=True)

    report_cmd = sub.add_parser("report")
    report_cmd.add_argument("--sqlite", type=Path, required=True)
    report_cmd.add_argument("--kalam", type=Path, required=True)

    args = parser.parse_args()
    if args.command == "sqlite":
        scripts = load_scripts(args.sql_dir)
        args.disk.parent.mkdir(parents=True, exist_ok=True)
        memory = time_sqlite(":memory:", scripts)
        disk = time_sqlite(str(args.disk), scripts)
        write_json(args.out, {"memory": memory, "disk": disk})
    elif args.command == "kalam":
        scripts = load_scripts(args.sql_dir)
        client, server = run_kalam(args.url, args.user, args.password, scripts)
        write_json(args.out, {"client": client, "server": server})
    else:
        print_report(args.sqlite, args.kalam)


if __name__ == "__main__":
    main()
