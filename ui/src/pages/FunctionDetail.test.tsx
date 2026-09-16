// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import FunctionDetail from "@/pages/FunctionDetail";
import { resolveProcedureCatalog } from "@/features/functions/catalog";
import type {
  ProcedureCatalogSnapshot,
  SystemProcedureLogRow,
  SystemRoutineParameterRow,
  SystemTypeFieldRow,
  SystemTypeRow,
} from "@/features/functions/types";
import type { InvokeProcedureResult } from "@/services/functionsService";

const {
  mockCatalogQuery,
  mockLogsQuery,
  mockInstancesQuery,
  mockInvoke,
  mockRollback,
  mockRefetchCatalog,
  mockRefetchLogs,
  invokeState,
} = vi.hoisted(() => ({
  mockCatalogQuery: vi.fn(),
  mockLogsQuery: vi.fn(),
  mockInstancesQuery: vi.fn(),
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

vi.mock("@monaco-editor/react", () => ({
  default: ({ value, language }: { value?: string; language?: string }) => (
    <pre data-language={language}>{value}</pre>
  ),
}));

vi.mock("@/store/apiSlice", () => ({
  useGetProcedureCatalogQuery: () => mockCatalogQuery(),
  useGetProcedureLogsQuery: (...args: unknown[]) => mockLogsQuery(...args),
  useGetModuleInstancesQuery: (...args: unknown[]) => mockInstancesQuery(...args),
  useInvokeProcedureMutation: () => [mockInvoke, invokeState],
  useRollbackModuleRevisionMutation: () => [mockRollback, { isLoading: false, error: null }],
}));

function parameter(
  routineId: string,
  name: string,
  ordinal: number,
  typeName: string,
  extras: Partial<SystemRoutineParameterRow> = {},
): SystemRoutineParameterRow {
  return {
    parameter_id: `${routineId}:${ordinal}`,
    routine_id: routineId,
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

const addressType: SystemTypeRow = {
  type_id: "api.address",
  namespace_id: "api",
  name: "address",
  kind: "composite",
  table_id: null,
  source_type_id: null,
  comment: null,
};

const orderItemType: SystemTypeRow = {
  type_id: "api.order_item",
  namespace_id: "api",
  name: "order_item",
  kind: "composite",
  table_id: null,
  source_type_id: null,
  comment: null,
};

const statusEnum: SystemTypeRow = {
  type_id: "chat.message_status",
  namespace_id: "chat",
  name: "message_status",
  kind: "enum",
  table_id: null,
  source_type_id: null,
  comment: null,
};

const addressFields: SystemTypeFieldRow[] = [
  { type_field_id: "api.address:city", type_id: "api.address", name: "city", ordinal: 1, field_type_id: null, type_name: "TEXT", is_array: false, not_null: true, nonempty: false, data_type: null },
  { type_field_id: "api.address:country", type_id: "api.address", name: "country", ordinal: 2, field_type_id: null, type_name: "TEXT", is_array: false, not_null: true, nonempty: false, data_type: null },
];

const orderItemFields: SystemTypeFieldRow[] = [
  { type_field_id: "api.order_item:sku", type_id: "api.order_item", name: "sku", ordinal: 1, field_type_id: null, type_name: "TEXT", is_array: false, not_null: true, nonempty: false, data_type: null },
  { type_field_id: "api.order_item:quantity", type_id: "api.order_item", name: "quantity", ordinal: 2, field_type_id: null, type_name: "INT", is_array: false, not_null: true, nonempty: false, data_type: null },
];

const enumFields: SystemTypeFieldRow[] = [
  { type_field_id: "chat.message_status:sent", type_id: "chat.message_status", name: "sent", ordinal: 1, field_type_id: null, type_name: "sent", is_array: false, not_null: true, nonempty: false, data_type: null },
  { type_field_id: "chat.message_status:delivered", type_id: "chat.message_status", name: "delivered", ordinal: 2, field_type_id: null, type_name: "delivered", is_array: false, not_null: true, nonempty: false, data_type: null },
];

const procedures = resolveProcedureCatalog(
  [
    {
      procedure_id: "api.create_order",
      schema: "api",
      name: "create_order",
      signature: "customer_id UUID, items api.order_item[]",
      return_type: "UUID",
      implementation: "module",
      module_id: "backend",
      revision_id: "backend:84ac91abcdef",
      security: "INVOKER",
      owner: "root",
      grants: "dba",
      comment: "Creates an order",
      language: null,
      source: null,
    },
    {
      procedure_id: "ns.book_tickets",
      schema: "ns",
      name: "book_tickets",
      signature: "venue_id TEXT NOT NULL",
      return_type: "TEXT",
      implementation: "inline",
      module_id: null,
      revision_id: "inline:deadbeef",
      security: "INVOKER",
      owner: "root",
      grants: "dba",
      comment: "Concert booking",
      language: "JAVASCRIPT",
      source: "var venueId = input.venue_id;\nreturn venueId;",
    },
  ],
  [
    parameter("api.create_order", "customer_id", 1, "UUID", { not_null: true }),
    parameter("api.create_order", "quantity", 2, "INT", { not_null: true }),
    parameter("api.create_order", "comment", 3, "TEXT"),
    parameter("api.create_order", "priority", 4, "BOOLEAN", { not_null: true }),
    parameter("api.create_order", "status", 5, "chat.message_status", { type_id: "chat.message_status" }),
    parameter("api.create_order", "items", 6, "api.order_item", { type_id: "api.order_item", is_array: true, not_null: true }),
    parameter("api.create_order", "address", 7, "api.address", { type_id: "api.address", not_null: true }),
  ],
  [addressType, orderItemType, statusEnum],
  [...addressFields, ...orderItemFields, ...enumFields],
);

const logs: SystemProcedureLogRow[] = [
  {
    timestamp: "2026-09-11T12:42:10.114Z",
    node_id: "n1",
    execution_id: "01KTEST",
    request_id: "01KTEST",
    procedure_id: "api.create_order",
    module_id: "backend",
    revision_id: "backend:84ac91abcdef",
    actor: "Alex",
    origin: "http",
    outcome: "ok",
    channel: "invocation",
    level: "info",
    error_code: null,
    message: "order created",
    duration_ms: 12,
  },
  {
    timestamp: "2026-09-11T12:38:11.000Z",
    node_id: "n1",
    execution_id: "01KWARN",
    request_id: "01KWARN",
    procedure_id: "api.create_order",
    module_id: "backend",
    revision_id: "backend:84ac91abcdef",
    actor: "Alex",
    origin: "sql",
    outcome: "log",
    channel: "ctx.log",
    level: "warn",
    error_code: null,
    message: "Rate limit approaching",
    duration_ms: 0,
  },
];

const snapshot: ProcedureCatalogSnapshot = {
  procedures,
  types: [addressType, orderItemType, statusEnum],
  typeFields: [...addressFields, ...orderItemFields, ...enumFields],
  parameters: [],
  modules: [
    {
      module_id: "backend",
      runtime: "typescript",
      current_revision_id: "backend:84ac91abcdef",
      contract_hash: "contract-hash",
      abi_version: 2,
    },
  ],
  revisions: [
    {
      module_id: "backend",
      revision_id: "backend:84ac91abcdef",
      artifact_id: "84ac91abcdef",
      artifact_bytes: 86016,
      contract_hash: "contract-hash",
      created_at: "2024-10-24T12:41:00.000Z",
      is_current: true,
      exports: "api.create_order",
    },
    {
      module_id: "backend",
      revision_id: "backend:719acprevious",
      artifact_id: "719acprevious",
      artifact_bytes: 82944,
      contract_hash: "older-hash",
      created_at: "2024-10-23T09:17:00.000Z",
      is_current: false,
      exports: "api.create_order",
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
    mockInstancesQuery.mockReturnValue({
      data: [
        {
          instance_id: 1,
          worker: 0,
          module_id: "backend",
          revision_id: "backend:84ac91abcdef",
          state: "idle",
          reserved_bytes: 64 * 1024 * 1024,
          used_heap_bytes: 2048,
          peak_heap_bytes: 4096,
          invocations: 3,
        },
      ],
      isFetching: false,
      error: null,
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
    expect(screen.getByRole("navigation", { name: "breadcrumb" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Functions" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Overview" }).getAttribute("data-state")).toBe("active");
    expect(screen.getByRole("tab", { name: "Logs" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Revisions" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Test" })).toBeTruthy();
    expect(screen.getAllByText("Creates an order").length).toBeGreaterThan(0);
    expect(screen.getByText("Function details")).toBeTruthy();
    expect(screen.getByText("Project module")).toBeTruthy();
    expect(screen.getByText("SECURITY INVOKER")).toBeTruthy();
    expect(screen.getByText(/backend:84ac91/)).toBeTruthy();
    expect(screen.getByText("2024-10-24T12:41:00Z")).toBeTruthy();
    expect(screen.getByText("TypeScript / V8")).toBeTruthy();
    expect(screen.getByText("Used heap")).toBeTruthy();
    expect(screen.getByText("Peak heap")).toBeTruthy();
    expect(screen.getByText("2.0 KB")).toBeTruthy();
    expect(screen.getByText("4.0 KB")).toBeTruthy();
    expect(screen.getByText("64 MB")).toBeTruthy();
    expect(screen.queryByTestId("function-inline-source")).toBeNull();
    expect(screen.getAllByText("Alex").length).toBeGreaterThan(0);
    expect(screen.getByText("HTTP")).toBeTruthy();
    expect(screen.getByText("SQL")).toBeTruthy();
    expect(screen.getByText("Rate limit approaching")).toBeTruthy();
    expect(mockLogsQuery).toHaveBeenCalledWith(
      expect.objectContaining({ procedureId: "api.create_order", limit: 10 }),
    );
  });

  it("shows inline source and isolate memory for an inline procedure", () => {
    mockInstancesQuery.mockReturnValue({
      data: [
        {
          instance_id: 7,
          worker: 0,
          module_id: "inline",
          revision_id: "inline:deadbeef",
          state: "idle",
          reserved_bytes: 32 * 1024 * 1024,
          used_heap_bytes: 1024,
          peak_heap_bytes: 1536,
          invocations: 1,
        },
      ],
      isFetching: false,
      error: null,
    });

    renderDetail("/functions/ns.book_tickets");
    expect(screen.getByText("Inline script")).toBeTruthy();
    expect(screen.getByText("JAVASCRIPT")).toBeTruthy();
    expect(screen.getByTestId("function-inline-source")).toBeTruthy();
    expect(screen.getByTestId("function-inline-source-editor")).toBeTruthy();
    expect(screen.getByText(/var venueId = input.venue_id/)).toBeTruthy();
    expect(screen.getByTestId("function-inline-source-editor").querySelector("[data-language='typescript']")).toBeTruthy();
    expect(screen.getByTestId("function-runtime-memory")).toBeTruthy();
    expect(screen.getByText("Used heap")).toBeTruthy();
    expect(screen.getByText("1.0 KB")).toBeTruthy();
    expect(screen.getByText(/inline:deadbeef/)).toBeTruthy();
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
    expect(screen.getAllByText("Alex").length).toBeGreaterThan(0);
    expect(screen.getByText("HTTP")).toBeTruthy();
    expect(screen.getByText("SQL")).toBeTruthy();
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
    expect(screen.getByRole("combobox", { name: /status/i })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "priority" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add item" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Run test" }));
    expect(screen.getByText("customer_id is required")).toBeTruthy();
    expect(mockInvoke).not.toHaveBeenCalled();

    fireEvent.change(screen.getByPlaceholderText("00000000-0000-0000-0000-000000000000"), {
      target: { value: "2c1c0a1a-1111-4111-8111-aaaaaaaaaaaa" },
    });
    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "2" } });
    fireEvent.change(screen.getByRole("textbox", { name: /city/i }), {
      target: { value: "Austin" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: /country/i }), {
      target: { value: "US" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Run test" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
    expect(await screen.findByText(/ord_1/)).toBeTruthy();
    expect(screen.getByText("order created")).toBeTruthy();
  });
});
