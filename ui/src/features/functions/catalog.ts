import { isBuiltinSqlTypeName } from "./format";
import type {
  CatalogParameterRow,
  CatalogProcedureRow,
  CatalogTypeFieldRow,
  CatalogTypeKind,
  CatalogTypeRow,
  ProcedureImplementation,
  ProcedureMetadata,
  ProcedureParameter,
  ResolvedKalamType,
  ResolvedTypeField,
} from "./types";

const CATALOG_TYPE_KINDS = new Set<CatalogTypeKind>([
  "implicit_table_row",
  "topic_payload",
  "row_alias",
  "composite",
  "enum",
]);

function unwrapBoolean(value: unknown, fallback = false): boolean {
  if (typeof value === "boolean") {
    return value;
  }
  if (typeof value === "number") {
    return value !== 0;
  }
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    return normalized === "true" || normalized === "1";
  }
  return fallback;
}

function unwrapNumber(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) {
      return parsed;
    }
  }
  return fallback;
}

function unwrapText(value: unknown): string | null {
  if (typeof value === "string") {
    const trimmed = value.trim();
    return trimmed.length > 0 ? trimmed : null;
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    return String(value);
  }
  return null;
}

function requiredText(value: unknown, fallback = ""): string {
  return unwrapText(value) ?? fallback;
}

export function parseCatalogTypeKind(value: string): CatalogTypeKind | null {
  return CATALOG_TYPE_KINDS.has(value as CatalogTypeKind) ? (value as CatalogTypeKind) : null;
}

export function parseImplementation(value: string): ProcedureImplementation {
  if (value === "module" || value === "inline" || value === "missing") {
    return value;
  }
  return "missing";
}

export function normalizeCatalogType(row: {
  type_id: unknown;
  namespace_id: unknown;
  name: unknown;
  kind: unknown;
  table_id?: unknown;
  source_type_id?: unknown;
  comment?: unknown;
}): CatalogTypeRow {
  return {
    typeId: requiredText(row.type_id),
    namespaceId: requiredText(row.namespace_id),
    name: requiredText(row.name),
    kind: requiredText(row.kind),
    tableId: unwrapText(row.table_id),
    sourceTypeId: unwrapText(row.source_type_id),
    comment: unwrapText(row.comment),
  };
}

export function normalizeCatalogTypeField(row: {
  type_field_id: unknown;
  type_id: unknown;
  name: unknown;
  ordinal: unknown;
  field_type_id?: unknown;
  type_name: unknown;
  is_array: unknown;
  not_null: unknown;
  nonempty: unknown;
}): CatalogTypeFieldRow {
  return {
    typeFieldId: requiredText(row.type_field_id),
    typeId: requiredText(row.type_id),
    name: requiredText(row.name),
    ordinal: unwrapNumber(row.ordinal),
    fieldTypeId: unwrapText(row.field_type_id),
    typeName: requiredText(row.type_name),
    isArray: unwrapBoolean(row.is_array),
    notNull: unwrapBoolean(row.not_null),
    nonempty: unwrapBoolean(row.nonempty),
  };
}

export function normalizeCatalogParameter(row: {
  parameter_id: unknown;
  routine_id: unknown;
  name: unknown;
  ordinal: unknown;
  type_id?: unknown;
  type_name: unknown;
  is_array: unknown;
  not_null: unknown;
  nonempty: unknown;
}): CatalogParameterRow {
  return {
    parameterId: requiredText(row.parameter_id),
    routineId: requiredText(row.routine_id),
    name: requiredText(row.name),
    ordinal: unwrapNumber(row.ordinal),
    typeId: unwrapText(row.type_id),
    typeName: requiredText(row.type_name),
    isArray: unwrapBoolean(row.is_array),
    notNull: unwrapBoolean(row.not_null),
    nonempty: unwrapBoolean(row.nonempty),
  };
}

