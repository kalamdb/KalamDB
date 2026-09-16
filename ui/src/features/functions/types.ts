import type {
  SystemModuleInstanceRow,
  SystemModuleRevisionRow,
  SystemModuleRow,
  SystemProcedureLogRow,
  SystemProcedureRow,
  SystemRoutineParameterRow,
  SystemTypeFieldRow,
  SystemTypeRow,
} from "@/lib/models";

export type {
  SystemModuleInstanceRow,
  SystemModuleRevisionRow,
  SystemModuleRow,
  SystemProcedureLogRow,
  SystemProcedureRow,
  SystemRoutineParameterRow,
  SystemTypeFieldRow,
  SystemTypeRow,
};

export type CatalogTypeKind =
  | "implicit_table_row"
  | "topic_payload"
  | "row_alias"
  | "composite"
  | "enum";

export type ProcedureImplementation = "inline" | "module" | "missing";

export type ProcedureStatus = "ready" | "unimplemented" | "warning" | "error";

export type FunctionDetailTab = "overview" | "logs" | "revisions" | "test";

export const FUNCTION_DETAIL_TABS: readonly FunctionDetailTab[] = [
  "overview",
  "logs",
  "revisions",
  "test",
] as const;

export type ProcedureLogLevel = "debug" | "info" | "warn" | "error";

export type ProcedureLogOrigin = "sql" | "http" | "topic" | "schedule" | "runtime";

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
  id: SystemProcedureRow["procedure_id"];
  schema: SystemProcedureRow["schema"];
  name: SystemProcedureRow["name"];
  signature: SystemProcedureRow["signature"];
  parameters: ProcedureParameter[];
  returnType: ResolvedKalamType;
  returnTypeName: SystemProcedureRow["return_type"];
  security: SystemProcedureRow["security"];
  owner: SystemProcedureRow["owner"];
  comment: SystemProcedureRow["comment"];
  implementation: ProcedureImplementation;
  moduleId: SystemProcedureRow["module_id"];
  revisionId: SystemProcedureRow["revision_id"];
  grants: SystemProcedureRow["grants"];
  language: SystemProcedureRow["language"];
  source: SystemProcedureRow["source"];
}

export interface ProcedureListItem extends ProcedureMetadata {
  status: ProcedureStatus;
  lastUpdatedMs: number | null;
  calls24h: number | null;
  errors24h: number | null;
  averageDurationMs: number | null;
}

export interface ProcedureCatalogSnapshot {
  procedures: ProcedureMetadata[];
  types: SystemTypeRow[];
  typeFields: SystemTypeFieldRow[];
  parameters: SystemRoutineParameterRow[];
  modules: SystemModuleRow[];
  revisions: SystemModuleRevisionRow[];
  logs: SystemProcedureLogRow[];
}

export function isFunctionDetailTab(value: string | undefined): value is FunctionDetailTab {
  return value === "overview" || value === "logs" || value === "revisions" || value === "test";
}
