// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import ProtectedRoute from "@/components/auth/ProtectedRoute";

const mockUseAuth = vi.fn();

vi.mock("@/lib/auth", () => ({
  useAuth: () => mockUseAuth(),
}));

function renderProtectedRoute(initialPath = "/dashboard") {
  return render(
    <MemoryRouter initialEntries={[initialPath]}>
      <Routes>
        <Route path="/login" element={<div>Login screen</div>} />
        <Route
          path="/dashboard"
          element={(
            <ProtectedRoute>
              <div>Secret dashboard</div>
            </ProtectedRoute>
          )}
        />
      </Routes>
    </MemoryRouter>,
  );
}

describe("ProtectedRoute", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    mockUseAuth.mockReset();
  });

  it("redirects unauthenticated users to the login route", async () => {
    mockUseAuth.mockReturnValue({
      isAuthenticated: false,
      isLoading: false,
      user: null,
      logout: vi.fn(),
    });

    renderProtectedRoute();

    expect(await screen.findByText("Login screen")).toBeTruthy();
  });

  it("shows access denied for authenticated non-admin users and lets them logout", () => {
    const logout = vi.fn();
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      isLoading: false,
      user: { role: "user" },
      logout,
    });

    renderProtectedRoute();

    expect(screen.getByText("Access Denied")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /logout and switch user/i }));
    expect(logout).toHaveBeenCalledTimes(1);
  });

  it("shows access denied for authenticated OIDC service users", () => {
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      isLoading: false,
      user: { id: "e_service", role: "service", email: "service@example.org" },
      logout: vi.fn(),
    });

    renderProtectedRoute();

    expect(screen.getByText("Access Denied")).toBeTruthy();
    expect(screen.getByText(/current role:/i).textContent).toContain("service");
  });

  it.each(["system", "dba"])("renders protected content for authenticated %s users", (role) => {
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      isLoading: false,
      user: { id: `e_${role}`, role, email: `${role}@example.org` },
      logout: vi.fn(),
    });

    renderProtectedRoute();

    expect(screen.getByText("Secret dashboard")).toBeTruthy();
  });
});
