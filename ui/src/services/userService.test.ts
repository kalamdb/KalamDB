import { afterEach, describe, expect, it, vi } from "vitest";
import type { User } from "@/services/userService";
import {
  createUser,
  fetchInviteUsers,
  fetchUsers,
  reinviteUserInvite,
  updateUser,
} from "@/services/userService";

const executeSqlMock = vi.fn();
const selectMock = vi.fn();

vi.mock("@/lib/kalam-client", () => ({
  executeSql: (sql: string) => executeSqlMock(sql),
}));

vi.mock("@/lib/db", () => ({
  getDb: () => ({ select: selectMock }),
}));

afterEach(() => {
  executeSqlMock.mockReset();
  selectMock.mockReset();
});

describe("User type", () => {
  it("uses the schema-backed user_id field directly", () => {
    const user: User = {
      user_id: "root",
      role: "system",
      name: "Root Admin",
      email: "root@localhost",
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
    };

    expect(user.user_id).toBe("root");
  });

  it("creates users without issuing a direct system.users update", async () => {
    executeSqlMock.mockResolvedValue([]);

    await createUser({
      username: "test2",
      password: "Password123!",
      role: "user",
      storage_mode: "table",
      storage_id: "local",
    });

    expect(executeSqlMock).toHaveBeenCalledTimes(1);
    expect(executeSqlMock).toHaveBeenCalledWith(
      "CREATE USER 'test2' WITH PASSWORD 'Password123!' ROLE 'user' STORAGE_MODE 'table' STORAGE_ID 'local'",
    );
  });

  it("creates OIDC email invites through CREATE USER INVITE", async () => {
    executeSqlMock.mockResolvedValue([]);

    await createUser({
      auth_type: "oidc_invite",
      email: "alice@example.com",
      role: "dba",
      invite_expires_at: 1770000000000,
    });

    expect(executeSqlMock).toHaveBeenCalledTimes(1);
    expect(executeSqlMock).toHaveBeenCalledWith(
      "CREATE USER INVITE 'alice@example.com' ROLE 'dba' EXPIRES_AT 1770000000000",
    );
  });

  it("reinvites OIDC email invites by dropping the invite and creating a fresh one", async () => {
    executeSqlMock.mockResolvedValue([]);

    await reinviteUserInvite(
      {
        user_id: "invite_alice",
        role: "dba",
        name: null,
        email: "alice@example.com",
        auth_type: "oidc_invite",
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
        invite_expires_at: "2026-01-02T00:00:00Z",
        invited_by: "root",
      },
      1770000000000,
    );

    expect(executeSqlMock).toHaveBeenCalledTimes(2);
    expect(executeSqlMock).toHaveBeenNthCalledWith(1, "DROP USER 'invite_alice'");
    expect(executeSqlMock).toHaveBeenNthCalledWith(
      2,
      "CREATE USER INVITE 'alice@example.com' ROLE 'dba' EXPIRES_AT 1770000000000 STORAGE_MODE 'table'",
    );
  });

  it("updates supported fields with alter-user storage statements", async () => {
    executeSqlMock.mockResolvedValue([]);

    await updateUser("test2", {
      role: "dba",
      storage_mode: "region",
      storage_id: "archive",
    });

    expect(executeSqlMock).toHaveBeenCalledTimes(3);
    expect(executeSqlMock).toHaveBeenNthCalledWith(1, "ALTER USER 'test2' SET ROLE 'dba'");
    expect(executeSqlMock).toHaveBeenNthCalledWith(2, "ALTER USER 'test2' SET STORAGE_MODE 'region'");
    expect(executeSqlMock).toHaveBeenNthCalledWith(3, "ALTER USER 'test2' SET STORAGE_ID 'archive'");
  });

  it("can clear a user storage id via alter user", async () => {
    executeSqlMock.mockResolvedValue([]);

    await updateUser("test2", {
      storage_id: null,
    });

    expect(executeSqlMock).toHaveBeenCalledTimes(1);
    expect(executeSqlMock).toHaveBeenCalledWith("ALTER USER 'test2' SET STORAGE_ID NULL");
  });

  it("fetches invite users by invite prefix", async () => {
    const rows = [
      {
        user_id: "invite_alice",
        role: "dba",
        email: "alice@example.com",
        auth_type: "oidc_invite",
        auth_data: null,
        storage_mode: "table",
        storage_id: null,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        last_seen: null,
        deleted_at: null,
        invite_expires_at: "2026-01-02T00:00:00Z",
        invited_by: "root",
        failed_login_attempts: 0,
        locked_until: null,
        last_login_at: null,
      },
    ];
    const orderByMock = vi.fn().mockResolvedValue(rows);
    const whereMock = vi.fn().mockReturnValue({ orderBy: orderByMock });
    const fromMock = vi.fn().mockReturnValue({ where: whereMock });
    selectMock.mockReturnValue({ from: fromMock });

    const result = await fetchInviteUsers();

    expect(selectMock).toHaveBeenCalledTimes(1);
    expect(fromMock).toHaveBeenCalledTimes(1);
    expect(whereMock).toHaveBeenCalledTimes(1);
    expect(orderByMock).toHaveBeenCalledTimes(1);
    expect(result).toEqual(rows);
  });

  it("paginates normal users with backend search", async () => {
    const rows = Array.from({ length: 4 }, (_, index) => ({
      user_id: `dev-${index}`,
      role: "user",
      name: `Developer ${index}`,
      email: `dev-${index}@example.com`,
      auth_type: "password",
      auth_data: null,
      storage_mode: "table",
      storage_id: null,
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      last_seen: null,
      deleted_at: null,
      invite_expires_at: null,
      invited_by: null,
      failed_login_attempts: 0,
      locked_until: null,
      last_login_at: null,
    }));
    const offsetMock = vi.fn().mockResolvedValue(rows);
    const limitMock = vi.fn().mockReturnValue({ offset: offsetMock });
    const orderByMock = vi.fn().mockReturnValue({ limit: limitMock });
    const whereMock = vi.fn().mockReturnValue({ orderBy: orderByMock });
    const fromMock = vi.fn().mockReturnValue({ where: whereMock });
    selectMock.mockReturnValue({ from: fromMock });

    const result = await fetchUsers({ search: "dev", limit: 3, offset: 5 });

    expect(selectMock).toHaveBeenCalledTimes(1);
    expect(fromMock).toHaveBeenCalledTimes(1);
    expect(whereMock).toHaveBeenCalledTimes(1);
    expect(orderByMock).toHaveBeenCalledTimes(1);
    expect(limitMock).toHaveBeenCalledWith(4);
    expect(offsetMock).toHaveBeenCalledWith(5);
    expect(result.users).toHaveLength(3);
    expect(result.hasMore).toBe(true);
  });
});
