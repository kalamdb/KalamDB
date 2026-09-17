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
    "chat.send_message": "missing"
  }
}
```

## Procedure states

| Kind | Meaning |
|------|---------|
| `module` | A named `procedure.<schema>.<method>(handler)` export is bound and included in the compiled registry |
| `inline` | No implemented project binding, and the SQL routine has an inline body |
| `missing` | No implemented project binding and no inline body (including `.unimplemented()` scaffolds) |

Deployment sends only `module` IDs as the revision export set. Empty export set means no module-backed procedures.

## Validation at build

| Check | Failure |
|-------|---------|
| Project-backed SQL routine with no implemented named binding and no inline | recorded as `missing`; CALL returns `PROCEDURE_NOT_IMPLEMENTED` |
| `procedure.<schema>.<method>` identity not in the SQL catalog | unknown export |
| Same procedure identity bound twice | duplicate export |
| Binding is not a top-level `export const` of a generated builder call | malformed export |
| `inline` in manifest but no catalog body | invalid |
| `.unimplemented()` or missing binding treated as a module export | missing export |

## Validation at activation

contractHash vs current catalog snapshot; abiVersion supported; artifactHash matches bytes; procedure set vs catalog routines.
