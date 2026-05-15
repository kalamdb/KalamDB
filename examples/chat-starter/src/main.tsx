import React from "react";
import { createRoot } from "react-dom/client";
import { KalamProvider } from "@kalamdb/react";
import { getClient } from "./client";
import { App } from "./components/App";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <KalamProvider client={getClient()}>
      <App />
    </KalamProvider>
  </React.StrictMode>,
);
