import { describe, expect, it } from "vitest";
import { parseCatalogTypeKind, resolveProcedureCatalog } from "./catalog";

import { formatCallSnippet, formatCompositeOutline, formatSignatureDetail } from "./format";
import { deriveProcedureStatus, currentRevisionForProcedure, toProcedureListItem } from "./status";
import {
  completionsForCallContext,
  filterProceduresByPartial,
  hoverForIdentifier,
  parseCallCompletionContext,
  procedureCompletionEntries,
  signatureHelpForCall,
  signatureHelpLabel,
  valueSuggestionsForType,
} from "./sqlCompletions";
import {
  buildInvocationBody,
  defaultTestValues,
  setPathValue,
  validateTestValues,
} from "./testValues";
import type {
  SystemProcedureLogRow,
  SystemProcedureRow,
  SystemRoutineParameterRow,
  SystemTypeFieldRow,
  SystemTypeRow,
} from "./types";

function procedure(overrides: Partial<SystemProcedureRow> = {}): SystemProcedureRow {
  return {
    procedure_id: "chat.send_message",
    schema: "chat",
    name: "send_message",
    signature: "conversation_id UUID, content TEXT, notify BOOLEAN",
    return_type: "chat.message",
    implementation: "module",
    module_id: "backend",
    revision_id: "backend:84ac91abcdef",
    security: "INVOKER",
    owner: "root",
    grants: "dba",
    comment: "Handles sending messages in chat.",
    language: null,
    source: null,
    ...overrides,
  };
}

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

function catalogType(
  typeId: string,
  namespaceId: string,
  name: string,
  kind: string,
  extras: Partial<SystemTypeRow> = {},
): SystemTypeRow {
  return {
    type_id: typeId,
    namespace_id: namespaceId,
    name,
    kind,
    table_id: null,
    source_type_id: null,
    comment: null,
    ...extras,
  };
}

function catalogField(
  typeId: string,
  name: string,
  ordinal: number,
  typeName: string,
  extras: Partial<SystemTypeFieldRow> = {},
): SystemTypeFieldRow {
  return {
    type_field_id: `${typeId}:${name}`,
    type_id: typeId,
    name,
    ordinal,
    field_type_id: null,
    type_name: typeName,
    is_array: false,
    not_null: extras.not_null ?? true,
    nonempty: false,
    data_type: null,
    ...extras,
  };
}

const messageType = catalogType("chat.message", "chat", "message", "composite", {
  table_id: "chat.message",
});
const addressType = catalogType("api.address", "api", "address", "composite");
const orderItemType = catalogType("api.order_item", "api", "order_item", "composite");
const statusEnum = catalogType("chat.message_status", "chat", "message_status", "enum");

const messageFields: SystemTypeFieldRow[] = [
  catalogField("chat.message", "id", 1, "UUID"),
  catalogField("chat.message", "content", 2, "TEXT"),
];
const addressFields: SystemTypeFieldRow[] = [
  catalogField("api.address", "city", 1, "TEXT"),
  catalogField("api.address", "country", 2, "TEXT"),
];
const orderItemFields: SystemTypeFieldRow[] = [
  catalogField("api.order_item", "sku", 1, "TEXT"),
  catalogField("api.order_item", "quantity", 2, "INT"),
];
const enumFields: SystemTypeFieldRow[] = [
  catalogField("chat.message_status", "sent", 1, "sent"),
  catalogField("chat.message_status", "delivered", 2, "delivered"),
  catalogField("chat.message_status", "read", 3, "read"),
];

describe("catalog type kind parsing", () => {
  it("accepts snake_case, PascalCase, and ENUM labels", () => {
    expect(parseCatalogTypeKind("enum")).toBe("enum");
    expect(parseCatalogTypeKind("Enum")).toBe("enum");
    expect(parseCatalogTypeKind("ENUM")).toBe("enum");
    expect(parseCatalogTypeKind("ImplicitTableRow")).toBe("implicit_table_row");
    expect(parseCatalogTypeKind("row_alias")).toBe("row_alias");
  });

  it("unwraps cell values and quoted tokens without throwing", () => {
    expect(parseCatalogTypeKind({ toJson: () => "enum" })).toBe("enum");
    expect(parseCatalogTypeKind({ Utf8: "composite" })).toBe("composite");
    expect(parseCatalogTypeKind('"enum"')).toBe("enum");
    expect(parseCatalogTypeKind(undefined)).toBeNull();
    expect(parseCatalogTypeKind(null)).toBeNull();
  });
});

