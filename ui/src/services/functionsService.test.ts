import { describe, expect, it } from "vitest";
import { logsForExecution, selectTypeFieldsForCatalog } from "./functionsService";
import type { SystemProcedureLogRow, SystemTypeFieldRow } from "@/features/functions/types";

function log(overrides: Partial<SystemProcedureLogRow>): SystemProcedureLogRow {
  return {
    timestamp: "2026-09-11T12:42:10.102Z",
    node_id: "n1",
    execution_id: "01KABC",
    request_id: "01KABC",
    procedure_id: "api.create_order",
    module_id: "backend",
    revision_id: "backend:84ac91",
    actor: "root",
    origin: "http",
    outcome: "log",
    channel: "ctx.log",
    level: "info",
    error_code: null,
    message: "starting create_order",
    duration_ms: 0,
    ...overrides,
  };
}

describe("logsForExecution", () => {
  it("returns logs for a known execution id in chronological order", () => {
    const logs = [
      log({ timestamp: "2026-09-11T12:42:10.114Z", message: "order created", outcome: "ok", duration_ms: 12 }),
      log({ timestamp: "2026-09-11T12:42:10.102Z", message: "starting create_order" }),
      log({
        timestamp: "2026-09-11T12:40:00.000Z",
        execution_id: "other",
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

function typeField(overrides: Partial<SystemTypeFieldRow>): SystemTypeFieldRow {
  return {
    type_field_id: "tf",
    type_id: "shop.order",
    name: "id",
    ordinal: 0,
    field_type_id: null,
    type_name: "text",
    is_array: false,
    not_null: false,
    nonempty: false,
    data_type: null,
    ...overrides,
  };
}

describe("selectTypeFieldsForCatalog", () => {
  it("keeps fields for seed types and walks nested named field types in memory", () => {
    const address = typeField({
      type_field_id: "shop.address:city",
      type_id: "shop.address",
      name: "city",
      type_name: "text",
    });
    const customer = typeField({
      type_field_id: "shop.customer:address",
      type_id: "shop.customer",
      name: "address",
      field_type_id: "shop.address",
      type_name: "shop.address",
    });
    const unused = typeField({
      type_field_id: "shop.unused:id",
      type_id: "shop.unused",
      name: "id",
    });

    expect(selectTypeFieldsForCatalog([customer, address, unused], ["shop.customer"])).toEqual([
      customer,
      address,
    ]);
  });

  it("does not treat builtin SQL type names as catalog types to walk", () => {
    const request = typeField({
      type_field_id: "api.health_request:n",
      type_id: "api.health_request",
      name: "n",
      field_type_id: "int",
      type_name: "int",
    });

    expect(selectTypeFieldsForCatalog([request], ["api.health_request"])).toEqual([request]);
  });
});
