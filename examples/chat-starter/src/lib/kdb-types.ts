// Shared mutation-helper types for the consumer side of <LiveQueries>.
//
// The SDK's exported generics for ctx.insert / ctx.update are pinned to a
// specific LiveQueriesDefinition shape that's awkward to thread through
// every component prop type. These aliases drop down a level — accept a
// drizzle Table reference + a `Record<string, unknown>` payload — which is
// what the call sites actually pass.
//
// One source of truth: re-import these from App.tsx, ChatBody, and
// Conversation.tsx. If the SDK ever publishes a friendlier mutation type
// we can swap it in here once.

import type { Table } from "drizzle-orm";

export type InsertFn = <T extends Table>(
  table: T,
) => { values: (row: Record<string, unknown>) => Promise<unknown> };

export type UpdateFn = <T extends Table>(
  table: T,
  id: string,
) => { set: (patch: Record<string, unknown>) => Promise<unknown> };