describe("procedure catalog resolver", () => {
  it("resolves scalar parameters, return types, and comments from catalog rows", () => {
    const [resolved] = resolveProcedureCatalog(
      [procedure()],
      [
        parameter("chat.send_message", "conversation_id", 1, "UUID", { not_null: true }),
        parameter("chat.send_message", "content", 2, "TEXT", { not_null: true }),
        parameter("chat.send_message", "notify", 3, "BOOLEAN"),
      ],
      [messageType],
      messageFields,
    );

    expect(resolved.id).toBe("chat.send_message");
    expect(resolved.parameters.map((item) => item.name)).toEqual([
      "conversation_id",
      "content",
      "notify",
    ]);
    expect(resolved.parameters[0]?.type).toMatchObject({
      kind: "builtin",
      sqlName: "UUID",
      notNull: true,
    });
    expect(resolved.parameters[2]?.type).toMatchObject({
      kind: "builtin",
      sqlName: "BOOLEAN",
      notNull: false,
    });
    expect(resolved.returnType).toMatchObject({
      kind: "composite",
      typeId: "chat.message",
    });
    expect(resolved.comment).toBe("Handles sending messages in chat.");
    expect(formatSignatureDetail(resolved)).toBe(
      "(conversation_id UUID NOT NULL, content TEXT NOT NULL, notify BOOLEAN) → chat.message",
    );
  });

  it("resolves enum, array, and nested composite parameter types", () => {
    const [createOrder] = resolveProcedureCatalog(
      [
        procedure({
          procedure_id: "api.create_order",
          schema: "api",
          name: "create_order",
          signature: "customer_id UUID, items api.order_item[]",
          return_type: "UUID",
          comment: null,
        }),
      ],
      [
        parameter("api.create_order", "customer_id", 1, "UUID", { not_null: true }),
        parameter("api.create_order", "items", 2, "api.order_item", {
          type_id: "api.order_item",
          is_array: true,
          not_null: true,
        }),
        parameter("api.create_order", "note", 3, "TEXT"),
      ],
      [orderItemType],
      orderItemFields,
    );

    expect(createOrder.parameters[1]?.type.kind).toBe("array");
    if (createOrder.parameters[1]?.type.kind === "array") {
      expect(createOrder.parameters[1].type.element).toMatchObject({
        kind: "composite",
        typeId: "api.order_item",
      });
    }

    const [createCustomer] = resolveProcedureCatalog(
      [
        procedure({
          procedure_id: "api.create_customer",
          schema: "api",
          name: "create_customer",
          return_type: "VOID",
        }),
      ],
      [
        parameter("api.create_customer", "name", 1, "TEXT", { not_null: true }),
        parameter("api.create_customer", "address", 2, "api.address", {
          type_id: "api.address",
          not_null: true,
        }),
        parameter("api.create_customer", "status", 3, "chat.message_status", {
          type_id: "chat.message_status",
        }),
      ],
      [addressType, statusEnum],
      [...addressFields, ...enumFields],
    );

    expect(createCustomer.parameters[1]?.type.kind).toBe("composite");
    expect(formatCompositeOutline(createCustomer.parameters[1]!.type)).toEqual([
      "api.address",
      "├── city TEXT NOT NULL",
      "└── country TEXT NOT NULL",
    ]);
    expect(createCustomer.parameters[2]?.type).toMatchObject({
      kind: "enum",
      values: ["sent", "delivered", "read"],
    });

    const pascalEnum = catalogType("chat.message_status", "chat", "message_status", "Enum");
    const [pascalResolved] = resolveProcedureCatalog(
      [
        procedure({
          procedure_id: "chat.set_status",
          schema: "chat",
          name: "set_status",
          signature: "status chat.message_status",
          return_type: "VOID",
        }),
      ],
      [parameter("chat.set_status", "status", 1, "message_status")],
      [pascalEnum],
      enumFields,
    );
    expect(pascalResolved.parameters[0]?.type).toMatchObject({
      kind: "enum",
      values: ["sent", "delivered", "read"],
    });

    const wrappedKind = catalogType("ns.ticket_status", "ns", "ticket_status", {
      toJson: () => "enum",
    } as unknown as string);
    const [wrappedResolved] = resolveProcedureCatalog(
      [
        procedure({
          procedure_id: "ns.book_tickets",
          schema: "ns",
          name: "book_tickets",
          signature: "status ns.ticket_status",
          return_type: "VOID",
        }),
      ],
      [
        parameter("ns.book_tickets", "status", 1, "ns.ticket_status", {
          type_id: { toJson: () => "ns.ticket_status" } as unknown as string,
          not_null: true,
        }),
      ],
      [wrappedKind],
      [
        catalogField("ns.ticket_status", "hold", 1, "hold"),
        catalogField("ns.ticket_status", "confirmed", 2, "confirmed"),
        catalogField("ns.ticket_status", "cancelled", 3, "cancelled"),
      ],
    );
    expect(wrappedResolved.parameters[0]?.type).toMatchObject({
      kind: "enum",
      values: ["hold", "confirmed", "cancelled"],
    });

    const unlabeledKind = catalogType("ns.ticket_status", "ns", "ticket_status", "");
    const [inferredResolved] = resolveProcedureCatalog(
      [
        procedure({
          procedure_id: "ns.hold_seat",
          schema: "ns",
          name: "hold_seat",
          signature: "status ns.ticket_status",
          return_type: "VOID",
        }),
      ],
      [parameter("ns.hold_seat", "status", 1, "ns.ticket_status", { type_id: "ns.ticket_status", not_null: true })],
      [unlabeledKind],
      [
        catalogField("ns.ticket_status", "hold", 1, "hold"),
        catalogField("ns.ticket_status", "confirmed", 2, "confirmed"),
      ],
    );
    expect(inferredResolved.parameters[0]?.type).toMatchObject({
      kind: "enum",
      values: ["hold", "confirmed"],
    });

    const [orphanResolved] = resolveProcedureCatalog(
      [
        procedure({
          procedure_id: "ns.orphan_status",
          schema: "ns",
          name: "orphan_status",
          signature: "status ns.ticket_status",
          return_type: "VOID",
        }),
      ],
      [parameter("ns.orphan_status", "status", 1, "ns.ticket_status", { type_id: "ns.ticket_status", not_null: true })],
      [],
      [
        catalogField("ns.ticket_status", "hold", 1, "hold"),
        catalogField("ns.ticket_status", "confirmed", 2, "confirmed"),
      ],
    );
    expect(orphanResolved.parameters[0]?.type).toMatchObject({
      kind: "enum",
      values: ["hold", "confirmed"],
    });

    const customerHover = hoverForIdentifier("api.create_customer", [createCustomer]);
    expect(customerHover).toContain("address api.address NOT NULL");
    expect(customerHover).toContain("api.address");
    expect(customerHover).toContain("city TEXT NOT NULL");
    expect(customerHover).toContain("country TEXT NOT NULL");
  });
});

