import { describe, expect, it } from "vitest";
import { resolveProcedureCatalog } from "./catalog";
import { formatCallSnippet, formatCompositeOutline, formatSignatureDetail } from "./format";
import { deriveProcedureStatus, toProcedureListItem } from "./status";
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
  CatalogParameterRow,
  CatalogProcedureRow,
  CatalogTypeFieldRow,
  CatalogTypeRow,
  ProcedureLogRecord,
} from "./types";

function procedure(overrides: Partial<CatalogProcedureRow> = {}): CatalogProcedureRow {
  return {
    procedureId: "chat.send_message",
    schema: "chat",
    name: "send_message",
    signature: "conversation_id UUID, content TEXT, notify BOOLEAN",
    returnType: "chat.message",
    implementation: "module",
    moduleId: "backend",
    revisionId: "backend:84ac91abcdef",
    security: "INVOKER",
    owner: "root",
    grants: "dba",
    comment: "Handles sending messages in chat.",
    ...overrides,
  };
}

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

const messageType: CatalogTypeRow = {
  typeId: "chat.message",
  namespaceId: "chat",
  name: "message",
  kind: "composite",
  tableId: "chat.message",
  sourceTypeId: null,
  comment: null,
};

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

const messageFields: CatalogTypeFieldRow[] = [
  {
    typeFieldId: "chat.message:id",
    typeId: "chat.message",
    name: "id",
    ordinal: 1,
    fieldTypeId: null,
    typeName: "UUID",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
  {
    typeFieldId: "chat.message:content",
    typeId: "chat.message",
    name: "content",
    ordinal: 2,
    fieldTypeId: null,
    typeName: "TEXT",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
];

const addressFields: CatalogTypeFieldRow[] = [
  {
    typeFieldId: "api.address:city",
    typeId: "api.address",
    name: "city",
    ordinal: 1,
    fieldTypeId: null,
    typeName: "TEXT",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
  {
    typeFieldId: "api.address:country",
    typeId: "api.address",
    name: "country",
    ordinal: 2,
    fieldTypeId: null,
    typeName: "TEXT",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
];

const orderItemFields: CatalogTypeFieldRow[] = [
  {
    typeFieldId: "api.order_item:sku",
    typeId: "api.order_item",
    name: "sku",
    ordinal: 1,
    fieldTypeId: null,
    typeName: "TEXT",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
  {
    typeFieldId: "api.order_item:quantity",
    typeId: "api.order_item",
    name: "quantity",
    ordinal: 2,
    fieldTypeId: null,
    typeName: "INT",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
];

const enumFields: CatalogTypeFieldRow[] = [
  {
    typeFieldId: "chat.message_status:sent",
    typeId: "chat.message_status",
    name: "sent",
    ordinal: 1,
    fieldTypeId: null,
    typeName: "sent",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
  {
    typeFieldId: "chat.message_status:delivered",
    typeId: "chat.message_status",
    name: "delivered",
    ordinal: 2,
    fieldTypeId: null,
    typeName: "delivered",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
  {
    typeFieldId: "chat.message_status:read",
    typeId: "chat.message_status",
    name: "read",
    ordinal: 3,
    fieldTypeId: null,
    typeName: "read",
    isArray: false,
    notNull: true,
    nonempty: false,
  },
];

describe("procedure catalog resolver", () => {
  it("resolves scalar parameters, return types, and comments from catalog rows", () => {
    const [resolved] = resolveProcedureCatalog(
      [procedure()],
      [
        parameter("chat.send_message", "conversation_id", 1, "UUID", { notNull: true }),
        parameter("chat.send_message", "content", 2, "TEXT", { notNull: true }),
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
          procedureId: "api.create_order",
          schema: "api",
          name: "create_order",
          signature: "customer_id UUID, items api.order_item[]",
          returnType: "UUID",
          comment: null,
        }),
      ],
      [
        parameter("api.create_order", "customer_id", 1, "UUID", { notNull: true }),
        parameter("api.create_order", "items", 2, "api.order_item", {
          typeId: "api.order_item",
          isArray: true,
          notNull: true,
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
          procedureId: "api.create_customer",
          schema: "api",
          name: "create_customer",
          returnType: "VOID",
        }),
      ],
      [
        parameter("api.create_customer", "name", 1, "TEXT", { notNull: true }),
        parameter("api.create_customer", "address", 2, "api.address", {
          typeId: "api.address",
          notNull: true,
        }),
        parameter("api.create_customer", "status", 3, "chat.message_status", {
          typeId: "chat.message_status",
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
    const customerHover = hoverForIdentifier("api.create_customer", [createCustomer]);
    expect(customerHover).toContain("address api.address NOT NULL");
    expect(customerHover).toContain("api.address");
    expect(customerHover).toContain("city TEXT NOT NULL");
    expect(customerHover).toContain("country TEXT NOT NULL");
  });
});

describe("procedure status", () => {
  it("marks missing implementations as errors and recent invocation errors as warnings", () => {
    const [ready] = resolveProcedureCatalog([procedure()], [], [], []);
    const [missing] = resolveProcedureCatalog(
      [procedure({ implementation: "missing", revisionId: null })],
      [],
      [],
      [],
    );
    const now = Date.parse("2026-09-11T12:00:00.000Z");
    const logs: ProcedureLogRecord[] = [
      {
        timestamp: "2026-09-11T11:00:00.000Z",
        nodeId: "n1",
        executionId: "01K",
        requestId: "01K",
        procedureId: ready.id,
        moduleId: "backend",
        revisionId: ready.revisionId,
        actor: "root",
        origin: "http",
        outcome: "error",
        channel: "invocation",
        level: "error",
        errorCode: "INVALID_ARGUMENTS",
        message: "failed",
        durationMs: 12,
      },
    ];

    expect(deriveProcedureStatus(ready, [], now)).toBe("ready");
    expect(deriveProcedureStatus(missing, [], now)).toBe("error");
    expect(deriveProcedureStatus(ready, logs, now)).toBe("warning");
    expect(toProcedureListItem(ready, logs, [], now).calls24h).toBe(1);
    expect(toProcedureListItem(ready, logs, [], now).errors24h).toBe(1);
  });
});

describe("typed test values", () => {
  it("generates scalar, nullable, enum, array, and composite controls and validates them", () => {
    const [resolved] = resolveProcedureCatalog(
      [
        procedure({
          procedureId: "api.create_order",
          schema: "api",
          name: "create_order",
          returnType: "UUID",
        }),
      ],
      [
        parameter("api.create_order", "customer_id", 1, "UUID", { notNull: true }),
        parameter("api.create_order", "quantity", 2, "INT", { notNull: true }),
        parameter("api.create_order", "comment", 3, "TEXT"),
        parameter("api.create_order", "priority", 4, "BOOLEAN", { notNull: true }),
        parameter("api.create_order", "status", 5, "chat.message_status", {
          typeId: "chat.message_status",
        }),
        parameter("api.create_order", "items", 6, "api.order_item", {
          typeId: "api.order_item",
          isArray: true,
          notNull: true,
        }),
        parameter("api.create_order", "address", 7, "api.address", {
          typeId: "api.address",
          notNull: true,
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
          procedureId: "chat.join_room",
          name: "join_room",
          signature: "room_id UUID",
          returnType: "VOID",
          comment: null,
        }),
        procedure({
          procedureId: "api.health",
          schema: "api",
          name: "health",
          signature: "",
          returnType: "BOOLEAN",
          comment: null,
        }),
      ],
      [
        parameter("chat.send_message", "conversation_id", 1, "UUID", { notNull: true }),
        parameter("chat.send_message", "content", 2, "TEXT", { notNull: true }),
        parameter("chat.send_message", "notify", 3, "BOOLEAN"),
        parameter("chat.join_room", "room_id", 1, "UUID", { notNull: true }),
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
      [procedure({ procedureId: "chat.set_status", name: "set_status", returnType: "VOID" })],
      [parameter("chat.set_status", "status", 1, "chat.message_status", { typeId: "chat.message_status" })],
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
