#!/usr/bin/env python3
"""Summarize recent kalamdb-server traces from Jaeger v2's HTTP API v3."""

from __future__ import annotations

import json
import statistics
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections import defaultdict
from datetime import datetime, timedelta, timezone

JAEGER = "http://127.0.0.1:16686"
SERVICE = "kalamdb-server"
LIMIT = 400


def fetch(path: str) -> dict:
    url = f"{JAEGER}{path}"
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.load(response)


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    idx = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * pct / 100.0)))
    return ordered[idx]


def collect_spans(payload: dict) -> list[dict]:
    result = payload.get("result") or payload
    spans: list[dict] = []
    for resource in result.get("resourceSpans") or []:
        for scope in resource.get("scopeSpans") or []:
            spans.extend(scope.get("spans") or [])
    return spans


def duration_us(span: dict) -> float:
    start = int(span.get("startTimeUnixNano") or 0)
    end = int(span.get("endTimeUnixNano") or 0)
    return (end - start) / 1000.0


def span_attrs(span: dict) -> dict[str, object]:
    out: dict[str, object] = {}
    for attribute in span.get("attributes") or []:
        key = attribute.get("key")
        value = attribute.get("value") or {}
        if "intValue" in value:
            out[key] = int(value["intValue"])
        elif "doubleValue" in value:
            out[key] = value["doubleValue"]
        elif "stringValue" in value:
            raw = value["stringValue"]
            out[key] = int(raw) if raw.lstrip("-").isdigit() else raw
        elif "boolValue" in value:
            out[key] = value["boolValue"]
    return out


READ_PREFIXES = (
    "sql.point_get",
    "sql.execute",
    "sql.scalar_to_arrow",
    "table.",
    "store.pk_get",
    "wire.",
)


def is_read_span(name: str) -> bool:
    return any(name == prefix or name.startswith(prefix) for prefix in READ_PREFIXES)


def print_table(title: str, by_name: dict[str, list[float]], width: int = 36) -> None:
    print(f"\n## {title}")
    if not by_name:
        print("  (no spans)")
        return
    rows = []
    for name, samples in by_name.items():
        rows.append(
            (
                name,
                len(samples),
                statistics.mean(samples),
                percentile(samples, 50),
                percentile(samples, 95),
                max(samples),
            )
        )
    rows.sort(key=lambda row: row[2], reverse=True)
    print(f"{'span':<{width}} {'n':>6} {'mean_us':>10} {'p50_us':>10} {'p95_us':>10} {'max_us':>10}")
    for name, count, mean, p50, p95, max_us in rows:
        print(
            f"{name:<{width}} {count:>6} {mean:>10.1f} {p50:>10.1f} {p95:>10.1f} {max_us:>10.1f}"
        )


def exclusive_us(span: dict, children: dict[str, list[dict]]) -> float:
    child_total = sum(duration_us(child) for child in children.get(span.get("spanId") or "", []))
    return max(0.0, duration_us(span) - child_total)


