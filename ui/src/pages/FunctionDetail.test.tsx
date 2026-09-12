// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import FunctionDetail from "@/pages/FunctionDetail";
import { resolveProcedureCatalog } from "@/features/functions/catalog";
import type {
  CatalogParameterRow,
  CatalogTypeFieldRow,
  CatalogTypeRow,
  ProcedureCatalogSnapshot,
  ProcedureLogRecord,
} from "@/features/functions/types";
import type { InvokeProcedureResult } from "@/services/functionsService";

const {
  mockCatalogQuery,
  mockLogsQuery,
  mockInvoke,
  mockRollback,
  mockRefetchCatalog,
  mockRefetchLogs,
  invokeState,
} = vi.hoisted(() => ({
  mockCatalogQuery: vi.fn(),
  mockLogsQuery: vi.fn(),
  mockInvoke: vi.fn(),
  mockRollback: vi.fn(),
  mockRefetchCatalog: vi.fn(),
  mockRefetchLogs: vi.fn(),
  invokeState: {
    data: null as InvokeProcedureResult | null,
    error: null as { status: string; error: string } | null,
    isLoading: false,
  },
}));

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: { id: "root", username: "Alex", role: "system" } }),
}));

vi.mock("@kalamdb/client", () => ({
  KalamCellValue: class KalamCellValue {
    constructor(private value: unknown) {}
    toJson() {
      return this.value;
    }
  },
}));

vi.mock("@/store/apiSlice", () => ({
  useGetProcedureCatalogQuery: () => mockCatalogQuery(),
  useGetProcedureLogsQuery: (...args: unknown[]) => mockLogsQuery(...args),
  useInvokeProcedureMutation: () => [mockInvoke, invokeState],
  useRollbackModuleRevisionMutation: () => [mockRollback, { isLoading: false, error: null }],
}));

function parameter(
  routineId: string,
  name: string,
  ordinal: number,
  typeName: string,
  extras: Partial<CatalogParameterRow> = {},
): CatalogParameterRow {
  return {
    parameterId: `${routineId}:${ordinal}`,
    routineId,
    name,
    ordinal,
    typeId: extras.typeId ?? null,
    typeName,
    isArray: extras.isArray ?? false,
    notNull: extras.notNull ?? false,
    nonempty: extras.nonempty ?? false,
  };
}

const addressType: CatalogTypeRow = {
  typeId: "api.address",
  namespaceId: "api",
  name: "address",
  kind: "composite",
  tableId: null,
  sourceTypeId: null,
  comment: null,
};

const orderItemType: CatalogTypeRow = {
  typeId: "api.order_item",
  namespaceId: "api",
  name: "order_item",
  kind: "composite",
  tableId: null,
  sourceTypeId: null,
  comment: null,
};

const statusEnum: CatalogTypeRow = {
  typeId: "chat.message_status",
  namespaceId: "chat",
  name: "message_status",
  kind: "enum",
  tableId: null,
  sourceTypeId: null,
  comment: null,
};

const addressFields: CatalogTypeFieldRow[] = [
  { typeFieldId: "api.address:city", typeId: "api.address", name: "city", ordinal: 1, fieldTypeId: null, typeName: "TEXT", isArray: false, notNull: true, nonempty: false },
  { typeFieldId: "api.address:country", typeId: "api.address", name: "country", ordinal: 2, fieldTypeId: null, typeName: "TEXT", isArray: false, notNull: true, nonempty: false },
];

const orderItemFields: CatalogTypeFieldRow[] = [
  { typeFieldId: "api.order_item:sku", typeId: "api.order_item", name: "sku", ordinal: 1, fieldTypeId: null, typeName: "TEXT", isArray: false, notNull: true, nonempty: false },
  { typeFieldId: "api.order_item:quantity", typeId: "api.order_item", name: "quantity", ordinal: 2, fieldTypeId: null, typeName: "INT", isArray: false, notNull: true, nonempty: false },
];

const enumFields: CatalogTypeFieldRow[] = [
  { typeFieldId: "chat.message_status:sent", typeId: "chat.message_status", name: "sent", ordinal: 1, fieldTypeId: null, typeName: "sent", isArray: false, notNull: true, nonempty: false },
  { typeFieldId: "chat.message_status:delivered", typeId: "chat.message_status", name: "delivered", ordinal: 2, fieldTypeId: null, typeName: "delivered", isArray: false, notNull: true, nonempty: false },
];

