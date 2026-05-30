// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import Settings from "./Settings";

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({
    user: {
      id: "user-system",
      username: "admin",
      email: "admin@example.org",
      role: "system",
    },
  }),
}));

vi.mock("@/components/settings/SettingsView", () => ({
  SettingsView: ({ filterCategory }: { filterCategory?: string }) => (
    <div data-testid="settings-view">{filterCategory ?? "all settings"}</div>
  ),
}));

vi.mock("./Cluster", () => ({
  default: () => <div>Cluster settings content</div>,
}));

vi.mock("./Storages", () => ({
  default: () => <div>Storage settings content</div>,
}));

function renderSettings(initialPath = "/settings") {
  return render(
    <MemoryRouter initialEntries={[initialPath]}>
      <Routes>
        <Route path="/settings" element={<Settings />} />
        <Route path="/settings/:category" element={<Settings />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("Settings", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders the Supabase-style settings sidebar with grouped KalamDB admin links", () => {
    renderSettings();

    const sidebar = screen.getByRole("navigation", { name: /settings sections/i });

    expect(screen.getByRole("heading", { name: "Settings" })).toBeTruthy();
    expect(sidebar.textContent).toContain("Configuration");
    expect(sidebar.textContent).toContain("All Settings");
    expect(sidebar.textContent).toContain("Cluster");
    expect(sidebar.textContent).toContain("Storages");
    expect(sidebar.textContent).toContain("Security");
    expect(screen.getByText("Current User")).toBeTruthy();
  });

  it("navigates from a settings link to the matching settings route", () => {
    renderSettings();

    const sidebar = screen.getByRole("navigation", { name: /settings sections/i });

    fireEvent.click(within(sidebar).getByRole("link", { name: "Cluster" }));

    expect(screen.getByText("Cluster settings content")).toBeTruthy();
  });
});