describe("procedure status", () => {
  it("marks missing implementations as unimplemented and recent invocation errors as warnings", () => {
    const [ready] = resolveProcedureCatalog([procedure()], [], [], []);
    const [missing] = resolveProcedureCatalog(
      [procedure({ implementation: "missing", revision_id: null })],
      [],
      [],
      [],
    );
    const now = Date.parse("2026-09-11T12:00:00.000Z");
    const logs: SystemProcedureLogRow[] = [
      {
        timestamp: "2026-09-11T11:00:00.000Z",
        node_id: "n1",
        execution_id: "01K",
        request_id: "01K",
        procedure_id: ready.id,
        module_id: "backend",
        revision_id: ready.revisionId,
        actor: "root",
        origin: "http",
        outcome: "error",
        channel: "invocation",
        level: "error",
        error_code: "INVALID_ARGUMENTS",
        message: "failed",
        duration_ms: 12,
      },
    ];

    expect(deriveProcedureStatus(ready, [], now)).toBe("ready");
    expect(deriveProcedureStatus(missing, [], now)).toBe("unimplemented");
    expect(deriveProcedureStatus(ready, logs, now)).toBe("warning");
    expect(toProcedureListItem(ready, logs, [], now).calls24h).toBe(1);
    expect(toProcedureListItem(ready, logs, [], now).errors24h).toBe(1);
  });

  it("derives lastUpdatedMs from revision created_at epoch strings", () => {
    const [resolved] = resolveProcedureCatalog([procedure()], [], [], []);
    const createdAtMs = Date.parse("2024-10-24T12:41:00.000Z");
    const item = toProcedureListItem(
      resolved,
      [],
      [
        {
          module_id: "backend",
          revision_id: resolved.revisionId ?? "backend:84ac91abcdef",
          artifact_id: "84ac91abcdef",
          artifact_bytes: 86016,
          contract_hash: "hash",
          created_at: String(createdAtMs),
          is_current: true,
          exports: resolved.id,
        },
      ],
    );
    expect(item.lastUpdatedMs).toBe(createdAtMs);
  });

  it("does not inherit the current project revision for inline procedures", () => {
    const [inline] = resolveProcedureCatalog(
      [
        procedure({
          procedure_id: "ns.book_tickets",
          schema: "ns",
          name: "book_tickets",
          implementation: "inline",
          module_id: null,
          revision_id: "inline:deadbeef",
          language: "JAVASCRIPT",
          source: "return input.venue_id;",
        }),
      ],
      [],
      [],
      [],
    );
    const createdAtMs = Date.parse("2024-10-24T12:41:00.000Z");
    const revisions = [
      {
        module_id: "backend",
        revision_id: "backend:84ac91abcdef",
        artifact_id: "84ac91abcdef",
        artifact_bytes: 86016,
        contract_hash: "hash",
        created_at: String(createdAtMs),
        is_current: true,
        exports: "api.create_order",
      },
    ];
    const logMs = Date.parse("2026-09-11T11:00:00.000Z");
    const item = toProcedureListItem(
      inline,
      [
        {
          timestamp: "2026-09-11T11:00:00.000Z",
          node_id: "n1",
          execution_id: "01K",
          request_id: "01K",
          procedure_id: inline.id,
          module_id: null,
          revision_id: inline.revisionId,
          actor: "root",
          origin: "sql",
          outcome: "ok",
          channel: "invocation",
          level: "info",
          error_code: null,
          message: "ok",
          duration_ms: 8,
        },
      ],
      revisions,
      logMs + 1_000,
    );

    expect(inline.source).toBe("return input.venue_id;");
    expect(inline.language).toBe("JAVASCRIPT");
    expect(currentRevisionForProcedure(inline, revisions)).toBeNull();
    expect(item.revisionId).toBe("inline:deadbeef");
    expect(item.lastUpdatedMs).toBe(logMs);
    expect(item.calls24h).toBe(1);
  });
});