const procedures = resolveProcedureCatalog(
  [
    {
      procedureId: "api.create_order",
      schema: "api",
      name: "create_order",
      signature: "customer_id UUID, items api.order_item[]",
      returnType: "UUID",
      implementation: "module",
      moduleId: "backend",
      revisionId: "backend:84ac91abcdef",
      security: "INVOKER",
      owner: "root",
      grants: "dba",
      comment: "Creates an order",
    },
  ],
  [
    parameter("api.create_order", "customer_id", 1, "UUID", { notNull: true }),
    parameter("api.create_order", "quantity", 2, "INT", { notNull: true }),
    parameter("api.create_order", "comment", 3, "TEXT"),
    parameter("api.create_order", "priority", 4, "BOOLEAN", { notNull: true }),
    parameter("api.create_order", "status", 5, "chat.message_status", { typeId: "chat.message_status" }),
    parameter("api.create_order", "items", 6, "api.order_item", { typeId: "api.order_item", isArray: true, notNull: true }),
    parameter("api.create_order", "address", 7, "api.address", { typeId: "api.address", notNull: true }),
  ],
  [addressType, orderItemType, statusEnum],
  [...addressFields, ...orderItemFields, ...enumFields],
);

const logs: ProcedureLogRecord[] = [
  {
    timestamp: "2026-09-11T12:42:10.114Z",
    nodeId: "n1",
    executionId: "01KTEST",
    requestId: "01KTEST",
    procedureId: "api.create_order",
    moduleId: "backend",
    revisionId: "backend:84ac91abcdef",
    actor: "Alex",
    origin: "http",
    outcome: "ok",
    channel: "invocation",
    level: "info",
    errorCode: null,
    message: "order created",
    durationMs: 12,
  },
  {
    timestamp: "2026-09-11T12:38:11.000Z",
    nodeId: "n1",
    executionId: "01KWARN",
    requestId: "01KWARN",
    procedureId: "api.create_order",
    moduleId: "backend",
    revisionId: "backend:84ac91abcdef",
    actor: "Alex",
    origin: "sql",
    outcome: "log",
    channel: "ctx.log",
    level: "warn",
    errorCode: null,
    message: "Rate limit approaching",
    durationMs: 0,
  },
];

const snapshot: ProcedureCatalogSnapshot = {
  procedures,
  types: [addressType, orderItemType, statusEnum],
  typeFields: [...addressFields, ...orderItemFields, ...enumFields],
  parameters: [],
  modules: [
    {
      moduleId: "backend",
      runtime: "typescript",
      currentRevisionId: "backend:84ac91abcdef",
      contractHash: "contract-hash",
      abiVersion: 2,
    },
  ],
  revisions: [
    {
      moduleId: "backend",
      revisionId: "backend:84ac91abcdef",
      artifactId: "84ac91abcdef",
      artifactBytes: 86016,
      contractHash: "contract-hash",
      createdAtMs: Date.parse("2024-10-24T12:41:00Z"),
      isCurrent: true,
      exports: ["api.create_order"],
    },
    {
      moduleId: "backend",
      revisionId: "backend:719acprevious",
      artifactId: "719acprevious",
      artifactBytes: 82944,
      contractHash: "older-hash",
      createdAtMs: Date.parse("2024-10-23T09:17:00Z"),
      isCurrent: false,
      exports: ["api.create_order"],
    },
  ],
  logs,
};

const successResult: InvokeProcedureResult = {
  ok: true,
  statusCode: 200,
  durationMs: 18,
  body: { order_id: "ord_1", status: "created" },
  errorCode: null,
  errorMessage: null,
  startedAt: "2026-09-11T12:42:10.000Z",
};

