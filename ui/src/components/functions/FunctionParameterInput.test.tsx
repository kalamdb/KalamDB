// @vitest-environment jsdom

import { useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FunctionParameterInput } from "./FunctionParameterInput";
import { resolveProcedureCatalog } from "@/features/functions/catalog";
import { defaultValueForType, type TestFieldError, type TestValue } from "@/features/functions/testValues";
import type { ResolvedKalamType, SystemRoutineParameterRow, SystemTypeFieldRow, SystemTypeRow } from "@/features/functions/types";

function parameter(
  name: string,
  ordinal: number,
  typeName: string,
  extras: Partial<SystemRoutineParameterRow> = {},
): SystemRoutineParameterRow {
  return {
    parameter_id: `p:${ordinal}`,
    routine_id: "api.create_order",
    name,
    ordinal,
    type_id: null,
    type_name: typeName,
    is_array: false,
    not_null: false,
    nonempty: false,
    data_type: null,
    ...extras,
  };
}

const types: SystemTypeRow[] = [
  {
    type_id: "api.address",
    namespace_id: "api",
    name: "address",
    kind: "composite",
    table_id: null,
    source_type_id: null,
    comment: null,
  },
  {
    type_id: "chat.message_status",
    namespace_id: "chat",
    name: "message_status",
    kind: "enum",
    table_id: null,
    source_type_id: null,
    comment: null,
  },
  {
    type_id: "api.order_item",
    namespace_id: "api",
    name: "order_item",
    kind: "composite",
    table_id: null,
    source_type_id: null,
    comment: null,
  },
];

const fields: SystemTypeFieldRow[] = [
  { type_field_id: "a:city", type_id: "api.address", name: "city", ordinal: 1, field_type_id: null, type_name: "TEXT", is_array: false, not_null: true, nonempty: false, data_type: null },
  { type_field_id: "a:country", type_id: "api.address", name: "country", ordinal: 2, field_type_id: null, type_name: "TEXT", is_array: false, not_null: true, nonempty: false, data_type: null },
  { type_field_id: "s:sent", type_id: "chat.message_status", name: "sent", ordinal: 1, field_type_id: null, type_name: "sent", is_array: false, not_null: true, nonempty: false, data_type: null },
  { type_field_id: "s:delivered", type_id: "chat.message_status", name: "delivered", ordinal: 2, field_type_id: null, type_name: "delivered", is_array: false, not_null: true, nonempty: false, data_type: null },
  { type_field_id: "i:sku", type_id: "api.order_item", name: "sku", ordinal: 1, field_type_id: null, type_name: "TEXT", is_array: false, not_null: true, nonempty: false, data_type: null },
  { type_field_id: "i:qty", type_id: "api.order_item", name: "quantity", ordinal: 2, field_type_id: null, type_name: "INT", is_array: false, not_null: true, nonempty: false, data_type: null },
];

const [procedure] = resolveProcedureCatalog(
  [
    {
      procedure_id: "api.create_order",
      schema: "api",
      name: "create_order",
      signature: "",
      return_type: "UUID",
      implementation: "module",
      module_id: "backend",
      revision_id: "backend:abc",
      security: "INVOKER",
      owner: "root",
      grants: "dba",
      comment: null,
      language: null,
      source: null,
    },
  ],
  [
    parameter("customer_id", 1, "UUID", { not_null: true }),
    parameter("comment", 2, "TEXT"),
    parameter("status", 3, "chat.message_status", { type_id: "chat.message_status" }),
    parameter("items", 4, "api.order_item", { type_id: "api.order_item", is_array: true, not_null: true }),
    parameter("address", 5, "api.address", { type_id: "api.address", not_null: true }),
  ],
  types,
  fields,
);

function Harness({
  name,
  type,
  initial,
}: {
  name: string;
  type: ResolvedKalamType;
  initial?: TestValue;
}) {
  const [value, setValue] = useState<TestValue>(initial ?? defaultValueForType(type));
  const errors: TestFieldError[] = [];
  return (
    <div>
      <FunctionParameterInput
        name={name}
        path={name}
        type={type}
        value={value}
        errors={errors}
        onChange={(_path, next) => setValue(next)}
      />
      <pre data-testid="value">{JSON.stringify(value)}</pre>
    </div>
  );
}

describe("FunctionParameterInput", () => {
  beforeEach(() => {
    vi.stubGlobal("PointerEvent", MouseEvent);
    Object.defineProperty(window.HTMLElement.prototype, "scrollIntoView", {
      value: vi.fn(),
      configurable: true,
    });
    Object.defineProperty(window.HTMLElement.prototype, "hasPointerCapture", {
      value: vi.fn(() => false),
      configurable: true,
    });
    Object.defineProperty(window.HTMLElement.prototype, "releasePointerCapture", {
      value: vi.fn(),
      configurable: true,
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("renders a UUID scalar control", () => {
    const type = procedure.parameters[0]!.type;
    render(<Harness name="customer_id" type={type} />);
    expect(screen.getByPlaceholderText("00000000-0000-0000-0000-000000000000")).toBeTruthy();
  });

  it("allows a nullable text parameter to stay empty", () => {
    const type = procedure.parameters[1]!.type;
    render(<Harness name="comment" type={type} />);
    expect(JSON.parse(screen.getByTestId("value").textContent ?? "")).toBeNull();
  });

  it("renders enum allowed values", () => {
    const type = procedure.parameters[2]!.type;
    render(<Harness name="status" type={type} />);
    fireEvent.click(screen.getByRole("combobox"));
    expect(screen.getByRole("option", { name: "NULL" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "sent" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "delivered" })).toBeTruthy();
  });

  it("supports adding and removing array items", () => {
    const type = procedure.parameters[3]!.type;
    render(<Harness name="items" type={type} />);
    fireEvent.click(screen.getByRole("button", { name: "Add item" }));
    expect(screen.getByText("sku")).toBeTruthy();
    expect(screen.getAllByText("sku")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Remove items item 1" }));
    expect(screen.queryByText("sku")).toBeNull();
  });

  it("renders nested composite fields", () => {
    const type = procedure.parameters[4]!.type;
    render(<Harness name="address" type={type} />);
    expect(screen.getByText("city")).toBeTruthy();
    expect(screen.getByText("country")).toBeTruthy();
  });
});
