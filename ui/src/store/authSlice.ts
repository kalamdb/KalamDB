import { createSlice, createAsyncThunk, PayloadAction } from "@reduxjs/toolkit";
import {
  authApi,
  probeBackendReachability,
  type UserInfo,
  type LoginRequest,
  ApiRequestError,
} from "../lib/api";
import { setClientToken, clearClient } from "../lib/kalam-client";
import {
  clearExternalAuthToken,
  externalLoginResponse,
  loadExternalAuthToken,
  storeExternalAuthToken,
} from "../lib/oauth";

interface AuthState {
  user: UserInfo | null;
  isLoading: boolean;
  isAuthenticated: boolean;
  accessToken: string | null;
  expiresAt: string | null; // Store as string for serializability
  error: string | null;
}

const initialState: AuthState = {
  user: null,
  isLoading: true,
  isAuthenticated: false,
  accessToken: null,
  expiresAt: null,
  error: null,
};

function normalizeUserInfo(user: UserInfo): UserInfo {
  const normalizedUsername = user.username?.trim();
  return {
    ...user,
    username: normalizedUsername && normalizedUsername.length > 0 ? normalizedUsername : user.id,
  };
}

function extractAuthErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof ApiRequestError) {
    return error.apiError.message;
  }
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  return fallback;
}

async function authenticateExternalToken(token: string) {
  await setClientToken(token);
  const currentUser = await authApi.me();
  storeExternalAuthToken(token);
  return externalLoginResponse(token, currentUser);
}

export const login = createAsyncThunk(
  "auth/login",
  async (credentials: LoginRequest, { rejectWithValue }) => {
    try {
      const status = await probeBackendReachability();
      if (status.needs_setup) {
        return rejectWithValue("Server setup is not complete yet.");
      }

      const response = await authApi.login(credentials);
      clearExternalAuthToken();
      await setClientToken(response.access_token);
      return response;
    } catch (err) {
      if (err instanceof TypeError) {
        return rejectWithValue("KalamDB server is unreachable.");
      }
      return rejectWithValue(extractAuthErrorMessage(err, "Login failed"));
    }
  }
);

export const logout = createAsyncThunk("auth/logout", async () => {
  try {
    await authApi.logout();
  } catch {
    // Ignore logout errors
  } finally {
    clearExternalAuthToken();
    await clearClient();
  }
});

export const loginWithExternalToken = createAsyncThunk(
  "auth/loginWithExternalToken",
  async (token: string, { rejectWithValue }) => {
    try {
      const status = await probeBackendReachability();
      if (status.needs_setup) {
        return rejectWithValue("Server setup is not complete yet.");
      }

      return await authenticateExternalToken(token);
    } catch (err) {
      clearExternalAuthToken();
      await clearClient();
      if (err instanceof TypeError) {
        return rejectWithValue("KalamDB server is unreachable.");
      }
      return rejectWithValue(extractAuthErrorMessage(err, "External login failed"));
    }
  }
);

export const refresh = createAsyncThunk(
  "auth/refresh",
  async (_, { rejectWithValue }) => {
    try {
      const response = await authApi.refresh();
      clearExternalAuthToken();
      await setClientToken(response.access_token);
      return response;
    } catch (err) {
      const externalToken = loadExternalAuthToken();
      if (externalToken) {
        try {
          return await authenticateExternalToken(externalToken);
        } catch {
          clearExternalAuthToken();
        }
      }
      await clearClient();
      if (err instanceof ApiRequestError) {
        return rejectWithValue(err.apiError.message);
      }
      return rejectWithValue("Refresh failed");
    }
  }
);

export const checkAuth = createAsyncThunk(
  "auth/checkAuth",
  async (_, { rejectWithValue }) => {
    try {
      const response = await authApi.refresh();
      clearExternalAuthToken();
      await setClientToken(response.access_token);
      return response;
    } catch (err) {
      const externalToken = loadExternalAuthToken();
      if (externalToken) {
        try {
          return await authenticateExternalToken(externalToken);
        } catch {
          clearExternalAuthToken();
        }
      }
      await clearClient();
      return rejectWithValue("Not authenticated");
    }
  }
);

const authSlice = createSlice({
  name: "auth",
  initialState,
  reducers: {
    setLoading: (state, action: PayloadAction<boolean>) => {
      state.isLoading = action.payload;
    },
    clearError: (state) => {
      state.error = null;
    },
  },
  extraReducers: (builder) => {
    builder
      // Login
      .addCase(login.pending, (state) => {
        state.isLoading = true;
        state.error = null;
      })
      .addCase(login.fulfilled, (state, action) => {
        state.user = normalizeUserInfo(action.payload.user);
        state.accessToken = action.payload.access_token;
        state.expiresAt = action.payload.expires_at;
        state.isAuthenticated = true;
        state.isLoading = false;
        state.error = null;
      })
      .addCase(login.rejected, (state, action) => {
        state.isLoading = false;
        state.error = action.payload as string;
      })
      // External login
      .addCase(loginWithExternalToken.pending, (state) => {
        state.isLoading = true;
        state.error = null;
      })
      .addCase(loginWithExternalToken.fulfilled, (state, action) => {
        state.user = normalizeUserInfo(action.payload.user);
        state.accessToken = action.payload.access_token;
        state.expiresAt = action.payload.expires_at;
        state.isAuthenticated = true;
        state.isLoading = false;
        state.error = null;
      })
      .addCase(loginWithExternalToken.rejected, (state, action) => {
        state.isLoading = false;
        state.error = action.payload as string;
      })
      // Logout
      .addCase(logout.fulfilled, (state) => {
        state.user = null;
        state.accessToken = null;
        state.expiresAt = null;
        state.isAuthenticated = false;
        state.error = null;
      })
      // Refresh
      .addCase(refresh.fulfilled, (state, action) => {
        state.user = normalizeUserInfo(action.payload.user);
        state.accessToken = action.payload.access_token;
        state.expiresAt = action.payload.expires_at;
        state.isAuthenticated = true;
        state.error = null;
      })
      .addCase(refresh.rejected, (state) => {
        state.user = null;
        state.accessToken = null;
        state.expiresAt = null;
        state.isAuthenticated = false;
      })
      // Check Auth
      .addCase(checkAuth.pending, (state) => {
        state.isLoading = true;
      })
      .addCase(checkAuth.fulfilled, (state, action) => {
        state.user = normalizeUserInfo(action.payload.user);
        state.accessToken = action.payload.access_token;
        state.expiresAt = action.payload.expires_at;
        state.isAuthenticated = true;
        state.isLoading = false;
        state.error = null;
      })
      .addCase(checkAuth.rejected, (state) => {
        state.user = null;
        state.accessToken = null;
        state.expiresAt = null;
        state.isAuthenticated = false;
        state.isLoading = false;
      });
  },
});

export const { setLoading, clearError } = authSlice.actions;
export default authSlice.reducer;
