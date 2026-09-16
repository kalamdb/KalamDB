import { api, ApiRequestError } from "@/lib/api";
import { getBackendOrigin } from "@/lib/backend-url";
import { getDb } from "@/lib/db";
import { getCurrentToken } from "@/lib/kalam-client";
import {
  system_module_revisions,
  system_module_instances,
  system_modules,
  system_procedure_logs,
  system_procedures,
  system_routine_parameters,
  system_type_fields,
  system_types,
} from "@/lib/schema";
import { and, desc, eq, inArray, or, type SQL } from "drizzle-orm";
import { catalogText, resolveProcedureCatalog } from "@/features/functions/catalog";
import { isBuiltinSqlTypeName } from "@/features/functions/format";
import { toProcedureListItem } from "@/features/functions/status";
import type { SystemModuleInstanceRow, SystemProcedureLogRow, SystemTypeFieldRow, SystemTypeRow } from "@/lib/models";
import type { ProcedureCatalogSnapshot, ProcedureListItem } from "@/features/functions/types";

export interface ProcedureLogFilters {
  procedureId: string;
  moduleId?: string;
  search?: string;
  level?: string;
  origin?: string;
  channel?: string;
  executionId?: string;
  revisionId?: string;
  limit?: number;
}

export interface InvokeProcedureResult {
  ok: boolean;
  statusCode: number;
  durationMs: number;
  body: unknown;
  errorCode: string | null;
  errorMessage: string | null;
  startedAt: string;
}

export interface RollbackModuleResult {
  status: string;
  outcome?: string;
  code?: string;
  message?: string;
}

const CATALOG_SCAN_LIMIT = 1000;

function isNamedCatalogTypeId(typeId: string): boolean {
  return Boolean(typeId) && !isBuiltinSqlTypeName(typeId);
}

export function selectTypeFieldsForCatalog(
  scannedFields: SystemTypeFieldRow[],
  seedTypeIds: Iterable<string>,
): SystemTypeFieldRow[] {
  const fieldsByTypeId = new Map<string, SystemTypeFieldRow[]>();
  for (const row of scannedFields) {
    const typeId = catalogText(row.type_id);
    if (!typeId) {
      continue;
    }
    const existing = fieldsByTypeId.get(typeId);
    if (existing) {
      existing.push(row);
    } else {
      fieldsByTypeId.set(typeId, [row]);
    }
  }

  const selected: SystemTypeFieldRow[] = [];
  const seen = new Set<string>();
  const pending = [...new Set(seedTypeIds)].filter(isNamedCatalogTypeId);

  for (let index = 0; index < pending.length; index += 1) {
    const typeId = pending[index];
    if (seen.has(typeId)) {
      continue;
    }
    seen.add(typeId);
    const rows = fieldsByTypeId.get(typeId);
    if (!rows) {
      continue;
    }
    selected.push(...rows);
    for (const row of rows) {
      const nested = catalogText(row.field_type_id);
      if (nested && !seen.has(nested) && isNamedCatalogTypeId(nested)) {
        pending.push(nested);
      }
    }
  }

  return selected;
}

async function fetchMissingTypes(typeIds: string[], presentTypeIds: Set<string>): Promise<SystemTypeRow[]> {
  const missing = [...new Set(typeIds)].filter(
    (typeId) => isNamedCatalogTypeId(typeId) && !presentTypeIds.has(typeId),
  );
  if (missing.length === 0) {
    return [];
  }
  const db = getDb();
  return db
    .select()
    .from(system_types)
    .where(inArray(system_types.type_id, missing))
    .limit(CATALOG_SCAN_LIMIT);
}

async function fetchTypeFieldsForCatalog(
  seedTypeIds: Iterable<string>,
  scannedFields: SystemTypeFieldRow[],
): Promise<SystemTypeFieldRow[]> {
  const selected = selectTypeFieldsForCatalog(scannedFields, seedTypeIds);
  if (scannedFields.length < CATALOG_SCAN_LIMIT) {
    return selected;
  }

  const selectedTypeIds = new Set(selected.map((row) => catalogText(row.type_id)).filter(Boolean));
  const missing = [...new Set(seedTypeIds)].filter(
    (typeId) => isNamedCatalogTypeId(typeId) && !selectedTypeIds.has(typeId),
  );
  if (missing.length === 0) {
    return selected;
  }

  const db = getDb();
  const extra = await db
    .select()
    .from(system_type_fields)
    .where(inArray(system_type_fields.type_id, missing))
    .limit(CATALOG_SCAN_LIMIT);
  return [...selected, ...selectTypeFieldsForCatalog(extra, missing)];
}

