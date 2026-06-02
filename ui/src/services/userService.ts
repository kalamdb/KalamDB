import { executeSql } from "@/lib/kalam-client";
import { getDb } from "@/lib/db";
import type { SystemUserListRow } from "@/lib/models";
import { system_users } from "@/lib/schema";
import { and, asc, isNull, like, sql, type SQL } from "drizzle-orm";
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

export interface UserListFilters {
  search?: string;
  limit?: number;
  offset?: number;
}

export interface UserListResult {
  users: User[];
  hasMore: boolean;
}

export type { CreateUserInput, UpdateUserInput };

const INVITE_USER_ID_PATTERN = "invite_%";

const userSelect = {
  user_id: system_users.user_id,
  role: system_users.role,
  name: system_users.name,
  email: system_users.email,
  auth_type: system_users.auth_type,
  auth_data: system_users.auth_data,
  storage_mode: system_users.storage_mode,
  storage_id: system_users.storage_id,
  created_at: system_users.created_at,
  updated_at: system_users.updated_at,
  last_seen: system_users.last_seen,
  deleted_at: system_users.deleted_at,
  invite_expires_at: system_users.invite_expires_at,
  invited_by: system_users.invited_by,
  failed_login_attempts: system_users.failed_login_attempts,
  locked_until: system_users.locked_until,
  last_login_at: system_users.last_login_at,
} as const;

function buildUserSearchCondition(search?: string): SQL | undefined {
  const normalizedSearch = search?.trim();
  if (!normalizedSearch) {
    return undefined;
  }

  const searchPattern = `%${normalizedSearch}%`;
  return sql`(
    ${system_users.user_id} LIKE ${searchPattern}
    OR ${system_users.name} LIKE ${searchPattern}
    OR ${system_users.email} LIKE ${searchPattern}
    OR ${system_users.role} LIKE ${searchPattern}
  )`;
}

function buildInviteConditions(): SQL[] {
  return [isNull(system_users.deleted_at), like(system_users.user_id, INVITE_USER_ID_PATTERN)];
}

function buildUserConditions(search?: string): SQL[] {
  const conditions: SQL[] = [
    isNull(system_users.deleted_at),
    sql`${system_users.user_id} NOT LIKE ${INVITE_USER_ID_PATTERN}`,
  ];

  const searchCondition = buildUserSearchCondition(search);
  if (searchCondition) {
    conditions.push(searchCondition);
  }

  return conditions;
}

export async function fetchInviteUsers(): Promise<User[]> {
  const db = getDb();

  return db
    .select(userSelect)
    .from(system_users)
    .where(and(...buildInviteConditions()))
    .orderBy(asc(system_users.user_id));
}

export async function fetchUsers(filters?: UserListFilters): Promise<UserListResult> {
  const db = getDb();
  const pageSize = filters?.limit ?? 25;

  const rows = await db
    .select(userSelect)
    .from(system_users)
    .where(and(...buildUserConditions(filters?.search)))
    .orderBy(asc(system_users.user_id))
    .limit(pageSize + 1)
    .offset(filters?.offset ?? 0);

  return {
    users: rows.slice(0, pageSize),
    hasMore: rows.length > pageSize,
  };
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

export async function reinviteUserInvite(invite: User, inviteExpiresAt: number): Promise<void> {
  if (invite.auth_type !== "oidc_invite") {
    throw new Error("Only OIDC invites can be reinvited");
  }
  if (!invite.email?.trim()) {
    throw new Error("Invite email is required");
  }

  await deleteUser(invite.user_id);
  await createUser({
    auth_type: "oidc_invite",
    email: invite.email,
    role: invite.role,
    invite_expires_at: inviteExpiresAt,
    storage_mode: invite.storage_mode === "region" ? "region" : "table",
    storage_id: invite.storage_id,
  });
}