export function normalizeCatalogProcedure(row: {
  procedure_id: unknown;
  schema: unknown;
  name: unknown;
  signature: unknown;
  return_type: unknown;
  implementation: unknown;
  module_id?: unknown;
  revision_id?: unknown;
  security: unknown;
  owner: unknown;
  grants: unknown;
  comment?: unknown;
}): CatalogProcedureRow {
  return {
    procedureId: requiredText(row.procedure_id),
    schema: requiredText(row.schema),
    name: requiredText(row.name),
    signature: requiredText(row.signature),
    returnType: requiredText(row.return_type, "VOID"),
    implementation: requiredText(row.implementation, "missing"),
    moduleId: unwrapText(row.module_id),
    revisionId: unwrapText(row.revision_id),
    security: requiredText(row.security, "INVOKER"),
    owner: requiredText(row.owner),
    grants: requiredText(row.grants),
    comment: unwrapText(row.comment),
  };
}

interface TypeIndex {
  types: Map<string, CatalogTypeRow>;
  fields: Map<string, CatalogTypeFieldRow[]>;
}

function buildTypeIndex(types: CatalogTypeRow[], fields: CatalogTypeFieldRow[]): TypeIndex {
  const typeMap = new Map<string, CatalogTypeRow>();
  for (const type of types) {
    typeMap.set(type.typeId, type);
  }

  const fieldMap = new Map<string, CatalogTypeFieldRow[]>();
  for (const field of fields) {
    const existing = fieldMap.get(field.typeId) ?? [];
    existing.push(field);
    fieldMap.set(field.typeId, existing);
  }
  for (const list of fieldMap.values()) {
    list.sort((left, right) => left.ordinal - right.ordinal);
  }

  return { types: typeMap, fields: fieldMap };
}

function wrapArray(
  element: ResolvedKalamType,
  notNull: boolean,
  nonempty: boolean,
): ResolvedKalamType {
  return {
    kind: "array",
    sqlName: `${formatInnerSqlName(element)}[]`,
    notNull,
    nonempty,
    element,
  };
}

function formatInnerSqlName(type: ResolvedKalamType): string {
  if (type.kind === "array") {
    return type.sqlName;
  }
  return type.sqlName;
}

function resolveNamedType(
  typeId: string,
  index: TypeIndex,
  notNull: boolean,
  nonempty: boolean,
  stack: Set<string>,
): ResolvedKalamType {
  if (stack.has(typeId)) {
    return { kind: "unknown", sqlName: typeId, notNull, nonempty };
  }

  const catalogType = index.types.get(typeId);
  if (!catalogType) {
    if (isBuiltinSqlTypeName(typeId)) {
      return {
        kind: "builtin",
        builtin: typeId.split("(")[0]?.trim().toUpperCase() || typeId.toUpperCase(),
        sqlName: typeId,
        notNull,
        nonempty,
      };
    }
    return { kind: "unknown", sqlName: typeId, notNull, nonempty };
  }

  if (catalogType.sourceTypeId && parseCatalogTypeKind(catalogType.kind) === "row_alias") {
    stack.add(typeId);
    const resolved = resolveNamedType(catalogType.sourceTypeId, index, notNull, nonempty, stack);
    stack.delete(typeId);
    return { ...resolved, sqlName: catalogType.typeId };
  }

  const kind = parseCatalogTypeKind(catalogType.kind);
  const fields = index.fields.get(typeId) ?? [];

  if (kind === "enum") {
    return {
      kind: "enum",
      typeId,
      sqlName: typeId,
      values: fields.map((field) => field.name),
      notNull,
      nonempty,
    };
  }

  if (kind === "composite" || kind === "implicit_table_row" || kind === "topic_payload") {
    stack.add(typeId);
    const resolvedFields: ResolvedTypeField[] = fields.map((field) => ({
      name: field.name,
      type: resolveTypeReference(
        field.fieldTypeId,
        field.typeName,
        field.isArray,
        field.notNull,
        field.nonempty,
        index,
        stack,
      ),
    }));
    stack.delete(typeId);
    return {
      kind: "composite",
      typeId,
      sqlName: typeId,
      fields: resolvedFields,
      notNull,
      nonempty,
    };
  }

  return { kind: "unknown", sqlName: typeId, notNull, nonempty };
}