describe("typed test values", () => {
  it("generates scalar, nullable, enum, array, and composite controls and validates them", () => {
    const [resolved] = resolveProcedureCatalog(
      [
        procedure({
          procedure_id: "api.create_order",
          schema: "api",
          name: "create_order",
          return_type: "UUID",
        }),
      ],
      [
        parameter("api.create_order", "customer_id", 1, "UUID", { not_null: true }),
        parameter("api.create_order", "quantity", 2, "INT", { not_null: true }),
        parameter("api.create_order", "comment", 3, "TEXT"),
        parameter("api.create_order", "priority", 4, "BOOLEAN", { not_null: true }),
        parameter("api.create_order", "status", 5, "chat.message_status", {
          type_id: "chat.message_status",
        }),
        parameter("api.create_order", "items", 6, "api.order_item", {
          type_id: "api.order_item",
          is_array: true,
          not_null: true,
        }),
        parameter("api.create_order", "address", 7, "api.address", {
          type_id: "api.address",
          not_null: true,
        }),
      ],
      [statusEnum, orderItemType, addressType],
      [...enumFields, ...orderItemFields, ...addressFields],
    );

    const values = defaultTestValues(resolved);
    expect(values.comment).toBeNull();
    expect(values.priority).toBe(false);
    expect(values.status).toBeNull();
    expect(values.items).toEqual([]);
    expect(values.address).toEqual({ city: "", country: "" });

    const requiredErrors = validateTestValues(resolved, values);
    expect(requiredErrors.map((error) => error.path)).toEqual(
      expect.arrayContaining(["customer_id", "quantity", "address.city", "address.country"]),
    );

    let next = setPathValue(values, "customer_id", "2c1c0a1a-1111-4111-8111-aaaaaaaaaaaa");
    next = setPathValue(next, "quantity", "2");
    next = setPathValue(next, "comment", "rush");
    next = setPathValue(next, "status", "sent");
    next = setPathValue(next, "items", [{ sku: "ABC-100", quantity: "2" }]);
    next = setPathValue(next, "address.city", "Austin");
    next = setPathValue(next, "address.country", "US");

    expect(validateTestValues(resolved, next)).toEqual([]);
    expect(buildInvocationBody(resolved, next)).toEqual({
      customer_id: "2c1c0a1a-1111-4111-8111-aaaaaaaaaaaa",
      quantity: 2,
      comment: "rush",
      priority: false,
      status: "sent",
      items: [{ sku: "ABC-100", quantity: 2 }],
      address: { city: "Austin", country: "US" },
    });
  });
});