def main() -> int:
    try:
        services = fetch("/api/v3/services")
    except urllib.error.URLError as error:
        print(f"Jaeger is not reachable at {JAEGER}: {error}", file=sys.stderr)
        return 1
    except json.JSONDecodeError:
        print(
            f"Jaeger at {JAEGER} did not return JSON from /api/v3/services. "
            "Use jaegertracing/jaeger:latest (v2) or the all-in-one from docker/utils.",
            file=sys.stderr,
        )
        return 1

    names = services.get("services") or []
    print(f"Jaeger services: {names}")
    if SERVICE not in names:
        print(f"Service {SERVICE!r} is not in Jaeger yet. Run scripts/run-kalamdb-trace.sh first.")
        return 1

    operations = fetch(f"/api/v3/operations?service={urllib.parse.quote(SERVICE)}")
    op_names = [item.get("name") for item in operations.get("operations") or []]
    print("Operations:", ", ".join(op_names))

    now = datetime.now(timezone.utc)
    start = now - timedelta(hours=2)
    query = urllib.parse.urlencode(
        {
            "query.serviceName": SERVICE,
            "query.startTimeMin": start.strftime("%Y-%m-%dT%H:%M:%SZ"),
            "query.startTimeMax": now.strftime("%Y-%m-%dT%H:%M:%SZ"),
            "query.searchDepth": str(LIMIT),
        }
    )
    payload = fetch(f"/api/v3/traces?{query}")
    spans = collect_spans(payload)
    print(f"\nFetched {len(spans)} spans")

    by_name: dict[str, list[float]] = defaultdict(list)
    by_trace: dict[str, list[dict]] = defaultdict(list)
    for span in spans:
        by_name[span.get("name") or "(unnamed)"].append(duration_us(span))
        by_trace[span.get("traceId") or ""].append(span)

    print_table("all spans (OTLP on — durations are inflated vs bake-off)", by_name)

    raft_names = [name for name in by_name if name.startswith("raft.")]
    print_table(
        "raft internals (client_write is wait; append/apply/persist are worker CPU+IO)",
        {name: by_name[name] for name in raft_names},
    )

    apply_counts: dict[int, int] = defaultdict(int)
    for span in spans:
        if span.get("name") != "raft.apply":
            continue
        count = span_attrs(span).get("entry_count")
        if isinstance(count, int):
            apply_counts[count] += 1
    if apply_counts:
        print("\n## raft.apply batch sizes")
        for count, times in sorted(apply_counts.items()):
            print(f"  {times} applies with entry_count={count}")

    def children_of(trace_spans: list[dict]) -> dict[str, list[dict]]:
        grouped: dict[str, list[dict]] = defaultdict(list)
        for span in trace_spans:
            parent = span.get("parentSpanId") or ""
            if parent:
                grouped[parent].append(span)
        return grouped

    def dump_example(title: str, required: str, exclude: str | None = None) -> None:
        print(f"\nExample waterfall ({title}):")
        for trace_id, trace_spans in by_trace.items():
            names = {span.get("name") for span in trace_spans}
            if required not in names:
                continue
            if exclude is not None and exclude in names:
                continue
            ordered = sorted(trace_spans, key=lambda span: int(span.get("startTimeUnixNano") or 0))
            start_ns = int(ordered[0].get("startTimeUnixNano") or 0)
            kids = children_of(trace_spans)
            print(f"  trace {trace_id[:16]}…")
            print(f"    {'offset':>9}  {'incl_us':>8}  {'excl_us':>8}  span")
            for span in ordered:
                offset_us = (int(span.get("startTimeUnixNano") or 0) - start_ns) / 1000.0
                extra = ""
                count = span_attrs(span).get("entry_count")
                if count is not None:
                    extra = f"  entry_count={count}"
                print(
                    f"    +{offset_us:8.1f}us  {duration_us(span):8.1f}  "
                    f"{exclusive_us(span, kids):8.1f}  {span.get('name')}{extra}"
                )
            return
        print("  (none)")

    exclusive_by_name: dict[str, list[float]] = defaultdict(list)
    read_inclusive: dict[str, list[float]] = defaultdict(list)
    for _trace_id, trace_spans in by_trace.items():
        names = {span.get("name") for span in trace_spans}
        if "sql.point_get" not in names:
            continue
        kids = children_of(trace_spans)
        for span in trace_spans:
            name = span.get("name") or "(unnamed)"
            exclusive_by_name[name].append(exclusive_us(span, kids))
            if is_read_span(name):
                read_inclusive[name].append(duration_us(span))

    read_exclusive = {
        name: samples
        for name, samples in exclusive_by_name.items()
        if is_read_span(name)
    }
    print_table(
        "point-get traces — inclusive (parent includes children; OTLP-inflated)",
        read_inclusive,
    )
    print_table(
        "point-get traces — exclusive (self time after subtracting children)",
        read_exclusive,
    )

    dump_example("insert", "sql.insert_literal")
    dump_example("point get", "sql.point_get", exclude="sql.insert_literal")
    dump_example("raft apply worker", "raft.apply")
    dump_example("raft persist", "raft.persist_applied")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
