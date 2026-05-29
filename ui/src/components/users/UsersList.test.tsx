// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import type { User } from "@/services/userService";
import { UsersList } from "./UsersList";

const mockDeleteUserMutation = vi.fn();
const mockInvitesRefetch = vi.fn();
const mockUsersRefetch = vi.fn();
let mockInvites: User[] = [];
let mockUserPage = { users: [] as User[], hasMore: false };

vi.mock("@/lib/auth", () => ({
  useAuth: () => ({ user: { username: "root", role: "system" } }),
}));

vi.mock("@/store/apiSlice", () => ({
  useCreateUserMutation: () => [vi.fn()],
  useDeleteUserMutation: () => [mockDeleteUserMutation],
  useGetInviteUsersListQuery: () => ({
    data: mockInvites,
    isFetching: false,
    error: null,
    refetch: mockInvitesRefetch,
  }),
  useGetUsersListQuery: () => ({
    data: mockUserPage,
    isFetching: false,
    error: null,
    refetch: mockUsersRefetch,
  }),
  useGetStoragesQuery: () => ({ data: [] }),
  useReinviteUserMutation: () => [vi.fn()],
  useUpdateUserMutation: () => [vi.fn()],
}));

function user(overrides: Partial<User>): User {
  return {
    user_id: "user-1",
    role: "user",
    email: "user@example.org",
    auth_type: "password",
    auth_data: null,
    storage_mode: "table",
    storage_id: null,
    failed_login_attempts: 0,
    locked_until: null,
    last_login_at: null,
    last_seen: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    deleted_at: null,
    invite_expires_at: null,
    invited_by: null,
    ...overrides,
  };
}

describe("UsersList", () => {
  beforeEach(() => {
    mockDeleteUserMutation.mockReturnValue({ unwrap: vi.fn().mockResolvedValue(undefined) });
    mockInvitesRefetch.mockReset();
    mockUsersRefetch.mockReset();
  });

  afterEach(() => {
    cleanup();
    mockInvites = [];
    mockUserPage = { users: [], hasMore: false };
    vi.useRealTimers();
  });

  it("shows pending invite-prefix users above paged users with expired invites marked", () => {
    vi.setSystemTime(new Date("2026-01-10T00:00:00Z"));
    mockInvites = [
      user({
        user_id: "invite_alice",
        role: "dba",
        email: "alice@example.org",
        invite_expires_at: "2026-01-11T00:00:00Z",
        created_at: "2026-01-02T00:00:00Z",
      }),
      user({
        user_id: "invite_bob",
        role: "service",
        email: "bob@example.org",
        invite_expires_at: "2026-01-09T00:00:00Z",
        created_at: "2026-01-03T00:00:00Z",
      }),
    ];
    mockUserPage = {
      users: [
        user({
          user_id: "dev-carol",
          role: "dba",
          email: "carol@example.org",
          auth_type: "oidc",
        }),
      ],
      hasMore: true,
    };

    render(<UsersList />);

    const invites = screen.getByRole("region", { name: /pending invites/i });
    const users = screen.getByRole("region", { name: /users list/i });

    expect(invites.compareDocumentPosition(users) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(invites).getByText("invite_alice")).toBeTruthy();
    expect(within(invites).getByText("alice@example.org")).toBeTruthy();
    expect(within(invites).getByRole("button", { name: /delete invite invite_alice/i })).toBeTruthy();
    expect(within(invites).getByRole("button", { name: /reinvite alice@example.org/i })).toBeTruthy();

    const expiredInviteRow = within(invites).getByText("invite_bob").closest("tr");
    expect(expiredInviteRow?.className).toContain("bg-red-50");
    expect(within(invites).getByText(/expired/i)).toBeTruthy();

    expect(within(users).queryByText("invite_alice")).toBeNull();
    expect(within(users).getByText("dev-carol")).toBeTruthy();
    expect(screen.getByPlaceholderText(/search users/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: /next users page/i })).toBeTruthy();
  });
});
