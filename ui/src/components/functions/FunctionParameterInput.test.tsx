// @vitest-environment jsdom

import { useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FunctionParameterInput } from "./FunctionParameterInput";
import { resolveProcedureCatalog } from "@/features/functions/catalog";
import { defaultValueForType, type TestFieldError, type TestValue } from "@/features/functions/testValues";
import type { CatalogParameterRow, CatalogTypeFieldRow, CatalogTypeRow, ResolvedKalamType } from "@/features/functions/types";

function parameter(
  name: string,
  ordinal: number,
  typeName: string,
  extras: Partial<CatalogParameterRow> = {},
): CatalogParameterRow {
  return {
    parameterId: `p:${ordinal}`,
    routineId: "api.create_order",
    name,
    ordinal,
    typeId: extras.typeId ?? null,
    typeName,
    isArray: extras.isArray ?? false,
    notNull: extras.notNull ?? false,
    nonempty: extras.nonempty ?? false,
  };
}

const types: CatalogTypeRow[] = [
  {
    typeId: "api.address",
    namespaceId: "api",
    name: "address",
    kind: "composite",
    tableId: null,
    sourceTypeId: null,
    comment: null,
  },
  {
    typeId: "chat.message_status",
    namespaceId: "chat",
    name: "message_status",
    kind: "enum",
    tableId: null,
    sourceTypeId: null,
    comment: null,
  },
  {
    typeId: "api.order_item",
    namespaceId: "api",
    name: "order_item",
    kind: "composite",
    tableId: null,
    sourceTypeId: null,
    comment: null,
  },
];

const fields: CatalogTypeFieldRow[] = [
  { typeFieldId: "a:city", typeId: "api.address", name: "city", ordinal: 1, fieldTypeId: null, typeName: "TEXT", isArray: false, notNull: true, nonempty: false },
  { typeFieldId: "a:country", typeId: "api.address", name: "country", ordinal: 2, fieldTypeId: null, typeName: "TEXT", isArray: false, notNull: true, nonempty: false },
  { typeFieldId: "s:sent", typeId: "chat.message_status", name: "sent", ordinal: 1, fieldTypeId: null, typeName: "sent", isArray: false, notNull: true, nonempty: false },
  { typeFieldId: "s:delivered", typeId: "chat.message_status", name: "delivered", ordinal: 2, fieldTypeId: null, typeName: "delivered", isArray: false, notNull: true, nonempty: false },
  { typeFieldId: "i:sku", typeId: "api.order_item", name: "sku", ordinal: 1, fieldTypeId: null, typeName: "TEXT", isArray: false, notNull: true, nonempty: false },
  { typeFieldId: "i:qty", typeId: "api.order_item", name: "quantity", ordinal: 2, fieldTypeId: null, typeName: "INT", isArray: false, notNull: true, nonempty: false },
];

const [procedure] = resolveProcedureCatalog(
  [
    {
      procedureId: "api.create_order",
      schema: "api",
      name: "create_order",
      signature: "",
      returnType: "UUID",
      implementation: "module",
      moduleId: "backend",
      revisionId: "backend:abc",
      security: "INVOKER",
      owner: "root",
      grants: "dba",
      comment: null,
    },
  ],
  [
    parameter("customer_id", 1, "UUID", { notNull: true }),
    parameter("comment", 2, "TEXT"),
    parameter("status", 3, "chat.message_status", { typeId: "chat.message_status" }),
    parameter("items", 4, "api.order_item", { typeId: "api.order_item", isArray: true, notNull: true }),
    parameter("address", 5, "api.address", { typeId: "api.address", notNull: true }),
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
