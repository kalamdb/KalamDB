import type {
  SystemProcedureRow,
  SystemRoutineParameterRow,
  SystemTypeFieldRow,
  SystemTypeRow,
} from "@/lib/models";
import { isBuiltinSqlTypeName } from "./format";
import type {
  CatalogTypeKind,
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

function normalizeKindToken(value: string): string {
  return value
    .trim()
    .replace(/^["']|["']$/g, "")
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/[\s-]+/g, "_")
    .toLowerCase();
}

export function parseCatalogTypeKind(value: unknown): CatalogTypeKind | null {
  const normalized = normalizeKindToken(catalogText(value));
  if (!normalized) {
    return null;
  }
  return CATALOG_TYPE_KINDS.has(normalized as CatalogTypeKind) ? (normalized as CatalogTypeKind) : null;
}

export function parseImplementation(value: unknown): ProcedureImplementation {
  const normalized = catalogText(value);
  if (normalized === "module" || normalized === "inline" || normalized === "missing") {
    return normalized;
  }
  return "missing";
}

interface TypeIndex {
  types: Map<string, SystemTypeRow>;
  fields: Map<string, SystemTypeFieldRow[]>;
}

export function catalogText(value: unknown): string {
  if (value == null) {
    return "";
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") {
    return String(value);
  }
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (typeof record.asString === "function") {
      const text = record.asString();
      if (typeof text === "string" && text !== "NULL") {
        return text;
      }
      if (text === null) {
        return "";
      }
    }
    if (typeof record.toJson === "function") {
      return catalogText(record.toJson());
    }
    if (typeof record.Utf8 === "string") {
      return record.Utf8;
    }
    if (typeof record.String === "string") {
      return record.String;
    }
    if (typeof record.toString === "function") {
      const text = record.toString();
      if (typeof text === "string" && text !== "[object Object]") {
        return text;
      }
    }
  }
  return "";
}

function catalogBool(value: unknown): boolean {
  if (typeof value === "boolean") {
    return value;
  }
  const text = catalogText(value).toLowerCase();
  return text === "true" || text === "1";
}

function catalogNumber(value: unknown): number {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  const parsed = Number(catalogText(value));
  return Number.isFinite(parsed) ? parsed : 0;
}

function looksLikeEnumFields(fields: SystemTypeFieldRow[]): boolean {
  return (
    fields.length > 0 &&
    fields.every((field) => {
      const fieldTypeId = catalogText(field.field_type_id);
      const name = catalogText(field.name);
      const typeName = catalogText(field.type_name);
      return !fieldTypeId && Boolean(name) && typeName === name;
    })
  );
}

function buildTypeIndex(types: SystemTypeRow[], fields: SystemTypeFieldRow[]): TypeIndex {
  const typeMap = new Map<string, SystemTypeRow>();
  for (const type of types) {
    const typeId = catalogText(type.type_id);
    const namespaceId = catalogText(type.namespace_id);
    const name = catalogText(type.name);
    if (typeId) {
      typeMap.set(typeId, type);
    }
    if (namespaceId && name) {
      typeMap.set(`${namespaceId}.${name}`, type);
    }
    if (name && !typeMap.has(name)) {
      typeMap.set(name, type);
    }
  }

  const fieldMap = new Map<string, SystemTypeFieldRow[]>();
  for (const field of fields) {
    const typeId = catalogText(field.type_id);
    if (!typeId) {
      continue;
    }
    const existing = fieldMap.get(typeId) ?? [];
    existing.push(field);
    fieldMap.set(typeId, existing);
  }
  for (const list of fieldMap.values()) {
    list.sort((left, right) => catalogNumber(left.ordinal) - catalogNumber(right.ordinal));
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

  const lookupId = catalogText(typeId) || typeId;
  const catalogType = index.types.get(lookupId);
  const orphanFields = index.fields.get(lookupId) ?? [];
  if (!catalogType) {
    if (looksLikeEnumFields(orphanFields)) {
      return {
        kind: "enum",
        typeId: lookupId,
        sqlName: lookupId,
        values: orphanFields.map((field) => catalogText(field.name)),
        notNull,
        nonempty,
      };
    }
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

  const catalogTypeId = catalogText(catalogType.type_id) || typeId;
  if (catalogText(catalogType.source_type_id) && parseCatalogTypeKind(catalogType.kind) === "row_alias") {
    stack.add(typeId);
    const resolved = resolveNamedType(catalogText(catalogType.source_type_id), index, notNull, nonempty, stack);
    stack.delete(typeId);
    return { ...resolved, sqlName: catalogTypeId };
  }

  const kind = parseCatalogTypeKind(catalogType.kind);
  const fields = index.fields.get(catalogTypeId) ?? [];

  if (kind === "enum" || (!kind && looksLikeEnumFields(fields))) {
    return {
      kind: "enum",
      typeId: catalogTypeId,
      sqlName: catalogTypeId,
      values: fields.map((field) => catalogText(field.name)),
      notNull,
      nonempty,
    };
  }

  if (kind === "composite" || kind === "implicit_table_row" || kind === "topic_payload") {
    stack.add(typeId);
    const resolvedFields: ResolvedTypeField[] = fields.map((field) => ({
      name: catalogText(field.name) || field.name,
      type: resolveTypeReference(
        field.field_type_id,
        field.type_name,
        catalogBool(field.is_array),
        catalogBool(field.not_null),
        catalogBool(field.nonempty),
        index,
        stack,
      ),
    }));
    stack.delete(typeId);
    return {
      kind: "composite",
      typeId: catalogTypeId,
      sqlName: catalogTypeId,
      fields: resolvedFields,
      notNull,
      nonempty,
    };
  }

  return { kind: "unknown", sqlName: catalogTypeId, notNull, nonempty };
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
  const namedTypeId = catalogText(typeId);
  const namedTypeName = catalogText(typeName) || typeName;
  const resolved = namedTypeId
    ? resolveNamedType(namedTypeId, index, innerNotNull, innerNonempty, stack)
    : isBuiltinSqlTypeName(namedTypeName)
      ? {
          kind: "builtin" as const,
          builtin: namedTypeName.split("(")[0]?.trim().toUpperCase() || namedTypeName.toUpperCase(),
          sqlName: namedTypeName,
          notNull: innerNotNull,
          nonempty: innerNonempty,
        }
      : index.types.has(namedTypeName)
        ? resolveNamedType(namedTypeName, index, innerNotNull, innerNonempty, stack)
        : { kind: "unknown" as const, sqlName: namedTypeName, notNull: innerNotNull, nonempty: innerNonempty };

  if (isArray) {
    return wrapArray(resolved, notNull, nonempty);
  }
  return { ...resolved, notNull, nonempty };
}

function resolveReturnType(returnTypeName: unknown, index: TypeIndex): ResolvedKalamType {
  const trimmed = catalogText(returnTypeName).trim();
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

function metadataFromRows(
  procedure: SystemProcedureRow,
  parameters: SystemRoutineParameterRow[],
  index: TypeIndex,
): ProcedureMetadata {
  const resolvedParameters: ProcedureParameter[] = parameters
    .filter((parameter) => catalogText(parameter.routine_id) === catalogText(procedure.procedure_id))
    .sort((left, right) => catalogNumber(left.ordinal) - catalogNumber(right.ordinal))
    .map((parameter) => ({
      name: catalogText(parameter.name) || parameter.name,
      ordinal: catalogNumber(parameter.ordinal),
      type: resolveTypeReference(
        parameter.type_id,
        parameter.type_name,
        catalogBool(parameter.is_array),
        catalogBool(parameter.not_null),
        catalogBool(parameter.nonempty),
        index,
      ),
    }));

  return {
    id: catalogText(procedure.procedure_id) || procedure.procedure_id,
    schema: catalogText(procedure.schema) || procedure.schema,
    name: catalogText(procedure.name) || procedure.name,
    signature: catalogText(procedure.signature) || procedure.signature,
    parameters: resolvedParameters,
    returnType: resolveReturnType(procedure.return_type || "VOID", index),
    returnTypeName: catalogText(procedure.return_type) || procedure.return_type || "VOID",
    security: catalogText(procedure.security) || procedure.security,
    owner: catalogText(procedure.owner) || procedure.owner,
    comment: procedure.comment == null ? null : catalogText(procedure.comment) || procedure.comment,
    implementation: parseImplementation(procedure.implementation),
    moduleId: procedure.module_id == null ? null : catalogText(procedure.module_id) || procedure.module_id,
    revisionId: procedure.revision_id == null ? null : catalogText(procedure.revision_id) || procedure.revision_id,
    grants: catalogText(procedure.grants) || procedure.grants,
    language: procedure.language == null ? null : catalogText(procedure.language) || procedure.language,
    source: procedure.source == null ? null : catalogText(procedure.source) || procedure.source,
  };
}

export function resolveProcedureMetadata(
  procedure: SystemProcedureRow,
  parameters: SystemRoutineParameterRow[],
  types: SystemTypeRow[],
  fields: SystemTypeFieldRow[],
): ProcedureMetadata {
  return metadataFromRows(procedure, parameters, buildTypeIndex(types, fields));
}

export function resolveProcedureCatalog(
  procedures: SystemProcedureRow[],
  parameters: SystemRoutineParameterRow[],
  types: SystemTypeRow[],
  fields: SystemTypeFieldRow[],
): ProcedureMetadata[] {
  const index = buildTypeIndex(types, fields);
  return procedures
    .slice()
    .sort((left, right) => catalogText(left.procedure_id).localeCompare(catalogText(right.procedure_id)))
    .map((procedure) => metadataFromRows(procedure, parameters, index));
}
