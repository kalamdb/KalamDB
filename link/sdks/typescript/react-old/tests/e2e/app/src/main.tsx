import React, { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { KalamProvider } from "@kalamdb/react";
import { getClient } from "./client-setup";
import { App } from "./App";

const params = new URLSearchParams(window.location.search);
const schemaSuffix = params.get("schema") ?? "default";
(globalThis as { __KDB_SCHEMA_SUFFIX__?: string }).__KDB_SCHEMA_SUFFIX__ = schemaSuffix;

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <KalamProvider client={getClient()}>
      <App />
    </KalamProvider>
  </StrictMode>,
);
