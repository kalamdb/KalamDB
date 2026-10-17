# Contract: Cleanup and Batched Rewrites

Applies to every slice in this feature.

## Cleanup step

A change that replaces behavior deletes the replaced path in the same change.

- Remove the old function, module, mapping table, fallback, and test helper that existed only for the old path.
- If two modules do the same work, keep one owner and delete the other.
- A temporary decoder or alias is allowed only when [schema-migration.md](schema-migration.md) or [type-system.md](type-system.md) requires it. It has one owner and a removal milestone on [cleanup-ledger.md](cleanup-ledger.md).
- Adding a new file requires a line in the slice note saying why an existing file could not take the change.
- The slice looks for a simpler structure. A rename that leaves the old control flow in place is not cleanup.

The slice is not accepted while a leftover product namespace identifier, a second type authority, or an unused compatibility path from that slice remains outside the exception list.

## Rewrite program

Location: `scripts/pg18-rewrite/`.

It is a repository tool. It is not linked into the server.

### Invocation

One batch per run. A batch is one crate or one test, fixture, or example directory.

```text
scripts/pg18-rewrite/rewrite.sh --batch <crate-or-directory> --exceptions <file>
```

The script prints the files it changed and the exception hits it skipped. Running it again on the same batch is a no-op.

### What it may change

- Rust identifiers and module names on the allowlist, starting with `NamespaceId` to `SchemaId` and the matching field and file names for the SQL schema concept
- Repeated SQL text on the allowlist, starting with `CREATE NAMESPACE` to `CREATE SCHEMA`, only in files not listed as negative tests

### What it must not change

- The deny list of PostgreSQL identifiers in [schema-migration.md](schema-migration.md)
- Negative tests that intentionally contain the old syntax
- Generated SDK trees
- Stored-format bytes, until the persisted-format decision for that batch is already written

Behavior that is not a mechanical replacement is edited in the same batch by hand, in that batch's files only, and covered by that batch's test command.

### Verification

Run the narrowest check that can fail the batch before starting the next one. Typical commands are in [quickstart.md](../quickstart.md). Record the command, the revision, and the ledger rows in the batch note.

Do not run the full workspace test suite as the only way to review a rename batch. Do run it at final stage 7.

### Integration safety

Rewrite exact allowlisted tokens/AST forms, not substrings in SQL literals, dollar-quoted routine bodies, comments or identifier values. Dry-run records expected match counts and file hashes; apply aborts on drift/unexpected matches. Preserve an auditable before/after diff and test assertion mapping. The tool cannot rename durable IDs or change semantics just because a spelling matches.

An exported type rename can span a dependency-complete integration batch even when edits run crate by crate. Keep a single temporary adapter only when needed for buildability, with owner and explicit removal task. Never create parallel mutable authorities. If removing an assertion that enforced old non-PG behavior, replace it with both the PostgreSQL regression and any explicit Kalam-extension regression; preserving an incorrect assertion is not a compatibility requirement.
