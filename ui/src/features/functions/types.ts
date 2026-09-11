export type CatalogTypeKind =
  | "implicit_table_row"
  | "topic_payload"
  | "row_alias"
  | "composite"
  | "enum";

export type ProcedureImplementation = "inline" | "module" | "missing";

export type ProcedureStatus = "ready" | "warning" | "error";

export type FunctionDetailTab = "overview" | "logs" | "revisions" | "test";

export const FUNCTION_DETAIL_TABS: readonly FunctionDetailTab[] = [
  "overview",
  "logs",
  "revisions",
  "test",
] as const;

export type ProcedureLogLevel = "debug" | "info" | "warn" | "error";

export type ProcedureLogOrigin = "sql" | "http" | "topic";

interface ResolvedTypeCommon {
  sqlName: string;
  notNull: boolean;
  nonempty: boolean;
}

export interface ResolvedBuiltinType extends ResolvedTypeCommon {
  kind: "builtin";
  builtin: string;
}

export interface ResolvedEnumType extends ResolvedTypeCommon {
  kind: "enum";
  typeId: string;
  values: string[];
}

export interface ResolvedCompositeType extends ResolvedTypeCommon {
  kind: "composite";
  typeId: string;
  fields: ResolvedTypeField[];
}

export interface ResolvedArrayType extends ResolvedTypeCommon {
  kind: "array";
  element: ResolvedKalamType;
}

export interface ResolvedUnknownType extends ResolvedTypeCommon {
  kind: "unknown";
}

export type ResolvedKalamType =
  | ResolvedBuiltinType
  | ResolvedEnumType
  | ResolvedCompositeType
  | ResolvedArrayType
  | ResolvedUnknownType;

export interface ResolvedTypeField {
  name: string;
  type: ResolvedKalamType;
}

export interface ProcedureParameter {
  name: string;
  ordinal: number;
  type: ResolvedKalamType;
}

export interface ProcedureMetadata {
  id: string;
  schema: string;
  name: string;
  signature: string;
  parameters: ProcedureParameter[];
  returnType: ResolvedKalamType;
  returnTypeName: string;
  security: string;
  owner: string;
  comment: string | null;
  implementation: ProcedureImplementation;
  moduleId: string | null;
  revisionId: string | null;
  grants: string;
}

export interface ProcedureListItem extends ProcedureMetadata {
  status: ProcedureStatus;
  lastUpdatedMs: number | null;
  calls24h: number | null;
  errors24h: number | null;
  averageDurationMs: number | null;
}

export interface ProcedureLogRecord {
  timestamp: string;
  nodeId: string;
  executionId: string;
  requestId: string;
  procedureId: string;
  moduleId: string | null;
  revisionId: string | null;
  actor: string;
  origin: string;
  outcome: string;
  channel: string;
  level: string;
  errorCode: string | null;
  message: string | null;
  durationMs: number;
}

export interface ModuleRevision {
  moduleId: string;
  revisionId: string;
  artifactId: string;
  artifactBytes: number;
  contractHash: string;
  createdAtMs: number;
  isCurrent: boolean;
  exports: string[];
}

export interface FunctionModule {
  moduleId: string;
  runtime: string;
  currentRevisionId: string | null;
  contractHash: string | null;
  abiVersion: number;
}

export interface CatalogTypeRow {
  typeId: string;
  namespaceId: string;
  name: string;
  kind: string;
  tableId: string | null;
  sourceTypeId: string | null;
  comment: string | null;
}

export interface CatalogTypeFieldRow {
  typeFieldId: string;
  typeId: string;
  name: string;
  ordinal: number;
  fieldTypeId: string | null;
  typeName: string;
  isArray: boolean;
  notNull: boolean;
  nonempty: boolean;
}

export interface CatalogParameterRow {
  parameterId: string;
  routineId: string;
  name: string;
  ordinal: number;
  typeId: string | null;
  typeName: string;
  isArray: boolean;
  notNull: boolean;
  nonempty: boolean;
}

export interface CatalogProcedureRow {
  procedureId: string;
  schema: string;
  name: string;
  signature: string;
  returnType: string;
  implementation: string;
  moduleId: string | null;
  revisionId: string | null;
  security: string;
  owner: string;
  grants: string;
  comment: string | null;
}

export interface ProcedureCatalogSnapshot {
  procedures: ProcedureMetadata[];
  types: CatalogTypeRow[];
  typeFields: CatalogTypeFieldRow[];
  parameters: CatalogParameterRow[];
  modules: FunctionModule[];
  revisions: ModuleRevision[];
  logs: ProcedureLogRecord[];
}

export function isFunctionDetailTab(value: string | undefined): value is FunctionDetailTab {
  return value === "overview" || value === "logs" || value === "revisions" || value === "test";
}
