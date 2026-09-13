import { describe, expect, it } from "vitest";
import type { SystemModuleInstanceRow } from "@/lib/models";
import { instancesForProcedure, summarizeModuleInstances, utf8ByteLength } from "./runtime";

function instance(overrides: Partial<SystemModuleInstanceRow> = {}): SystemModuleInstanceRow {
  return {
    instance_id: 1,
    worker: 0,
    module_id: "backend",
    revision_id: "backend:abc",
    state: "idle",
    reserved_bytes: 64 * 1024 * 1024,
    used_heap_bytes: 2048,
    peak_heap_bytes: 4096,
    invocations: 3,
    ...overrides,
  };
}

describe("instancesForProcedure", () => {
  it("matches a pinned revision before falling back to the module", () => {
    const rows = [
      instance({ instance_id: 1, revision_id: "backend:abc" }),
      instance({ instance_id: 2, revision_id: "backend:other" }),
      instance({ instance_id: 3, module_id: "inline", revision_id: "inline:deadbeef" }),
    ];

    expect(
      instancesForProcedure(rows, { moduleId: "backend", revisionId: "backend:abc" }).map(
        (row) => row.instance_id,
      ),
    ).toEqual([1]);
    expect(
      instancesForProcedure(rows, { moduleId: "backend", revisionId: null }).map(
        (row) => row.instance_id,
      ),
    ).toEqual([1, 2]);
    expect(
      instancesForProcedure(rows, { moduleId: null, revisionId: "inline:deadbeef" }).map(
        (row) => row.instance_id,
      ),
    ).toEqual([3]);
    expect(instancesForProcedure(rows, { moduleId: null, revisionId: null })).toEqual([]);
  });
});

describe("summarizeModuleInstances", () => {
  it("aggregates heap, reservation, and isolate calls", () => {
    const summary = summarizeModuleInstances([
      instance({ used_heap_bytes: 100, peak_heap_bytes: 400, reserved_bytes: 10, invocations: 2, state: "idle" }),
      instance({
        instance_id: 2,
        used_heap_bytes: 50,
        peak_heap_bytes: 80,
        reserved_bytes: 20,
        invocations: 1,
        state: "active",
      }),
    ]);

    expect(summary).toEqual({
      isolateCount: 2,
      usedHeapBytes: 150,
      peakHeapBytes: 400,
      reservedBytes: 30,
      invocations: 3,
      states: ["idle", "active"],
    });
  });

  it("falls back to used heap when peak is missing", () => {
    expect(summarizeModuleInstances([instance({ peak_heap_bytes: 0, used_heap_bytes: 512 })]).peakHeapBytes).toBe(
      512,
    );
  });
});

describe("utf8ByteLength", () => {
  it("counts UTF-8 bytes", () => {
    expect(utf8ByteLength("abc")).toBe(3);
    expect(utf8ByteLength("é")).toBe(2);
  });
});
