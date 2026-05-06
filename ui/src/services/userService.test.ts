import { afterEach, describe, expect, it, vi } from "vitest";
import type { User } from "@/services/userService";
import { createUser, updateUser } from "@/services/userService";

const executeSqlMock = vi.fn();

vi.mock("@/lib/kalam-client", () => ({
  executeSql: (sql: string) => executeSqlMock(sql),
}));

afterEach(() => {
  executeSqlMock.mockReset();
});

describe("User type", () => {
  it("uses the schema-backed user_id field directly", () => {
    const user: User = {
      user_id: "root",
      role: "system",
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
});
