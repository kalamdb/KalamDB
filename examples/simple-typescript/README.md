# Realtime Ops Feed

Smallest useful browser example for the TypeScript SDK.

This app shows three KalamDB basics with very little code:

- SQL reads with `queryAll()`
- live updates with `live()`
- browser auth using `Auth.basic()`

Open it in two tabs, add an event in one tab, and the second tab updates immediately.

## Quick Start

From this folder:

```bash
npm install
kalam dev
```

`kalam dev` starts a local KalamDB server, applies `kalam/schema.sql`, keeps `kalam/migrations/` up to date, regenerates TypeScript schema artifacts, and runs the Vite app.

Open the Vite URL printed in the terminal, usually `http://127.0.0.1:5173`.

Default local credentials are `root` / `kalamdb123` at `http://127.0.0.1:2900`.

## What To Try

1. Open the app in two browser tabs.
2. Submit a new event in one tab.
3. Watch the same row appear in the other tab without refreshing.

## Files Worth Reading

- `kalam.toml`: `kalam dev` project config.
- `kalam/schema.sql`: namespace, table, and seed rows.
- `kalam/migrations/0001_init.sql`: initial migration checked into the repo.
- `src/App.tsx`: complete example logic.
- `tests/realtime.spec.ts`: browser end-to-end test.

## Tests

```bash
npm test
```

The Playwright test runs its own setup helper, opens two pages, submits an event, and confirms both tabs show the same live row.
