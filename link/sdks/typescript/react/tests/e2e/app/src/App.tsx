import React from "react";
import { DrizzlePage } from "./pages/DrizzlePage";
import { SqlPage } from "./pages/SqlPage";
import { MultiQueryPage } from "./pages/MultiQueryPage";
import { MutationsPage } from "./pages/MutationsPage";
import { SelectionPage } from "./pages/SelectionPage";
import { ColumnMappingPage } from "./pages/ColumnMappingPage";
import { RemountPage } from "./pages/RemountPage";
import { PartialFailurePage } from "./pages/PartialFailurePage";
import { LimitPage } from "./pages/LimitPage";
import { RefetchPage } from "./pages/RefetchPage";
import { CompositeKeyPage } from "./pages/CompositeKeyPage";
import { SelectTransformPage } from "./pages/SelectTransformPage";
import { MissingTablePage } from "./pages/MissingTablePage";
import { DisconnectPage } from "./pages/DisconnectPage";
import { BadAuthPage } from "./pages/BadAuthPage";
import { HighRowCountPage } from "./pages/HighRowCountPage";

const pages: Record<string, React.ComponentType> = {
  drizzle: DrizzlePage,
  sql: SqlPage,
  multi: MultiQueryPage,
  mutations: MutationsPage,
  selection: SelectionPage,
  "column-mapping": ColumnMappingPage,
  remount: RemountPage,
  "partial-failure": PartialFailurePage,
  limit: LimitPage,
  refetch: RefetchPage,
  "composite-key": CompositeKeyPage,
  "select-transform": SelectTransformPage,
  "missing-table": MissingTablePage,
  disconnect: DisconnectPage,
  "bad-auth": BadAuthPage,
  "high-rows": HighRowCountPage,
};

export function App() {
  const params = new URLSearchParams(window.location.search);
  const name = params.get("page") ?? "drizzle";
  const Page = pages[name];
  if (!Page) {
    return <p>Unknown page: {name}</p>;
  }
  return <Page />;
}