export async function fetchModuleInstances(): Promise<SystemModuleInstanceRow[]> {
  const db = getDb();
  return db.select().from(system_module_instances).limit(CATALOG_SCAN_LIMIT);
}

export async function fetchProcedureCatalog(): Promise<ProcedureCatalogSnapshot> {
  const db = getDb();
  const [procedures, parameters, scannedTypes, scannedTypeFields, modules, revisions, logs] =
    await Promise.all([
      db.select().from(system_procedures).limit(CATALOG_SCAN_LIMIT),
      db.select().from(system_routine_parameters).limit(CATALOG_SCAN_LIMIT),
      db.select().from(system_types).limit(CATALOG_SCAN_LIMIT),
      db.select().from(system_type_fields).limit(CATALOG_SCAN_LIMIT),
      db.select().from(system_modules).limit(CATALOG_SCAN_LIMIT),
      db.select().from(system_module_revisions).limit(CATALOG_SCAN_LIMIT),
      db.select().from(system_procedure_logs).orderBy(desc(system_procedure_logs.timestamp)).limit(200),
    ]);

  const types: SystemTypeRow[] = [...scannedTypes];
  const presentTypeIds = new Set(types.map((type) => catalogText(type.type_id)).filter(Boolean));

  const seedTypeIds = new Set<string>();
  for (const parameter of parameters) {
    const typeId = catalogText(parameter.type_id) || catalogText(parameter.type_name);
    if (typeId && isNamedCatalogTypeId(typeId)) {
      seedTypeIds.add(typeId);
    }
  }
  for (const procedure of procedures) {
    const returnType = catalogText(procedure.return_type);
    if (returnType && returnType.toUpperCase() !== "VOID") {
      const inner = returnType.endsWith("[]") ? returnType.slice(0, -2) : returnType;
      if (isNamedCatalogTypeId(inner)) {
        seedTypeIds.add(inner);
      }
    }
  }

  const extraTypes = await fetchMissingTypes([...seedTypeIds], presentTypeIds);
  types.push(...extraTypes);
  for (const type of extraTypes) {
    const typeId = catalogText(type.type_id);
    if (typeId) {
      presentTypeIds.add(typeId);
    }
  }

  const sourceTypeIds: string[] = [];
  for (const type of types) {
    const typeId = catalogText(type.type_id);
    if (!typeId || !seedTypeIds.has(typeId)) {
      continue;
    }
    const source = catalogText(type.source_type_id);
    if (source && isNamedCatalogTypeId(source)) {
      seedTypeIds.add(source);
      sourceTypeIds.push(source);
    }
  }
  const extraSourceTypes = await fetchMissingTypes(sourceTypeIds, presentTypeIds);
  types.push(...extraSourceTypes);
  for (const type of extraSourceTypes) {
    const typeId = catalogText(type.type_id);
    if (typeId) {
      presentTypeIds.add(typeId);
    }
  }

  const typeFields = await fetchTypeFieldsForCatalog(seedTypeIds, scannedTypeFields);

  return {
    procedures: resolveProcedureCatalog(procedures, parameters, types, typeFields),
    types,
    typeFields,
    parameters,
    modules,
    revisions,
    logs,
  };
}

export function listItemsFromCatalog(
  snapshot: ProcedureCatalogSnapshot,
  nowMs = Date.now(),
): ProcedureListItem[] {
  return snapshot.procedures.map((procedure) =>
    toProcedureListItem(procedure, snapshot.logs, snapshot.revisions, nowMs),
  );
}