export function resolveTypeReference(
  typeId: string | null,
  typeName: string,
  isArray: boolean,
  notNull: boolean,
  nonempty: boolean,
  index: TypeIndex,
  stack: Set<string> = new Set(),
): ResolvedKalamType {
  const innerNotNull = isArray ? false : notNull;
  const innerNonempty = isArray ? false : nonempty;
  const resolved = typeId
    ? resolveNamedType(typeId, index, innerNotNull, innerNonempty, stack)
    : isBuiltinSqlTypeName(typeName)
      ? {
          kind: "builtin" as const,
          builtin: typeName.split("(")[0]?.trim().toUpperCase() || typeName.toUpperCase(),
          sqlName: typeName,
          notNull: innerNotNull,
          nonempty: innerNonempty,
        }
      : index.types.has(typeName)
        ? resolveNamedType(typeName, index, innerNotNull, innerNonempty, stack)
        : { kind: "unknown" as const, sqlName: typeName, notNull: innerNotNull, nonempty: innerNonempty };

  if (isArray) {
    return wrapArray(resolved, notNull, nonempty);
  }
  return { ...resolved, notNull, nonempty };
}

function resolveReturnType(returnTypeName: string, index: TypeIndex): ResolvedKalamType {
  const trimmed = returnTypeName.trim();
  if (!trimmed || trimmed.toUpperCase() === "VOID") {
    return { kind: "builtin", builtin: "VOID", sqlName: "VOID", notNull: false, nonempty: false };
  }

  const isArray = trimmed.endsWith("[]");
  const inner = isArray ? trimmed.slice(0, -2) : trimmed;
  return resolveTypeReference(
    index.types.has(inner) ? inner : null,
    inner,
    isArray,
    false,
    false,
    index,
  );
}

export function resolveProcedureMetadata(
  procedure: CatalogProcedureRow,
  parameters: CatalogParameterRow[],
  types: CatalogTypeRow[],
  fields: CatalogTypeFieldRow[],
): ProcedureMetadata {
  const index = buildTypeIndex(types, fields);
  const resolvedParameters: ProcedureParameter[] = parameters
    .filter((parameter) => parameter.routineId === procedure.procedureId)
    .sort((left, right) => left.ordinal - right.ordinal)
    .map((parameter) => ({
      name: parameter.name,
      ordinal: parameter.ordinal,
      type: resolveTypeReference(
        parameter.typeId,
        parameter.typeName,
        parameter.isArray,
        parameter.notNull,
        parameter.nonempty,
        index,
      ),
    }));

  return {
    id: procedure.procedureId,
    schema: procedure.schema,
    name: procedure.name,
    signature: procedure.signature,
    parameters: resolvedParameters,
    returnType: resolveReturnType(procedure.returnType, index),
    returnTypeName: procedure.returnType,
    security: procedure.security,
    owner: procedure.owner,
    comment: procedure.comment,
    implementation: parseImplementation(procedure.implementation),
    moduleId: procedure.moduleId,
    revisionId: procedure.revisionId,
    grants: procedure.grants,
  };
}

export function resolveProcedureCatalog(
  procedures: CatalogProcedureRow[],
  parameters: CatalogParameterRow[],
  types: CatalogTypeRow[],
  fields: CatalogTypeFieldRow[],
): ProcedureMetadata[] {
  const index = buildTypeIndex(types, fields);
  return procedures
    .slice()
    .sort((left, right) => left.procedureId.localeCompare(right.procedureId))
    .map((procedure) => {
      const resolvedParameters: ProcedureParameter[] = parameters
        .filter((parameter) => parameter.routineId === procedure.procedureId)
        .sort((left, right) => left.ordinal - right.ordinal)
        .map((parameter) => ({
          name: parameter.name,
          ordinal: parameter.ordinal,
          type: resolveTypeReference(
            parameter.typeId,
            parameter.typeName,
            parameter.isArray,
            parameter.notNull,
            parameter.nonempty,
            index,
          ),
        }));

      return {
        id: procedure.procedureId,
        schema: procedure.schema,
        name: procedure.name,
        signature: procedure.signature,
        parameters: resolvedParameters,
        returnType: resolveReturnType(procedure.returnType, index),
        returnTypeName: procedure.returnType,
        security: procedure.security,
        owner: procedure.owner,
        comment: procedure.comment,
        implementation: parseImplementation(procedure.implementation),
        moduleId: procedure.moduleId,
        revisionId: procedure.revisionId,
        grants: procedure.grants,
      };
    });
}
