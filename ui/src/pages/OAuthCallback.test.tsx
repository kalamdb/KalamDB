// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import OAuthCallback from "@/pages/OAuthCallback";

const mockLoginWithExternalToken = vi.fn();
const mockNavigate = vi.fn();
const mockConsumeOAuthRedirect = vi.fn();

vi.mock("react-router-dom", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-router-dom")>();
  return {
    ...actual,
    useNavigate: () => mockNavigate,
  };
});

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({
    loginWithExternalToken: mockLoginWithExternalToken,
  }),
}));

vi.mock("@/lib/oauth", () => ({
  consumeOAuthRedirect: (...args: unknown[]) => mockConsumeOAuthRedirect(...args),
}));

describe("OAuthCallback", () => {
  beforeEach(() => {
    mockLoginWithExternalToken.mockReset();
    mockNavigate.mockReset();
    mockConsumeOAuthRedirect.mockReset();
    Object.defineProperty(window, "location", {
      configurable: true,
      value: new URL("https://admin.example.com/ui/oauth/callback?code=code-1&state=state-1"),
    });
  });

  it("logs in with the provider token and navigates to the saved return path", async () => {
    mockConsumeOAuthRedirect.mockResolvedValue({ token: "provider.id.token", returnTo: "/sql" });
    mockLoginWithExternalToken.mockResolvedValue(undefined);

    render(
      <MemoryRouter>
        <OAuthCallback />
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(mockLoginWithExternalToken).toHaveBeenCalledWith("provider.id.token");
    });
    expect(mockNavigate).toHaveBeenCalledWith("/sql", { replace: true });
  });

  it("shows a failure state for invalid callback state", async () => {
    mockConsumeOAuthRedirect.mockRejectedValue(new Error("OAuth login state did not match"));

    render(
      <MemoryRouter>
        <OAuthCallback />
      </MemoryRouter>,
    );

    expect(await screen.findByText("OAuth login state did not match")).toBeTruthy();
    expect(mockLoginWithExternalToken).not.toHaveBeenCalled();
    expect(mockNavigate).not.toHaveBeenCalled();
  });
});