# Data Model: Schema-First Functions Runtime (V1.1)

## CatalogRoutine

Persisted `system.routines` row. SQL contract plus optional inline implementation.

| Field | Type | Notes |
|-------|------|--------|
| routine_id | RoutineId | PK, schema-qualified |
| namespace_id | NamespaceId | |
| name | String | Unqualified |
| owner | UserId | DEFINER principal |
| security | RoutineSecurityMode | Invoker / Definer |
| language | Option<String> | Set **only** when inline body exists (`JAVASCRIPT` / `TYPESCRIPT` / existing SQL) |
| body | Option<String> | Raw inline source (introspection) |
| inline_source_hash | Option<String> | SHA-256 of body; **new** |
| inline_artifact_id | Option<ArtifactId> | Compiled JS for inline JS; **new**. TS inline has source but no executable artifact until project build |
| return_type_id / name / flags | existing | Unchanged |
| comment | Option<String> | Unchanged |

**Validation**: LANGUAGE required iff body present. Project-backed procedures may have language/body null. Do not create `FunctionModule` rows from inline DDL.

## CatalogRoutineParameter / CatalogRoutineGrant

Unchanged structurally. Grants: add **`RoutineGrantee::Anonymous`**. `PUBLIC` does **not** match `Role::Anonymous`.

## CatalogFunctionModule

One row per **project** module (typically `backend`), not per procedure.

| Field | Type | Notes |
|-------|------|--------|
| module_id | FunctionModuleId | From `[functions] module` |
| runtime | FunctionRuntime | `javascript` / `typescript` (compiled JS) |
| active_revision_id | Option<FunctionRevisionId> | CAS target |

## CatalogFunctionRevision

Immutable history.

| Field | Type | Notes |
|-------|------|--------|
| revision_id | FunctionRevisionId | PK |
| module_id | FunctionModuleId | |
| artifact_id | ArtifactId | Content-addressed |
| contract_hash | String | Must match catalog snapshot at activation |
| abi_version | u32 | Must be 2 for this feature |
| source_hash / lockfile_hash | optional | From manifest |
| status | derived | `active` if pointed by module; else `ready` |

**State**: `ready` → `active` (CAS) → `ready` (superseded). Never mutate artifact bytes. Rollback is pointer swap `active` A → `ready` B becomes `active`.

## CatalogFunctionArtifact

Unchanged content-addressed blob metadata. Bytes in filestore. Load **once** per revision into `Arc<ModuleRevision>`.

## BuildManifest (not catalog — artifact sidecar)

See [contracts/build-manifest.md](contracts/build-manifest.md). Maps each routine to `module` | `inline`. Used to build `ActiveFunctionSet`.

## ActiveFunctionSet (in-memory)

| Field | Type | Notes |
|-------|------|--------|
| generation | u64 | Monotonic publish counter |
| contract_hash | ContractHash | |
| module_revision | Option<Arc<ModuleRevision>> | Project artifact |
| procedures | HashMap<RoutineId, ImplementationRef> | Resolved at publish |

```text
ImplementationRef =
  Module { revision: Arc<ModuleRevision>, procedure: ProcedureSlot }
  | Inline { artifact: Arc<InlineArtifact> }
  | Missing
```

**Publish**: catalog CAS success → rebuild set → `ArcSwap::store`. Roots hold `Arc<ActiveFunctionSet>` for life.

## FunctionExecutionRoot (in-memory, per request)

| Field | Notes |
|-------|--------|
| execution_id / request_id | Observability |
| actor | Immutable UserId + Role |
| origin | Sql / Http / Topic / Schedule / Wire |
| namespace | NamespaceId |
| deadline / cancellation | Covers JS, host, DB, nested, queue |
| deployment | Arc<ActiveFunctionSet> |
| transaction | Shared scope; lazy write begin if safe |
| http | Option<Arc<HttpInvocationContext>> |
| sql_sessions | PrincipalSessionCache SmallVec |
| lease | RuntimeLease (lane + isolate) — roots only |

**Lifecycle**: create → admit (queue/active/memory) → checkout lease → run → commit/rollback → release lease → drop active-run row.

## ProcedureFrame (in-memory)

`SmallVec<[ProcedureFrame; 4]>`, max depth 16.

| Field | Notes |
|-------|--------|
| routine_id | |
| effective_principal | UserId + Role |
| security | Invoker / Definer |

Copying a tiny stack for a concurrent nested branch is allowed. Nested calls do **not** clone a new root or HostSession `Vec` as the primary design (today they do).

## HttpInvocationContext

| Field | Notes |
|-------|--------|
| method / path | |
| headers | `http::HeaderMap` or Actix equivalent, `Arc` |
| query | lazy |
| response | Mutex; **root-only** mutation; validated names/sizes |

Non-HTTP origins: `http = None`.

## RuntimeInstance / RuntimeLease

Keyed by `(lane_id, revision_id)`. States: `created` → `busy` → `idle` (if heap < soft and cleanup proven) or `destroyed`. Idle TTL / max age / max invocations per instance. Unrelated revisions never `rebind()`.

## RuntimeValueBuffer

In-memory FlatBuffer tagged with contract/signature hash. Shared via `bytes::Bytes`. Nested call clones the `Bytes` (refcounted), not the payload. Invalid hash → re-encode or error.

## system.active_function_runs (virtual)

In-memory map keyed by `execution_id`. Columns: execution_id, request_id, routine_id, revision_id, actor, principal, origin, started_at, depth. Insert on root start, delete on finish. **No** RocksDB write.

## Relationships

```text
CatalogFunctionModule 1──* CatalogFunctionRevision *──1 CatalogFunctionArtifact
CatalogRoutine 0..1 inline artifact
ActiveFunctionSet ── resolves ──* CatalogRoutine → ImplementationRef
FunctionExecutionRoot 1──* ProcedureFrame
FunctionExecutionRoot 1──1 RuntimeLease ──1 RuntimeInstance
FunctionExecutionRoot 1──1 Arc<ActiveFunctionSet>
```

## Validation rules

- Activation refuses mismatched contract hash, ABI, missing artifact, or export set vs catalog routines.
- Rollback refuses missing artifact, unsupported ABI, incompatible contract vs current schema.
- Missing implementation → `PROCEDURE_NOT_IMPLEMENTED` (not filesystem error).
- Input/output/host-result bytes count toward limits **before** admission/queue.
