# Summarizer Agent

Worker-only example for `@kalamdb/consumer`.

It demonstrates the smallest useful topic consumer flow:

1. a row lands in `blog.blogs`
2. KalamDB routes the change into `blog.summarizer`
3. the worker consumes the topic
4. the worker writes `summary` back into the same row
5. failures are stored in `blog.summary_failures`

There is no model dependency here. The summarizer is deterministic so the example is easy to read and the test is stable.

Use `change.data` as the blog row inside `onChange()`. This example routes changes from `blog.blogs`, which is a `SHARED` table, so `change.user` is expected to be `undefined`.

## Quick Start

From this folder:

```bash
npm install
kalam dev
```

`kalam dev` starts a local KalamDB server, applies `kalam/schema.sql`, keeps `kalam/migrations/` up to date, and runs the worker with `npm run start`.

Default local credentials are `root` / `kalamdb123` at `http://127.0.0.1:2900`.

## Files Worth Reading

- `kalam.toml`: `kalam dev` project config.
- `kalam/schema.sql`: schema, topic route, and one seed blog row.
- `kalam/migrations/0001_init.sql`: initial migration checked into the repo.
- `src/agent.ts`: full worker logic.
- `tests/summarizer.integration.test.ts`: end-to-end verification.

## Tests

With a KalamDB server available:

```bash
npm test
```

The integration test starts the agent, inserts a blog row, and waits until the summary field is written back by the worker.
