import { api, ApiRequestError } from "@/lib/api";
import { getBackendOrigin } from "@/lib/backend-url";
import { getDb } from "@/lib/db";
import { getCurrentToken } from "@/lib/kalam-client";
import {
  system_module_revisions,
  system_modules,
  system_procedure_logs,
  system_procedures,
  system_routine_parameters,
  system_type_fields,
  system_types,
} from "@/lib/schema";
import { and, desc, eq, type SQL } from "drizzle-orm";
import {
  normalizeCatalogParameter,
  normalizeCatalogProcedure,
  normalizeCatalogType,
  normalizeCatalogTypeField,
  resolveProcedureCatalog,
} from "@/features/functions/catalog";
import { toProcedureListItem } from "@/features/functions/status";
import type {
  CatalogParameterRow,
  CatalogProcedureRow,
  CatalogTypeFieldRow,
  CatalogTypeRow,
  FunctionModule,
  ModuleRevision,
  ProcedureCatalogSnapshot,
  ProcedureListItem,
  ProcedureLogRecord,
} from "@/features/functions/types";

export interface ProcedureLogFilters {
  procedureId: string;
  search?: string;
  level?: string;
  origin?: string;
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

function unwrapBoolean(value: unknown): boolean {
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
  return false;
}

function splitExports(value: string): string[] {
  if (!value.trim()) {
    return [];
  }
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function normalizeProcedureLog(row: {
  timestamp: unknown;
  node_id: unknown;
  execution_id: unknown;
  request_id: unknown;
  procedure_id: unknown;
  module_id?: unknown;
  revision_id?: unknown;
  actor: unknown;
  origin: unknown;
  outcome: unknown;
  channel: unknown;
  level: unknown;
  error_code?: unknown;
  message?: unknown;
  duration_ms: unknown;
}): ProcedureLogRecord {
  return {
    timestamp: unwrapText(row.timestamp) ?? "",
    nodeId: unwrapText(row.node_id) ?? "",
    executionId: unwrapText(row.execution_id) ?? "",
    requestId: unwrapText(row.request_id) ?? "",
    procedureId: unwrapText(row.procedure_id) ?? "",
    moduleId: unwrapText(row.module_id),
    revisionId: unwrapText(row.revision_id),
    actor: unwrapText(row.actor) ?? "",
    origin: unwrapText(row.origin) ?? "",
    outcome: unwrapText(row.outcome) ?? "",
    channel: unwrapText(row.channel) ?? "",
    level: unwrapText(row.level) ?? "info",
    errorCode: unwrapText(row.error_code),
    message: unwrapText(row.message),
    durationMs: unwrapNumber(row.duration_ms),
  };
}

export function normalizeModuleRevision(row: {
  module_id: unknown;
  revision_id: unknown;
  artifact_id: unknown;
  artifact_bytes: unknown;
  contract_hash: unknown;
  created_at: unknown;
  is_current: unknown;
  exports: unknown;
}): ModuleRevision {
  return {
    moduleId: unwrapText(row.module_id) ?? "",
    revisionId: unwrapText(row.revision_id) ?? "",
    artifactId: unwrapText(row.artifact_id) ?? "",
    artifactBytes: unwrapNumber(row.artifact_bytes),
    contractHash: unwrapText(row.contract_hash) ?? "",
    createdAtMs: unwrapNumber(row.created_at),
    isCurrent: unwrapBoolean(row.is_current),
    exports: splitExports(unwrapText(row.exports) ?? ""),
  };
}

export function normalizeFunctionModule(row: {
  module_id: unknown;
  runtime: unknown;
  current_revision_id?: unknown;
  contract_hash?: unknown;
  abi_version: unknown;
}): FunctionModule {
  return {
    moduleId: unwrapText(row.module_id) ?? "",
    runtime: unwrapText(row.runtime) ?? "typescript",
    currentRevisionId: unwrapText(row.current_revision_id),
    contractHash: unwrapText(row.contract_hash),
    abiVersion: unwrapNumber(row.abi_version),
  };
}

export async function fetchProcedureCatalog(): Promise<ProcedureCatalogSnapshot> {
  const db = getDb();
  const [procedureRows, parameterRows, typeRows, fieldRows, moduleRows, revisionRows, logRows] =
    await Promise.all([
      db.select().from(system_procedures),
      db.select().from(system_routine_parameters),
      db.select().from(system_types),
      db.select().from(system_type_fields),
      db.select().from(system_modules),
      db.select().from(system_module_revisions),
      db.select().from(system_procedure_logs).orderBy(desc(system_procedure_logs.timestamp)).limit(200),
    ]);

  const procedures: CatalogProcedureRow[] = procedureRows.map(normalizeCatalogProcedure);
  const parameters: CatalogParameterRow[] = parameterRows.map(normalizeCatalogParameter);
  const types: CatalogTypeRow[] = typeRows.map(normalizeCatalogType);
  const typeFields: CatalogTypeFieldRow[] = fieldRows.map(normalizeCatalogTypeField);
  const modules = moduleRows.map(normalizeFunctionModule);
  const revisions = revisionRows.map(normalizeModuleRevision);
  const logs = logRows.map(normalizeProcedureLog);

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

export async function fetchProcedureLogs(filters: ProcedureLogFilters): Promise<ProcedureLogRecord[]> {
  const db = getDb();
  const conditions: SQL[] = [eq(system_procedure_logs.procedure_id, filters.procedureId)];

  if (filters.level && filters.level !== "all") {
    conditions.push(eq(system_procedure_logs.level, filters.level));
  }
  if (filters.origin && filters.origin !== "all") {
    conditions.push(eq(system_procedure_logs.origin, filters.origin));
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
  const logs = rows.map(normalizeProcedureLog);
  if (!search) {
    return logs;
  }

  return logs.filter((log) => {
    return (
      log.message?.toLowerCase().includes(search) ||
      log.executionId.toLowerCase().includes(search) ||
      log.revisionId?.toLowerCase().includes(search) ||
      log.errorCode?.toLowerCase().includes(search) ||
      log.actor.toLowerCase().includes(search)
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
  logs: ProcedureLogRecord[],
  procedureId: string,
  startedAt: string,
  executionId?: string,
): ProcedureLogRecord[] {
  if (executionId) {
    return logs
      .filter((log) => log.procedureId === procedureId && log.executionId === executionId)
      .sort((left, right) => left.timestamp.localeCompare(right.timestamp));
  }

  const startedMs = Date.parse(startedAt);
  const matching = logs.filter((log) => {
    if (log.procedureId !== procedureId) {
      return false;
    }
    const timestampMs = Date.parse(log.timestamp);
    if (Number.isNaN(timestampMs) || Number.isNaN(startedMs)) {
      return true;
    }
    return timestampMs + 2000 >= startedMs;
  });
  const newestExecution = matching.find((log) => log.outcome === "ok" || log.outcome === "error")
    ?.executionId;
  if (!newestExecution) {
    return matching.sort((left, right) => left.timestamp.localeCompare(right.timestamp));
  }
  return matching
    .filter((log) => log.executionId === newestExecution)
    .sort((left, right) => left.timestamp.localeCompare(right.timestamp));
}

export function moduleRuntimeLabel(snapshot: ProcedureCatalogSnapshot, moduleId: string | null): string {
  if (!moduleId) {
    return "TypeScript / V8";
  }
  const module = snapshot.modules.find((item) => item.moduleId === moduleId);
  const runtime = module?.runtime?.trim() || "typescript";
  if (runtime.toLowerCase() === "typescript") {
    return "TypeScript / V8";
  }
  return runtime;
}
