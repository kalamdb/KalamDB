import { describe, expect, it } from "vitest";
import { isReadOnlyNamespace } from "../table-editor/types";
import {
  preferredExplorerNamespace,
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

  it("opens on an application namespace instead of the built-in dba catalog", () => {
    expect(
      reconcileActiveNamespace({
        activeNamespace: "",
        namespaces: ["dba", "default", "home_check", "system"],
        selectedTableKey: null,
        previousSelectedTableKey: null,
      }),
    ).toBe("home_check");
  });

  it("treats only exact built-in names as catalogs, so prefixed application schemas stay editable", () => {
    expect(preferredExplorerNamespace(["dba", "dba_metrics", "system_metrics"])).toBe("dba_metrics");
    expect(preferredExplorerNamespace(["SYSTEM", "Public_app"])).toBe("Public_app");
    expect(isReadOnlyNamespace("dba")).toBe(true);
    expect(isReadOnlyNamespace("DBA")).toBe(true);
    expect(isReadOnlyNamespace("system")).toBe(true);
    expect(isReadOnlyNamespace("information_schema")).toBe(true);
    expect(isReadOnlyNamespace("dba_metrics")).toBe(false);
    expect(isReadOnlyNamespace("system_metrics")).toBe(false);
    expect(isReadOnlyNamespace("default")).toBe(false);
  });
});
