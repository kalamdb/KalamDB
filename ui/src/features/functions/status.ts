import type {
  ModuleRevision,
  ProcedureListItem,
  ProcedureLogRecord,
  ProcedureMetadata,
  ProcedureStatus,
} from "./types";

const DAY_MS = 24 * 60 * 60 * 1000;
const INVOCATION_OUTCOMES = new Set(["ok", "error"]);

function parseTimestampMs(timestamp: string): number | null {
  const parsed = Date.parse(timestamp);
  return Number.isNaN(parsed) ? null : parsed;
}

export function deriveProcedureStatus(
  procedure: ProcedureMetadata,
  logs: ProcedureLogRecord[],
  nowMs = Date.now(),
): ProcedureStatus {
  if (procedure.implementation === "missing") {
    return "error";
  }

  const cutoff = nowMs - DAY_MS;
  const hasRecentError = logs.some((log) => {
    if (log.procedureId !== procedure.id) {
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
  revisions: ModuleRevision[],
): ModuleRevision | null {
  if (procedure.revisionId) {
    return revisions.find((revision) => revision.revisionId === procedure.revisionId) ?? null;
  }
  if (!procedure.moduleId) {
    return null;
  }
  return (
    revisions.find((revision) => revision.moduleId === procedure.moduleId && revision.isCurrent) ??
    null
  );
}

export function toProcedureListItem(
  procedure: ProcedureMetadata,
  logs: ProcedureLogRecord[],
  revisions: ModuleRevision[],
  nowMs = Date.now(),
): ProcedureListItem {
  const cutoff = nowMs - DAY_MS;
  const invocationLogs = logs.filter((log) => {
    if (log.procedureId !== procedure.id || !INVOCATION_OUTCOMES.has(log.outcome)) {
      return false;
    }
    const timestampMs = parseTimestampMs(log.timestamp);
    return timestampMs !== null && timestampMs >= cutoff;
  });
  const durations = invocationLogs
    .map((log) => log.durationMs)
    .filter((duration) => Number.isFinite(duration) && duration > 0);
  const currentRevision = currentRevisionForProcedure(procedure, revisions);

  return {
    ...procedure,
    status: deriveProcedureStatus(procedure, logs, nowMs),
    lastUpdatedMs: currentRevision?.createdAtMs ?? null,
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
  warning: number;
  error: number;
} {
  return items.reduce(
    (summary, item) => {
      summary.total += 1;
      summary[item.status] += 1;
      return summary;
    },
    { total: 0, ready: 0, warning: 0, error: 0 },
  );
}
