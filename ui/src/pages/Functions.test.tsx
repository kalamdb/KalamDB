// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import Functions from "@/pages/Functions";
import type { ProcedureCatalogSnapshot } from "@/features/functions/types";

const mockCatalogQuery = vi.fn();
const mockRefetch = vi.fn();

vi.mock("@/store/apiSlice", () => ({
  useGetProcedureCatalogQuery: () => mockCatalogQuery(),
}));

const snapshot: ProcedureCatalogSnapshot = {
  procedures: [
    {
      id: "api.create_order",
      schema: "api",
      name: "create_order",
      signature: "customer_id UUID",
      parameters: [],
      returnType: { kind: "builtin", builtin: "UUID", sqlName: "UUID", notNull: false, nonempty: false },
      returnTypeName: "UUID",
      security: "INVOKER",
      owner: "root",
      comment: "Creates an order",
      implementation: "module",
      moduleId: "backend",
      revisionId: "backend:aaa111",
      grants: "dba",
      language: null,
      source: null,
    },
    {
      id: "chat.send_message",
      schema: "chat",
      name: "send_message",
      signature: "conversation_id UUID, content TEXT",
      parameters: [],
      returnType: { kind: "unknown", sqlName: "chat.message", notNull: false, nonempty: false },
      returnTypeName: "chat.message",
      security: "INVOKER",
      owner: "root",
      comment: "Handles sending messages in chat.",
      implementation: "module",
      moduleId: "backend",
      revisionId: "backend:84ac91abcdef",
      grants: "dba",
      language: null,
      source: null,
    },
    {
      id: "api.plus_one",
      schema: "api",
      name: "plus_one",
      signature: "x INT",
      parameters: [],
      returnType: { kind: "builtin", builtin: "INT", sqlName: "INT", notNull: false, nonempty: false },
      returnTypeName: "INT",
      security: "INVOKER",
      owner: "root",
      comment: null,
      implementation: "missing",
      moduleId: null,
      revisionId: null,
      grants: "dba",
      language: null,
      source: null,
    },
  ],
  types: [],
  typeFields: [],
  parameters: [],
  modules: [
    {
      module_id: "backend",
      runtime: "typescript",
      current_revision_id: "backend:84ac91abcdef",
      contract_hash: "hash",
      abi_version: 2,
    },
  ],
  revisions: [
    {
      module_id: "backend",
      revision_id: "backend:84ac91abcdef",
      artifact_id: "84ac91abcdef",
      artifact_bytes: 84000,
      contract_hash: "hash",
      created_at: "2024-10-24T12:41:00.000Z",
      is_current: true,
      exports: "chat.send_message",
    },
  ],
  logs: [
    {
      timestamp: "2026-09-11T11:00:00.000Z",
      node_id: "n1",
      execution_id: "01KERR",
      request_id: "01KERR",
      procedure_id: "chat.send_message",
      module_id: "backend",
      revision_id: "backend:84ac91abcdef",
      actor: "root",
      origin: "http",
      outcome: "error",
      channel: "invocation",
      level: "error",
      error_code: "RATE_LIMIT",
      message: "rate limited",
      duration_ms: 12,
    },
  ],
};

function renderList() {
  return render(
    <MemoryRouter initialEntries={["/functions"]}>
      <Routes>
        <Route path="/functions" element={<Functions />} />
        <Route path="/functions/:procedureId" element={<div>Function detail</div>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("Functions list", () => {
  beforeEach(() => {
    mockRefetch.mockReset();
    mockCatalogQuery.mockReturnValue({
      data: snapshot,
      isFetching: false,
      error: null,
      refetch: mockRefetch,
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("loads procedures from the catalog", () => {
    renderList();
    expect(screen.getByText("api.create_order")).toBeTruthy();
    expect(screen.getByText("chat.send_message")).toBeTruthy();
    expect(screen.getByText("3 functions")).toBeTruthy();
    expect(screen.getAllByText("Ready").length).toBeGreaterThan(0);
    expect(screen.getByText("Unimplemented")).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Function" })).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Status" })).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Current revision" })).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Last updated" })).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Calls 24h" })).toBeTruthy();
    expect(screen.getByText("backend:aaa111")).toBeTruthy();
  });

  it("filters procedures by search", () => {
    renderList();
    fireEvent.change(screen.getByPlaceholderText("Search functions..."), { target: { value: "chat" } });
    expect(screen.getByText("chat.send_message")).toBeTruthy();
    expect(screen.queryByText("api.create_order")).toBeNull();
  });

  it("opens the function detail page when a row is clicked", () => {
    renderList();
    fireEvent.click(screen.getByText("chat.send_message"));
    expect(screen.getByText("Function detail")).toBeTruthy();
  });
});