describe("SQL CALL completions", () => {
  it("filters procedures, inserts CALL snippets, and tracks the active parameter", () => {
    const procedures = resolveProcedureCatalog(
      [
        procedure(),
        procedure({
          procedure_id: "chat.join_room",
          name: "join_room",
          signature: "room_id UUID",
          return_type: "VOID",
          comment: null,
        }),
        procedure({
          procedure_id: "api.health",
          schema: "api",
          name: "health",
          signature: "",
          return_type: "BOOLEAN",
          comment: null,
        }),
      ],
      [
        parameter("chat.send_message", "conversation_id", 1, "UUID", { not_null: true }),
        parameter("chat.send_message", "content", 2, "TEXT", { not_null: true }),
        parameter("chat.send_message", "notify", 3, "BOOLEAN"),
        parameter("chat.join_room", "room_id", 1, "UUID", { not_null: true }),
      ],
      [messageType, statusEnum],
      [...messageFields, ...enumFields],
    );

    expect(filterProceduresByPartial(procedures, "cha").map((item) => item.id)).toEqual([
      "chat.join_room",
      "chat.send_message",
    ]);
    const sendMessage = procedures.find((item) => item.id === "chat.send_message")!;
    const entries = procedureCompletionEntries([sendMessage]);
    expect(entries[0]?.insertText).toContain("CALL chat.send_message(");
    expect(entries[0]?.insertText).toContain("${1:conversation_id}");
    expect(entries[0]?.detail).toContain("→ chat.message");
    expect(formatCallSnippet(sendMessage)).toContain("CALL chat.send_message(");
    expect(parseCallCompletionContext("CALL cha")).toEqual({
      kind: "name",
      partial: "cha",
      replaceFrom: 5,
    });
    expect(parseCallCompletionContext("CALL chat.send_message(")).toEqual({
      kind: "arguments",
      procedureId: "chat.send_message",
      activeParameter: 0,
    });
    expect(
      parseCallCompletionContext("CALL chat.send_message(\n    '2c...uuid...',\n    "),
    ).toEqual({
      kind: "arguments",
      procedureId: "chat.send_message",
      activeParameter: 1,
    });
    expect(signatureHelpLabel(sendMessage)).toContain("content TEXT NOT NULL");
    expect(signatureHelpLabel(sendMessage)).toContain("RETURNS chat.message");

    const notify = sendMessage.parameters[2]!.type;
    expect(valueSuggestionsForType(notify).map((item) => item.label)).toEqual(
      expect.arrayContaining(["TRUE", "FALSE", "NULL"]),
    );
    const statusType = resolveProcedureCatalog(
      [procedure({ procedure_id: "chat.set_status", name: "set_status", return_type: "VOID" })],
      [parameter("chat.set_status", "status", 1, "chat.message_status", { type_id: "chat.message_status" })],
      [statusEnum],
      enumFields,
    )[0]!.parameters[0]!.type;
    expect(valueSuggestionsForType(statusType).map((item) => item.label)).toEqual(
      expect.arrayContaining(["'sent'", "'delivered'", "'read'", "NULL"]),
    );

    expect(completionsForCallContext({ kind: "name", partial: "cha", replaceFrom: 5 }, procedures).map((item) => item.label)).toEqual(
      ["chat.join_room", "chat.send_message"],
    );
    expect(completionsForCallContext({ kind: "name", partial: "cha", replaceFrom: 5 }, procedures)[0]?.insertText).not.toMatch(/^CALL /i);

    const argumentHelp = signatureHelpForCall(
      { kind: "arguments", procedureId: "chat.send_message", activeParameter: 1 },
      procedures,
    );
    expect(argumentHelp?.activeParameter).toBe(1);
    expect(argumentHelp?.parameters[1]?.label).toContain("content TEXT NOT NULL");
    expect(argumentHelp?.label).toContain("RETURNS chat.message");

    const hover = hoverForIdentifier("chat.send_message", procedures);
    expect(hover).toContain("Parameters:");
    expect(hover).toContain("conversation_id UUID NOT NULL");
    expect(hover).toContain("Returns:");
    expect(hover).toContain("chat.message");
    expect(hover).toContain("Security:");
    expect(hover).toContain("INVOKER");
    expect(hover).toContain("Handles sending messages in chat.");

    const booleanSuggestions = completionsForCallContext(
      { kind: "arguments", procedureId: "chat.send_message", activeParameter: 2 },
      procedures,
    );
    expect(booleanSuggestions.map((item) => item.label)).toEqual(expect.arrayContaining(["TRUE", "FALSE", "NULL"]));
  });
});
