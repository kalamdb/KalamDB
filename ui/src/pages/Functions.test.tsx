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
    },
  ],
  types: [],
  typeFields: [],
  parameters: [],
  modules: [
    {
      moduleId: "backend",
      runtime: "typescript",
      currentRevisionId: "backend:84ac91abcdef",
      contractHash: "hash",
      abiVersion: 2,
    },
  ],
  revisions: [
    {
      moduleId: "backend",
      revisionId: "backend:84ac91abcdef",
      artifactId: "84ac91abcdef",
      artifactBytes: 84000,
      contractHash: "hash",
      createdAtMs: Date.parse("2024-10-24T12:41:00Z"),
      isCurrent: true,
      exports: ["chat.send_message"],
    },
  ],
  logs: [
    {
      timestamp: "2026-09-11T11:00:00.000Z",
      nodeId: "n1",
      executionId: "01KERR",
      requestId: "01KERR",
      procedureId: "chat.send_message",
      moduleId: "backend",
      revisionId: "backend:84ac91abcdef",
      actor: "root",
      origin: "http",
      outcome: "error",
      channel: "invocation",
      level: "error",
      errorCode: "RATE_LIMIT",
      message: "rate limited",
      durationMs: 12,
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
    expect(screen.getByText("2 functions")).toBeTruthy();
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
