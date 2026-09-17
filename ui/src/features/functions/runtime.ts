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

export interface MemorySlice {
  key: "used" | "peak" | "headroom";
  name: string;
  value: number;
  color: string;
}

/// Splits a reservation into live heap, the extra the isolate has needed before (peak - used),
/// and what is reserved but never touched. Slices never overlap so they sum to `reservedBytes`.
export function memorySlices(summary: Pick<ProcedureRuntimeSummary, "usedHeapBytes" | "peakHeapBytes" | "reservedBytes">): MemorySlice[] {
  const used = Math.max(0, summary.usedHeapBytes);
  const peak = Math.max(used, summary.peakHeapBytes);
  const reserved = Math.max(peak, summary.reservedBytes);
  return [
    { key: "used", name: "Live heap", value: used, color: "#0f766e" },
    { key: "peak", name: "Peak above live", value: peak - used, color: "#f59e0b" },
    { key: "headroom", name: "Unused reservation", value: reserved - peak, color: "#cbd5e1" },
  ];
}

export function utilizationPercent(usedBytes: number, reservedBytes: number): number | null {
  if (reservedBytes <= 0) {
    return null;
  }
  return Math.min(100, Math.round((Math.max(0, usedBytes) / reservedBytes) * 100));
}

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}
