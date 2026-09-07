# Function build manifest

Written next to the compiled artifact (`functions/.kalam/build/manifest.json`). Exact field names may snake_case in Rust; JSON uses the keys below.

```json
{
  "manifestVersion": 1,
  "module": "backend",
  "runtime": "javascript",
  "abiVersion": 2,
  "contractHash": "...",
  "artifactHash": "...",
  "sourceHash": "...",
  "lockfileHash": "...",
  "procedures": {
    "api.create_order": "module",
    "api.health": "inline",
    "chat.send_message": "module"
  }
}
```

## Validation at build

| Check | Failure |
|-------|---------|
| Project-backed SQL routine with no file and no inline | missing export |
| File/export with no SQL routine | unknown export |
| `inline` in manifest but no catalog body | invalid |
| `module` but registry missing default export | missing export |

## Validation at activation

contractHash vs current catalog snapshot; abiVersion supported; artifactHash matches bytes; procedure set vs catalog routines.
