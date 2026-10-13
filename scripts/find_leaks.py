#!/usr/bin/env python3
"""Find test functions that create namespaces/users/topics but don't clean them up."""

import os
import re

test_dir = "cli/tests"

# Find all .rs files
rs_files = []
for root, dirs, files in os.walk(test_dir):
    for f in files:
        if f.endswith(".rs"):
            rs_files.append(os.path.join(root, f))

rs_files.sort()

results = []

for fpath in rs_files:
    with open(fpath) as fp:
        content = fp.read()

    # Skip mod.rs and harness files
    basename = os.path.basename(fpath)
    if basename == "mod.rs":
        continue
    # Skip non-test gateway files (just re-exports)
    if content.strip().startswith("mod ") and len(content) < 200:
        continue

    # Check if file creates namespaces or users at all
    has_create_ns = bool(re.search(r"CREATE NAMESPACE|generate_unique_namespace", content))
    # Real CREATE USER (not CREATE USER TABLE)
    has_create_user = bool(re.search(r"CREATE USER\s+(?!TABLE)", content, re.IGNORECASE))
    has_create_topic = bool(re.search(r"CREATE TOPIC", content))

    if not has_create_ns and not has_create_user and not has_create_topic:
        continue

    lines = content.split("\n")

    # Find function start lines (with their decorator context)
    fn_starts = []
    for i, line in enumerate(lines):
        m = re.match(r"\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\(", line)
        if m:
            # Check if preceded by #[test] or #[tokio::test]
            is_test = False
            for j in range(max(0, i-5), i):
                if re.search(r"#\[(?:tokio::)?test", lines[j]):
                    is_test = True
                    break
            fn_starts.append((i, m.group(1), is_test))

    for idx, (start_line, fn_name, is_test) in enumerate(fn_starts):
        if idx + 1 < len(fn_starts):
            end_line = fn_starts[idx + 1][0]
        else:
            end_line = len(lines)

        fn_body = "\n".join(lines[start_line:end_line])

        # Check what this function creates
        creates_ns = bool(re.search(r"CREATE NAMESPACE|generate_unique_namespace", fn_body))
        drops_ns = bool(re.search(r"DROP (?:NAMESPACE|SCHEMA)", fn_body))
        # Distinguish CREATE USER (entity) from CREATE USER TABLE
        creates_user = bool(re.search(r"CREATE USER\s+(?!TABLE)", fn_body, re.IGNORECASE))
        drops_user = bool(re.search(r"DROP USER", fn_body))
        creates_topic = bool(re.search(r"CREATE TOPIC", fn_body))
        drops_topic = bool(re.search(r"DROP TOPIC", fn_body))

        # Find namespace variable name
        ns_var_match = re.search(
            r"let\s+(\w+)\s*=\s*(?:common::)?generate_unique_namespace", fn_body
        )
        ns_var = ns_var_match.group(1) if ns_var_match else None

        # Find executor
        executors = set()
        for ex in [
            "execute_sql_as_root_via_client",
            "execute_sql_as_root_via_cli",
            "execute_sql_via_http_as_root",
            "execute_sql",
            "execute_on_node",
        ]:
            if ex in fn_body:
                executors.add(ex)

        leaked = []
        if creates_ns and not drops_ns:
            leaked.append("namespace")
        if creates_user and not drops_user:
            leaked.append("user")
        if creates_topic and not drops_topic:
            leaked.append("topic")

        if leaked:
            results.append(
                {
                    "file": fpath,
                    "fn": fn_name,
                    "line": start_line + 1,
                    "leaked": leaked,
                    "ns_var": ns_var,
                    "executors": sorted(executors),
                    "is_test": is_test,
                }
            )

# Print results grouped by file
print("Found %d leaking test functions:\n" % len(results))

current_file = None
for r in results:
    if r["file"] != current_file:
        current_file = r["file"]
        print("=" * 100)
        print("FILE: %s" % r["file"])
        print("=" * 100)
    tag = "[TEST]" if r["is_test"] else "[HELPER]"
    print("  %s %s() (line %d)" % (tag, r["fn"], r["line"]))
    print("    Leaked resources: %s" % ", ".join(r["leaked"]))
    if r["ns_var"]:
        print("    Namespace variable: %s" % r["ns_var"])
    exs = ", ".join(r["executors"]) if r["executors"] else "none found"
    print("    Executors: %s" % exs)
    print()

# Summary
ns_leaks = sum(1 for r in results if "namespace" in r["leaked"])
user_leaks = sum(1 for r in results if "user" in r["leaked"])
topic_leaks = sum(1 for r in results if "topic" in r["leaked"])
test_leaks = sum(1 for r in results if r["is_test"])
helper_leaks = sum(1 for r in results if not r["is_test"])
files_affected = len(set(r["file"] for r in results))

print("\n" + "=" * 100)
print("SUMMARY")
print("=" * 100)
print("Total leaking functions: %d (%d tests, %d helpers)" % (len(results), test_leaks, helper_leaks))
print("Files affected: %d" % files_affected)
print("Namespace leaks: %d" % ns_leaks)
print("User leaks: %d" % user_leaks)
print("Topic leaks: %d" % topic_leaks)
