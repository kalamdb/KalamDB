import { toMilliseconds } from "@/lib/formatters";
import type {
  ProcedureListItem,
  ProcedureMetadata,
  ProcedureStatus,
  SystemModuleRevisionRow,
  SystemProcedureLogRow,
} from "./types";

const DAY_MS = 24 * 60 * 60 * 1000;
const INVOCATION_OUTCOMES = new Set(["ok", "error"]);

export function parseTimestampMs(value: string | number | null | undefined): number | null {
  if (value === null || value === undefined || value === "") {
    return null;
  }
  const parsed = toMilliseconds(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

export function deriveProcedureStatus(
  procedure: ProcedureMetadata,
  logs: SystemProcedureLogRow[],
  nowMs = Date.now(),
): ProcedureStatus {
  if (procedure.implementation === "missing") {
    return "unimplemented";
  }

  const cutoff = nowMs - DAY_MS;
  const hasRecentError = logs.some((log) => {
    if (log.procedure_id !== procedure.id) {
      return false;
    }
    if (log.outcome !== "error" && log.level.toLowerCase() !== "error") {
      return false;
    }
    const timestampMs = parseTimestampMs(log.timestamp);
    return timestampMs !== null && timestampMs >= cutoff;
  });

  return hasRecentError ? "warning" : "ready";
}

export function currentRevisionForProcedure(
  procedure: ProcedureMetadata,
  revisions: SystemModuleRevisionRow[],
): SystemModuleRevisionRow | null {
  if (!procedure.moduleId) {
    if (!procedure.revisionId) {
      return null;
    }
    return revisions.find((revision) => revision.revision_id === procedure.revisionId) ?? null;
  }

  const moduleRevisions = revisions.filter((revision) => revision.module_id === procedure.moduleId);

  if (procedure.revisionId) {
    const exact =
      moduleRevisions.find((revision) => revision.revision_id === procedure.revisionId) ??
      revisions.find((revision) => revision.revision_id === procedure.revisionId);
    if (exact) {
      return exact;
    }
  }

  const current = moduleRevisions.find((revision) => revision.is_current);
  if (current) {
    return current;
  }

  if (moduleRevisions.length === 0) {
    return null;
  }

  return moduleRevisions.reduce((latest, revision) =>
    (parseTimestampMs(revision.created_at) ?? 0) > (parseTimestampMs(latest.created_at) ?? 0)
      ? revision
      : latest,
  );
}

function latestLogMs(procedureId: string, logs: SystemProcedureLogRow[]): number | null {
  let latest: number | null = null;
  for (const log of logs) {
    if (log.procedure_id !== procedureId) {
      continue;
    }
    const timestampMs = parseTimestampMs(log.timestamp);
    if (timestampMs !== null && (latest === null || timestampMs > latest)) {
      latest = timestampMs;
    }
  }
  return latest;
}

export function toProcedureListItem(
  procedure: ProcedureMetadata,
  logs: SystemProcedureLogRow[],
  revisions: SystemModuleRevisionRow[],
  nowMs = Date.now(),
): ProcedureListItem {
  const cutoff = nowMs - DAY_MS;
  const invocationLogs = logs.filter((log) => {
    if (log.procedure_id !== procedure.id || !INVOCATION_OUTCOMES.has(log.outcome)) {
      return false;
    }
    const timestampMs = parseTimestampMs(log.timestamp);
    return timestampMs !== null && timestampMs >= cutoff;
  });
  const durations = invocationLogs
    .map((log) => log.duration_ms)
    .filter((duration) => Number.isFinite(duration) && duration > 0);
  const currentRevision = currentRevisionForProcedure(procedure, revisions);

  return {
    ...procedure,
    status: deriveProcedureStatus(procedure, logs, nowMs),
    lastUpdatedMs:
      (currentRevision ? parseTimestampMs(currentRevision.created_at) : null) ??
      latestLogMs(procedure.id, logs),
    calls24h: invocationLogs.length,
    errors24h: invocationLogs.filter((log) => log.outcome === "error").length,
    averageDurationMs:
      durations.length === 0
        ? null
        : durations.reduce((sum, duration) => sum + duration, 0) / durations.length,
  };
}

export function summarizeProcedureStatuses(items: ProcedureListItem[]): {
  total: number;
  ready: number;
  unimplemented: number;
  warning: number;
  error: number;
} {
  return items.reduce(
    (summary, item) => {
      summary.total += 1;
      summary[item.status] += 1;
      return summary;
    },
    { total: 0, ready: 0, unimplemented: 0, warning: 0, error: 0 },
  );
}
