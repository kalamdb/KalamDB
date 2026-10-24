# Decision: application readiness and database stability are release gates

Date: 2026-10-22. Status: accepted specification direction; runtime implementation/evidence pending. Scope: Spec 038 on feat/pg-dialect.

## Context

Generated contracts and efficient binary transport simplify backend wiring but do not by themselves provide secure app setup, correct organization membership, request-scoped server rendering, reliable optimistic UI, atomic booking, backup recovery or database consistency under faults. The prior serialization plan needs explicit product and correctness acceptance, not an unbounded list of extra APIs.

Source review identified concrete test priorities: transaction documentation says direct DML callbacks do not add statement-wide atomicity; React update/delete pending tracking is keyed by Sets; platform documentation is narrower than a claim of universal Dart support. These observations require current-code verification and tests, not an assertion that every related feature is absent or exploitable.

## Decision

Use the existing SDK/auth/query/function/live/store/Raft owners. Add a minimal online path, progressive optional reactive/durable paths, and two executable application fixtures (school and helpdesk/chat). Publish a capability matrix with verified/planned/conditional/deferred status. The code generation contract stays single-source and the client/backend use one selected codec.

Add Gate APP for complete developer flows and Gate STABLE for explicit SQL/transaction/durability guarantees with real-engine fault/history tests. Both are prerequisites of Gate R and final T050. No mock backend or documentation checklist can certify storage, authorization or consensus behavior. The new spec has DB-01..DB-20, DX-01..DX-04 and tasks T094..T121.

Correctness/security/durability remains P0; measured resource efficiency and reduced generic app glue are P1; new zero-copy is optional P2. No historical release compatibility, MessagePack adapter, second ORM/cache/auth engine or arbitrary client workflow runtime is added. Fresh start does not remove the need to restore current-format backups or preserve newly committed data across supported failures.

## Consequences

Declare atomicity/isolation/acknowledgement and table/data-group boundaries for every advertised surface. Fix or reject unsupported atomic operations before effects rather than hiding partial writes behind typed SDK methods. Use positive and negative access tests, exact SQL-result oracles, seeded histories, actual process/storage/cluster faults, hot/cold consistency and independent restore drills. Repeatable failures become regression fixtures.

Prototype only the minimal application integrations needed for named fixtures; explicitly defer unrelated platform products. Measure handwritten infrastructure and round trips under equivalent auth/error/recovery scope. Competitive superiority requires reproducible evidence, not another platform's marketing claim or a smaller happy-path snippet.

References: [readiness](../../../specs/038-unified-sdk-contracts/app-readiness.md), [application contract](../../../specs/038-unified-sdk-contracts/contracts/application-development.md), [stability contract](../../../specs/038-unified-sdk-contracts/contracts/database-stability.md), [platform research](../../../specs/038-unified-sdk-contracts/research-app-platforms.md), [task continuation](../../../specs/038-unified-sdk-contracts/tasks-app-stability.md).
