import { describe, expect, it } from "vitest";
import authReducer, { login, refresh, checkAuth, loginWithExternalToken } from "@/store/authSlice";

const basePayload = {
  user: {
    id: "root",
    role: "system",
    email: "root@localhost",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
  admin_ui_access: true,
  expires_at: "2026-01-01T01:00:00Z",
  access_token: "token",
  refresh_token: "refresh",
  refresh_expires_at: "2026-01-02T00:00:00Z",
};

describe("authSlice user normalization", () => {
  it("falls back username to user.id on login fulfillment", () => {
    const state = authReducer(undefined, {
      type: login.fulfilled.type,
      payload: basePayload,
    });

    expect(state.user?.id).toBe("root");
    expect(state.user?.username).toBe("root");
  });

  it("preserves explicit username on refresh/checkAuth fulfillment", () => {
    const refreshState = authReducer(undefined, {
      type: refresh.fulfilled.type,
      payload: {
        ...basePayload,
        user: {
          ...basePayload.user,
          username: "admin",
        },
      },
    });

    const checkAuthState = authReducer(undefined, {
      type: checkAuth.fulfilled.type,
      payload: {
        ...basePayload,
        user: {
          ...basePayload.user,
          username: "admin",
        },
      },
    });

    expect(refreshState.user?.username).toBe("admin");
    expect(checkAuthState.user?.username).toBe("admin");
  });

  it("stores external bearer sessions on OIDC login fulfillment", () => {
    const state = authReducer(undefined, {
      type: loginWithExternalToken.fulfilled.type,
      payload: {
        ...basePayload,
        access_token: "provider.id.token",
        refresh_token: "",
      },
    });

    expect(state.isAuthenticated).toBe(true);
    expect(state.accessToken).toBe("provider.id.token");
    expect(state.user?.username).toBe("root");
    expect(state.error).toBeNull();
  });
});
