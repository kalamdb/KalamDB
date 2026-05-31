import { describe, expect, it } from "vitest";
import {
  reconcileActiveNamespace,
  tablesForNamespace,
} from "./NamespaceTableBrowser";
import type { StudioNamespace } from "./types";

const schema: StudioNamespace[] = [
  {
    database: "database",
    name: "default",
    tables: [
      {
        database: "database",
        namespace: "default",
        name: "events",
        tableType: "shared",
        columns: [
          {
            name: "id",
            dataType: "BIGINT",
            isNullable: false,
            isPrimaryKey: true,
            ordinal: 1,
          },
          {
            name: "payload",
            dataType: "JSON",
            isNullable: true,
            isPrimaryKey: false,
            ordinal: 2,
          },
        ],
      },
    ],
  },
  {
    database: "database",
    name: "analytics",
    tables: [
      {
        database: "database",
        namespace: "analytics",
        name: "daily_rollups",
        tableType: "user",
        columns: [
          {
            name: "event_total",
            dataType: "INT",
            isNullable: false,
            isPrimaryKey: false,
            ordinal: 1,
          },
        ],
      },
    ],
  },
];

describe("NamespaceTableBrowser", () => {
  it("filters tables inside the selected namespace by table or column name", () => {
    expect(tablesForNamespace(schema, "default", "").map((table) => table.name)).toEqual([
      "events",
    ]);
    expect(
      tablesForNamespace(schema, "default", "payload").map((table) => table.name),
    ).toEqual(["events"]);
    expect(
      tablesForNamespace(schema, "default", "daily").map((table) => table.name),
    ).toEqual([]);
  });

  it("does not snap manual namespace changes back to the selected table namespace", () => {
    const namespaces = ["default", "analytics"];

    expect(
      reconcileActiveNamespace({
        activeNamespace: "analytics",
        namespaces,
        selectedTableKey: "default.events",
        previousSelectedTableKey: "default.events",
      }),
    ).toBe("analytics");

    expect(
      reconcileActiveNamespace({
        activeNamespace: "analytics",
        namespaces,
        selectedTableKey: "default.events",
        previousSelectedTableKey: null,
      }),
    ).toBe("default");
  });
});
