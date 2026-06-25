# Live OKF Context Sync

AI apps need per-user context: preferences, notes, runbooks, policies, and project knowledge.
This example shows how to sync a **Google Open Knowledge Format (OKF)** folder into KalamDB without building a custom realtime backend.

Use it when the same user runs multiple clients — a laptop sync worker, a browser UI, a **local agent**, and a **cloud agent** — and all of them need realtime access to the same folder of OKF Markdown files for agent context.

KalamDB stores live file metadata in a user-scoped SQL table. File bytes are handled through upload/download. Every client subscribes to metadata changes and downloads OKF files only when needed.

**This is a showcase app built on top of KalamDB.** KalamDB does not implement folder sync internally.

## Google Open Knowledge Format (OKF)

**Google Open Knowledge Format (OKF)** is a folder of Markdown files with optional YAML frontmatter that agents use as structured context (profiles, runbooks, project notes). This example treats each user's `context/{username}/` directory as their OKF root.

## Multi-client sync

```text
Device A (OKF folder)     Device B (OKF folder)
         \                       /
          v                     v
            KalamDB metadata + files
                      |
        +-------------+-------------+
        |             |             |
     web UI      local agent    cloud agent
```

Each device runs a sync worker for its OKF folder. The web UI and agents do not read disk — they query metadata and download files from KalamDB, so they always see the same live context.

## Design rule

The source of truth for the database schema is:

```text
kalam/schema.sql
```

Not `schema.ts`, Drizzle schema, or generated TypeScript schema.

The TypeScript code uses the KalamDB TypeScript SDK to connect, query, subscribe, upload, and download files.

## Quick start

```bash
npm install
kalam dev
```

Then:

1. Open the web app at `http://127.0.0.1:5177`
2. Login as **alice** (default)
3. Edit `context/alice/profile.md` (an OKF profile file)
4. See the file update live in the UI
5. Run the demo agent: `npm run dev:agent -- "Who is Alice?"`

`kalam dev` starts KalamDB Server, applies `kalam/schema.sql`, runs the sync worker, web UI, and demo agent.

Site docs: [Live OKF Context Sync](https://kalamdb.org/docs/use-cases/live-okf-context-sync)

Default local credentials:

- Server: `http://127.0.0.1:2900`
- Root (managed by kalam dev): `root` / `kalamdb123`
- Demo users: `alice` / `alice123`, `bob` / `bob123`

## What this demonstrates

```text
local Google Open Knowledge Format (OKF) folder
  -> file upload
  -> KalamDB metadata row
  -> live subscription
  -> file download
  -> UI and agents see latest OKF context
```

## Project layout

```text
examples/live-okf-context-sync/
  README.md
  kalam.toml
  package.json
  kalam/schema.sql
  context/alice/...
  context/bob/...
  src/
    client.ts
    sync-worker.ts
    file-store.ts
    conflicts.ts
    frontmatter.ts
    agent.ts
    web.tsx
```

## Schema

`kalam/schema.sql` defines one user-scoped metadata table. OKF file content is **not** stored in SQL columns.

The `file_ref` column uses KalamDB's `FILE` datatype so the TypeScript SDK can upload and download bytes with `queryWithFiles()` and file download URLs.

## Sync flow

When a local OKF Markdown file changes:

1. Compute `sha256`
2. Parse OKF YAML frontmatter
3. Upload file bytes first
4. Compare `base_sha256` with server `sha256`
5. Update the canonical row or create a dated conflict copy
6. Live subscribers on other clients receive the metadata update

Conflict policy: the faster writer keeps the canonical path. A stale writer on another device creates `profile.conflict-{timestamp}.md`.

## Files worth reading

- `kalam/schema.sql` — the only schema source of truth
- `src/sync-worker.ts` — local OKF folder watcher and push logic
- `src/web.tsx` — live metadata table UI
- `src/agent.ts` — downloads canonical OKF files for demo answers (not from disk)
- `kalam.toml` — `kalam dev` process wiring

## Validation

```bash
npm test
```

With `kalam dev` running:

```bash
npm run test:e2e
```

## Bob isolation demo

1. Open the UI and switch to **bob**
2. Bob only sees his own `context_files` rows
3. Bob cannot download Alice's `file_ref` values

To sync Bob's OKF folder on another client, run: `KALAM_SYNC_USER=bob npm run dev:sync`
