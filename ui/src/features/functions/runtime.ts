import type { SystemModuleInstanceRow } from "@/lib/models";
import type { ProcedureMetadata } from "./types";

export function instancesForProcedure(
  instances: SystemModuleInstanceRow[],
  procedure: Pick<ProcedureMetadata, "moduleId" | "revisionId">,
): SystemModuleInstanceRow[] {
  if (procedure.revisionId) {
    return instances.filter((row) => row.revision_id === procedure.revisionId);
  }
  if (procedure.moduleId) {
    return instances.filter((row) => row.module_id === procedure.moduleId);
  }
  return [];
}

export interface ProcedureRuntimeSummary {
  isolateCount: number;
  usedHeapBytes: number;
  peakHeapBytes: number;
  reservedBytes: number;
  invocations: number;
  states: string[];
}

function numeric(value: number | string | null | undefined): number {
  const parsed = typeof value === "number" ? value : Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function summarizeModuleInstances(rows: SystemModuleInstanceRow[]): ProcedureRuntimeSummary {
  const states = [...new Set(rows.map((row) => row.state).filter(Boolean))];
  const usedHeapBytes = rows.reduce((sum, row) => sum + numeric(row.used_heap_bytes), 0);
  const peakHeapBytes = rows.reduce((max, row) => Math.max(max, numeric(row.peak_heap_bytes)), 0);
  return {
    isolateCount: rows.length,
    usedHeapBytes,
    peakHeapBytes: peakHeapBytes > 0 ? peakHeapBytes : usedHeapBytes,
    reservedBytes: rows.reduce((sum, row) => sum + numeric(row.reserved_bytes), 0),
    invocations: rows.reduce((sum, row) => sum + numeric(row.invocations), 0),
    states,
  };
}

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}
