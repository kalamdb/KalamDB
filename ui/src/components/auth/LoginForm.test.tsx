// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import LoginForm from "@/components/auth/LoginForm";

const mockLogin = vi.fn();
const mockUseAuth = vi.fn();
const mockLoginOptions = vi.fn();
const mockBuildOAuthAuthorizationUrl = vi.fn();

vi.mock("@/lib/auth", () => ({
  useAuth: () => mockUseAuth(),
}));

vi.mock("@/lib/api", () => ({
  authApi: {
    loginOptions: () => mockLoginOptions(),
  },
}));

vi.mock("@/lib/oauth", () => ({
  buildOAuthAuthorizationUrl: (...args: unknown[]) => mockBuildOAuthAuthorizationUrl(...args),
}));

describe("LoginForm", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    mockLogin.mockReset();
    mockUseAuth.mockReset();
    mockUseAuth.mockReturnValue({
      login: mockLogin,
      error: null,
      isLoading: false,
    });
    mockLoginOptions.mockResolvedValue({
      local: { enabled: true },
      oidc: null,
    });
    mockBuildOAuthAuthorizationUrl.mockResolvedValue("https://idp.example.com/auth");
  });

  it("submits canonical credentials with user + password", async () => {
    render(
      <MemoryRouter>
        <LoginForm />
      </MemoryRouter>,
    );

    fireEvent.change(screen.getByLabelText(/username/i), {
      target: { value: "root" },
    });
    fireEvent.change(screen.getByLabelText(/password/i), {
      target: { value: "kalamdb123" },
    });

    fireEvent.click(screen.getAllByRole("button", { name: /^log in$/i })[0]);

    expect(mockLogin).toHaveBeenCalledTimes(1);
    expect(mockLogin).toHaveBeenCalledWith({
      user: "root",
      password: "kalamdb123",
    });
  });

  it("shows validation error when username or password is missing", () => {
    render(
      <MemoryRouter>
        <LoginForm />
      </MemoryRouter>,
    );

    fireEvent.click(screen.getAllByRole("button", { name: /^log in$/i })[0]);

    expect(screen.getByText(/please enter username and password/i)).toBeTruthy();
    expect(mockLogin).not.toHaveBeenCalled();
  });

  it("hides local username and password controls when local auth is disabled", async () => {
    mockLoginOptions.mockResolvedValue({
      local: { enabled: false },
      oidc: null,
    });

    render(
      <MemoryRouter>
        <LoginForm />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(screen.queryByLabelText(/username/i)).toBeNull();
    });
    expect(screen.queryByLabelText(/password/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /^log in$/i })).toBeNull();
  });

  it("shows OIDC login and hides local credentials when local auth is disabled", async () => {
    mockLoginOptions.mockResolvedValue({
      local: { enabled: false },
      oidc: {
        enabled: true,
        display_name: "Dex",
        issuer: "https://idp.example.com/dex",
        client_id: "kalamdb-admin",
        authorization_endpoint: "https://idp.example.com/dex/auth",
        token_endpoint: "https://idp.example.com/dex/token",
        scopes: ["openid", "email"],
      },
    });

    render(
      <MemoryRouter>
        <LoginForm />
      </MemoryRouter>,
    );

    expect(await screen.findByRole("button", { name: /continue with dex/i })).toBeTruthy();
    expect(screen.queryByLabelText(/username/i)).toBeNull();
    expect(screen.queryByLabelText(/password/i)).toBeNull();
    expect(screen.queryByRole("button", { name: /^log in$/i })).toBeNull();
  });

  it("shows an explanatory warning when OIDC is enabled but discovery endpoints are missing", async () => {
    mockLoginOptions.mockResolvedValue({
      local: { enabled: true },
      oidc: {
        enabled: true,
        display_name: "Dex",
        issuer: "http://127.0.0.1:5556",
        client_id: "client",
        authorization_endpoint: null,
        token_endpoint: null,
        scopes: ["openid", "email", "profile"],
      },
    });

    render(
      <MemoryRouter>
        <LoginForm />
      </MemoryRouter>,
    );

    expect(await screen.findByText(/dex login is enabled on the server/i)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /continue with dex/i })).toBeNull();
    expect(screen.getByLabelText(/username/i)).toBeTruthy();
    expect(screen.getByLabelText(/password/i)).toBeTruthy();
  });

  it("shows both OIDC and local login when both methods are enabled", async () => {
    mockLoginOptions.mockResolvedValue({
      local: { enabled: true },
      oidc: {
        enabled: true,
        display_name: "Company SSO",
        issuer: "https://idp.example.com/realms/kalamdb",
        client_id: "kalamdb-admin",
        authorization_endpoint: "https://idp.example.com/auth",
        token_endpoint: "https://idp.example.com/token",
        scopes: ["openid", "profile"],
      },
    });

    render(
      <MemoryRouter>
        <LoginForm />
      </MemoryRouter>,
    );

    expect(await screen.findByRole("button", { name: /continue with company sso/i })).toBeTruthy();
    expect(screen.getByLabelText(/username/i)).toBeTruthy();
    expect(screen.getByLabelText(/password/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: /^log in$/i })).toBeTruthy();
  });

  it("starts the PKCE redirect for the configured OIDC option", async () => {
    const assign = vi.fn();
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...window.location, assign },
    });
    mockBuildOAuthAuthorizationUrl.mockResolvedValue("https://idp.example.com/auth?state=state-1");
    mockLoginOptions.mockResolvedValue({
      local: { enabled: false },
      oidc: {
        enabled: true,
        display_name: "Dex",
        issuer: "https://idp.example.com/dex",
        client_id: "kalamdb-admin",
        authorization_endpoint: "https://idp.example.com/dex/auth",
        token_endpoint: "https://idp.example.com/dex/token",
        scopes: ["openid"],
      },
    });

    render(
      <MemoryRouter>
        <LoginForm returnTo="/sql" />
      </MemoryRouter>,
    );

    fireEvent.click(await screen.findByRole("button", { name: /continue with dex/i }));

    expect(mockBuildOAuthAuthorizationUrl).toHaveBeenCalledWith(
      expect.objectContaining({ display_name: "Dex" }),
      "/sql",
    );
    await waitFor(() => {
      expect(assign).toHaveBeenCalledWith("https://idp.example.com/auth?state=state-1");
    });
  });
});
