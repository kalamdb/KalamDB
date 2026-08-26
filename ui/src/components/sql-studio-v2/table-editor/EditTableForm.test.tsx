// @vitest-environment jsdom

import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqlPreviewProvider } from "@/components/sql-preview";
import { Toaster } from "@/components/ui/toaster-provider";
import editorTabReducer, {
  startCreateTable,
} from "@/features/sql-studio/state/editorTabSlice";
import { EditTableForm } from "./EditTableForm";
import { emptyDraft, newDraftColumn } from "./types";

vi.mock("@kalamdb/client", () => ({
  FileRef: class FileRef {},
}));

function renderCreateTableForm() {
  const store = configureStore({
    reducer: {
      editorTab: editorTabReducer,
    },
  });
  const draft = emptyDraft("default");
  const createdAt = newDraftColumn();
  createdAt.id = "created-at";
  createdAt.name = "created_at";
  createdAt.type = "TIMESTAMP";
  createdAt.defaultExpr = "NOW()";
  const title = newDraftColumn();
  title.id = "title";
  title.name = "title";
  title.type = "TEXT";
  title.defaultExpr = "ULID()";
  draft.columns = [...draft.columns, createdAt, title];
  store.dispatch(startCreateTable({ namespace: "default", emptyDraft: draft }));

  render(
    <Provider store={store}>
      <Toaster>
        <SqlPreviewProvider>
          <EditTableForm schema={[]} />
        </SqlPreviewProvider>
      </Toaster>
    </Provider>,
  );

  return store;
}

describe("EditTableForm", () => {
  afterEach(() => {
    cleanup();
  });

  it("reorders draft columns with the row drag handle", () => {
    const store = renderCreateTableForm();

    const titleHandle = screen.getByLabelText("Reorder title");
    const idRow = screen.getByDisplayValue("id").closest("tr");
    if (!idRow) throw new Error("id row not found");

    fireEvent.dragStart(titleHandle);
    fireEvent.dragOver(idRow);
    fireEvent.drop(idRow);

    expect(
      store.getState().editorTab.draft?.columns.map((column) => column.name),
    ).toEqual(["title", "id", "created_at"]);
  });

  it("shows only compatible default presets for the selected datatype", () => {
    renderCreateTableForm();

    const timestampRow = screen.getByDisplayValue("created_at").closest("tr");
    if (!timestampRow) throw new Error("timestamp row not found");
    expect(within(timestampRow).getByText("NOW()")).toBeTruthy();
    expect(within(timestampRow).queryByText("SNOWFLAKE_ID()")).toBeNull();

    const textRow = screen.getByDisplayValue("title").closest("tr");
    if (!textRow) throw new Error("text row not found");
    expect(within(textRow).getByText("ULID()")).toBeTruthy();
    expect(within(textRow).queryByText("NOW()")).toBeNull();
  });

  it("hides access level and shows policies for shared tables", () => {
    const store = configureStore({
      reducer: {
        editorTab: editorTabReducer,
      },
    });
    store.dispatch(
      startCreateTable({
        namespace: "default",
        emptyDraft: emptyDraft("default", "shared"),
      }),
    );

    render(
      <Provider store={store}>
        <Toaster>
          <SqlPreviewProvider>
            <EditTableForm schema={[]} />
          </SqlPreviewProvider>
        </Toaster>
      </Provider>,
    );

    expect(screen.queryByText("Access level")).toBeNull();
    expect(screen.getByTestId("table-policies-section")).toBeTruthy();

    fireEvent.click(screen.getByTestId("table-policy-add"));
    fireEvent.change(screen.getByTestId("table-policy-name"), {
      target: { value: "owner_read" },
    });
    fireEvent.change(screen.getByTestId("table-policy-using"), {
      target: { value: "owner_id = CURRENT_USER()" },
    });

    expect(store.getState().editorTab.draft?.policies).toEqual([
      expect.objectContaining({
        name: "owner_read",
        command: "select",
        usingExpr: "owner_id = CURRENT_USER()",
        isNew: true,
      }),
    ]);
  });
});