export async function fetchProcedureLogs(filters: ProcedureLogFilters): Promise<SystemProcedureLogRow[]> {
  const db = getDb();
  const identity = filters.moduleId
    ? or(
        eq(system_procedure_logs.procedure_id, filters.procedureId),
        and(eq(system_procedure_logs.channel, "lifecycle"), eq(system_procedure_logs.module_id, filters.moduleId)),
      )
    : eq(system_procedure_logs.procedure_id, filters.procedureId);
  const conditions: SQL[] = identity ? [identity] : [eq(system_procedure_logs.procedure_id, filters.procedureId)];

  if (filters.level && filters.level !== "all") {
    conditions.push(eq(system_procedure_logs.level, filters.level));
  }
  if (filters.origin && filters.origin !== "all") {
    conditions.push(eq(system_procedure_logs.origin, filters.origin));
  }
  if (filters.channel && filters.channel !== "all") {
    conditions.push(eq(system_procedure_logs.channel, filters.channel));
  }
  if (filters.executionId?.trim()) {
    conditions.push(eq(system_procedure_logs.execution_id, filters.executionId.trim()));
  }
  if (filters.revisionId?.trim()) {
    conditions.push(eq(system_procedure_logs.revision_id, filters.revisionId.trim()));
  }

  const rows = await db
    .select()
    .from(system_procedure_logs)
    .where(and(...conditions))
    .orderBy(desc(system_procedure_logs.timestamp))
    .limit(filters.limit ?? 200);

  const search = filters.search?.trim().toLowerCase();
  if (!search) {
    return rows;
  }

  return rows.filter((log) => {
    return (
      log.message?.toLowerCase().includes(search) ||
      log.execution_id.toLowerCase().includes(search) ||
      log.revision_id?.toLowerCase().includes(search) ||
      log.error_code?.toLowerCase().includes(search) ||
      log.actor.toLowerCase().includes(search) ||
      log.origin.toLowerCase().includes(search) ||
      log.channel.toLowerCase().includes(search) ||
      log.outcome.toLowerCase().includes(search)
    );
  });
}

export async function invokeProcedure(
  schema: string,
  name: string,
  body: Record<string, unknown>,
): Promise<InvokeProcedureResult> {
  const startedAt = new Date().toISOString();
  const started = performance.now();
  const token = getCurrentToken();
  const headers = new Headers({ "Content-Type": "application/json" });
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  const response = await fetch(
    `${getBackendOrigin()}/v1/functions/${encodeURIComponent(schema)}/${encodeURIComponent(name)}`,
    {
      method: "POST",
      credentials: "include",
      headers,
      body: JSON.stringify(body),
    },
  );
  const durationMs = Math.max(0, Math.round(performance.now() - started));
  const rawText = await response.text();
  let parsed: unknown = null;
  if (rawText.trim()) {
    try {
      parsed = JSON.parse(rawText) as unknown;
    } catch {
      parsed = rawText;
    }
  }

  if (!response.ok) {
    const record = parsed && typeof parsed === "object" ? (parsed as Record<string, unknown>) : {};
    return {
      ok: false,
      statusCode: response.status,
      durationMs,
      body: parsed,
      errorCode: typeof record.code === "string" ? record.code : null,
      errorMessage:
        typeof record.message === "string"
          ? record.message
          : `Request failed with status ${response.status}`,
      startedAt,
    };
  }

  return {
    ok: true,
    statusCode: response.status,
    durationMs,
    body: parsed,
    errorCode: null,
    errorMessage: null,
    startedAt,
  };
}

export async function rollbackModuleRevision(
  moduleId: string,
  revisionId: string,
): Promise<RollbackModuleResult> {
  try {
    return await api.post<RollbackModuleResult>(
      `/functions/modules/${encodeURIComponent(moduleId)}/rollback`,
      { revisionId },
    );
  } catch (error) {
    if (error instanceof ApiRequestError) {
      throw new Error(error.apiError.message || error.message);
    }
    throw error;
  }
}

export function logsForExecution(
  logs: SystemProcedureLogRow[],
  procedureId: string,
  startedAt: string,
  executionId?: string,
): SystemProcedureLogRow[] {
  if (executionId) {
    return logs
      .filter((log) => log.procedure_id === procedureId && log.execution_id === executionId)
      .sort((left, right) => left.timestamp.localeCompare(right.timestamp));
  }

  const startedMs = Date.parse(startedAt);
  const matching = logs.filter((log) => {
    if (log.procedure_id !== procedureId) {
      return false;
    }
    const timestampMs = Date.parse(log.timestamp);
    if (Number.isNaN(timestampMs) || Number.isNaN(startedMs)) {
      return true;
    }
    return timestampMs + 2000 >= startedMs;
  });
  const newestExecution = matching.find((log) => log.outcome === "ok" || log.outcome === "error")
    ?.execution_id;
  if (!newestExecution) {
    return matching.sort((left, right) => left.timestamp.localeCompare(right.timestamp));
  }
  return matching
    .filter((log) => log.execution_id === newestExecution)
    .sort((left, right) => left.timestamp.localeCompare(right.timestamp));
}

export function moduleRuntimeLabel(snapshot: ProcedureCatalogSnapshot, moduleId: string | null): string {
  if (!moduleId) {
    return "TypeScript / V8";
  }
  const module = snapshot.modules.find((item) => item.module_id === moduleId);
  const runtime = module?.runtime?.trim() || "typescript";
  if (runtime.toLowerCase() === "typescript") {
    return "TypeScript / V8";
  }
  return runtime;
}
