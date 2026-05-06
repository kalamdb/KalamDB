import { executeSql } from "@/lib/kalam-client";
import { getDb } from "@/lib/db";
import type { SystemUserListRow } from "@/lib/models";
import { system_users } from "@/lib/schema";
import { isNull, asc } from "drizzle-orm";
import {
  buildCreateUserSql,
  buildDeleteUserSql,
  buildUpdateUserEmailSql,
  buildUpdateUserPasswordSql,
  buildUpdateUserRoleSql,
  buildUpdateUserStorageIdSql,
  buildUpdateUserStorageModeSql,
  type CreateUserInput,
  type UpdateUserInput,
} from "@/services/sql/queries/userQueries";

export type User = SystemUserListRow;

export type { CreateUserInput, UpdateUserInput };

export async function fetchUsers(): Promise<User[]> {
  const db = getDb();
  return db
    .select({
      user_id: system_users.user_id,
      role: system_users.role,
      email: system_users.email,
      auth_type: system_users.auth_type,
      auth_data: system_users.auth_data,
      storage_mode: system_users.storage_mode,
      storage_id: system_users.storage_id,
      created_at: system_users.created_at,
      updated_at: system_users.updated_at,
      last_seen: system_users.last_seen,
      deleted_at: system_users.deleted_at,
      failed_login_attempts: system_users.failed_login_attempts,
      locked_until: system_users.locked_until,
      last_login_at: system_users.last_login_at,
    })
    .from(system_users)
    .where(isNull(system_users.deleted_at))
    .orderBy(asc(system_users.user_id));
}

export async function createUser(input: CreateUserInput): Promise<void> {
  await executeSql(buildCreateUserSql(input));
}

export async function updateUser(username: string, input: UpdateUserInput): Promise<void> {
  if (input.role) {
    await executeSql(buildUpdateUserRoleSql(username, input.role));
  }
  if (input.password) {
    await executeSql(buildUpdateUserPasswordSql(username, input.password));
  }
  if (input.email !== undefined) {
    await executeSql(buildUpdateUserEmailSql(username, input.email));
  }
  if (input.storage_mode !== undefined && input.storage_mode !== null) {
    await executeSql(buildUpdateUserStorageModeSql(username, input.storage_mode));
  }
  if (input.storage_id !== undefined) {
    await executeSql(buildUpdateUserStorageIdSql(username, input.storage_id));
  }
}

export async function deleteUser(username: string): Promise<void> {
  await executeSql(buildDeleteUserSql(username));
}