function renderDetail(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/functions/:procedureId" element={<FunctionDetail />} />
        <Route path="/functions/:procedureId/:tab" element={<FunctionDetail />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("Function detail", () => {
  beforeEach(() => {
    vi.stubGlobal(
      "ResizeObserver",
      class ResizeObserver {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
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
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    });

    invokeState.data = null;
    invokeState.error = null;
    invokeState.isLoading = false;
    mockRefetchCatalog.mockReset();
    mockRefetchLogs.mockReset();
    mockInvoke.mockReset();
    mockRollback.mockReset();
    mockCatalogQuery.mockReturnValue({
      data: snapshot,
      isFetching: false,
      error: null,
      refetch: mockRefetchCatalog,
    });
    mockLogsQuery.mockReturnValue({
      data: logs,
      isFetching: false,
      error: null,
      refetch: mockRefetchLogs,
    });
    mockInvoke.mockImplementation(() => {
      invokeState.data = successResult;
      return { unwrap: vi.fn().mockResolvedValue(successResult) };
    });
    mockRollback.mockReturnValue({
      unwrap: vi.fn().mockResolvedValue({ status: "ok" }),
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("renders overview metadata and the current revision", () => {
    renderDetail("/functions/api.create_order");
    expect(screen.getByRole("heading", { name: "api.create_order" })).toBeTruthy();
    expect(screen.getAllByText("Creates an order").length).toBeGreaterThan(0);
    expect(screen.getByText("Function details")).toBeTruthy();
    expect(screen.getByText(/backend:84ac91/)).toBeTruthy();
    expect(screen.getByText("TypeScript / V8")).toBeTruthy();
    expect(screen.getByText("Rate limit approaching")).toBeTruthy();
    expect(mockLogsQuery).toHaveBeenCalledWith(
      expect.objectContaining({ procedureId: "api.create_order", limit: 10 }),
    );
  });

  it("requests logs for the current procedure and renders levels", () => {
    renderDetail("/functions/api.create_order/logs");
    expect(mockLogsQuery).toHaveBeenCalled();
    const logsCall = mockLogsQuery.mock.calls
      .map((call) => call[0] as { procedureId?: string; search?: string } | undefined)
      .find((arg) => arg && "search" in arg);
    expect(logsCall?.procedureId).toBe("api.create_order");
    expect(screen.getByText("WARN")).toBeTruthy();
    expect(screen.getByText("Rate limit approaching")).toBeTruthy();
  });

  it("shows an empty logs state", () => {
    mockLogsQuery.mockReturnValue({
      data: [],
      isFetching: false,
      error: null,
      refetch: mockRefetchLogs,
    });
    renderDetail("/functions/api.create_order/logs");
    expect(screen.getByText("No logs for this procedure.")).toBeTruthy();
  });

  it("loads revisions, identifies the current revision, and confirms rollback", async () => {
    renderDetail("/functions/api.create_order/revisions");
    expect(screen.getByText("Current")).toBeTruthy();
    fireEvent.click(screen.getByText(/backend:719ac/));
    expect(await screen.findByText("Rollback to this revision")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Rollback to this revision" }));
    expect(await screen.findByText("Roll back to this revision?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Rollback" }));
    await waitFor(() => {
      expect(mockRollback).toHaveBeenCalledWith({
        moduleId: "backend",
        revisionId: "backend:719acprevious",
      });
    });
  });

  it("shows the backend error when rollback fails", async () => {
    mockRollback.mockReturnValue({
      unwrap: vi.fn().mockRejectedValue({ status: "CUSTOM_ERROR", error: "revision is stale" }),
    });
    renderDetail("/functions/api.create_order/revisions");
    fireEvent.click(screen.getByText(/backend:719ac/));
    fireEvent.click(await screen.findByRole("button", { name: "Rollback to this revision" }));
    fireEvent.click(await screen.findByRole("button", { name: "Rollback" }));
    expect(await screen.findByText("revision is stale")).toBeTruthy();
  });

  it("validates required test fields, executes the procedure, and renders response logs", async () => {
    renderDetail("/functions/api.create_order/test");

    expect(screen.getByText("customer_id")).toBeTruthy();
    expect(screen.getByText("comment")).toBeTruthy();
    expect(screen.getByRole("switch", { name: "priority" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add item" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Run test" }));
    expect(screen.getByText("customer_id is required")).toBeTruthy();
    expect(mockInvoke).not.toHaveBeenCalled();

    fireEvent.change(screen.getByPlaceholderText("00000000-0000-0000-0000-000000000000"), {
      target: { value: "2c1c0a1a-1111-4111-8111-aaaaaaaaaaaa" },
    });
    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "2" } });
    fireEvent.change(screen.getByText("city").parentElement!.querySelector("input")!, {
      target: { value: "Austin" },
    });
    fireEvent.change(screen.getByText("country").parentElement!.querySelector("input")!, {
      target: { value: "US" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Run test" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
    expect(await screen.findByText(/ord_1/)).toBeTruthy();
    expect(screen.getByText("order created")).toBeTruthy();
  });
});
