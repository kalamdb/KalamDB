import { describe, expect, it } from "vitest";
import { logsForExecution } from "./functionsService";
import type { ProcedureLogRecord } from "@/features/functions/types";

function log(overrides: Partial<ProcedureLogRecord>): ProcedureLogRecord {
  return {
    timestamp: "2026-09-11T12:42:10.102Z",
    nodeId: "n1",
    executionId: "01KABC",
    requestId: "01KABC",
    procedureId: "api.create_order",
    moduleId: "backend",
    revisionId: "backend:84ac91",
    actor: "root",
    origin: "http",
    outcome: "log",
    channel: "ctx.log",
    level: "info",
    errorCode: null,
    message: "starting create_order",
    durationMs: 0,
    ...overrides,
  };
}

describe("logsForExecution", () => {
  it("returns logs for a known execution id in chronological order", () => {
    const logs = [
      log({ timestamp: "2026-09-11T12:42:10.114Z", message: "order created", outcome: "ok", durationMs: 12 }),
      log({ timestamp: "2026-09-11T12:42:10.102Z", message: "starting create_order" }),
      log({
        timestamp: "2026-09-11T12:40:00.000Z",
        executionId: "other",
        message: "unrelated",
      }),
    ];

    expect(
      logsForExecution(logs, "api.create_order", "2026-09-11T12:42:10.000Z", "01KABC").map(
        (item) => item.message,
      ),
    ).toEqual(["starting create_order", "order created"]);
  });
});
