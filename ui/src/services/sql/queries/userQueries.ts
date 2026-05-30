export interface CreateUserInput {
  username: string;
  password?: string;
  auth_type?: "password" | "oidc";
  auth_data?: string;
  role?: string;
  email?: string;
  storage_mode?: "table" | "region" | null;
  storage_id?: string | null;
}

export interface UpdateUserInput {
  role?: string;
  password?: string;
  email?: string;
  storage_mode?: "table" | "region" | null;
  storage_id?: string | null;
}

function escapeSqlLiteral(value: string): string {
  return value.replace(/'/g, "''");
}

export function buildCreateUserSql(input: CreateUserInput): string {
  const authType = (input.auth_type ?? "password").toLowerCase();
  let sql = `CREATE USER '${escapeSqlLiteral(input.username)}'`;

  if (authType === "oidc") {
    sql += ` WITH OIDC`;
    if (input.auth_data?.trim()) {
      sql += ` '${escapeSqlLiteral(input.auth_data.trim())}'`;
    }
  } else {
    const password = input.password?.trim();
    if (!password) {
      throw new Error("Password is required for password auth type");
    }
    sql += ` WITH PASSWORD '${escapeSqlLiteral(password)}'`;
  }

  if (input.role) {
    sql += ` ROLE '${escapeSqlLiteral(input.role)}'`;
  }
  if (input.email?.trim()) {
    sql += ` EMAIL '${escapeSqlLiteral(input.email.trim())}'`;
  }
  if (input.storage_mode) {
    sql += ` STORAGE_MODE '${escapeSqlLiteral(input.storage_mode)}'`;
  }
  if (input.storage_id?.trim()) {
    sql += ` STORAGE_ID '${escapeSqlLiteral(input.storage_id.trim())}'`;
  }
  return sql;
}

export function buildUpdateUserRoleSql(username: string, role: string): string {
  return `ALTER USER '${escapeSqlLiteral(username)}' SET ROLE '${escapeSqlLiteral(role)}'`;
}

export function buildUpdateUserPasswordSql(username: string, password: string): string {
  return `ALTER USER '${escapeSqlLiteral(username)}' SET PASSWORD '${escapeSqlLiteral(password)}'`;
}

export function buildUpdateUserEmailSql(username: string, email: string): string {
  return `ALTER USER '${escapeSqlLiteral(username)}' SET EMAIL '${escapeSqlLiteral(email)}'`;
}

export function buildUpdateUserStorageModeSql(
  username: string,
  storageMode: "table" | "region",
): string {
  return `ALTER USER '${escapeSqlLiteral(username)}' SET STORAGE_MODE '${escapeSqlLiteral(storageMode)}'`;
}

export function buildUpdateUserStorageIdSql(
  username: string,
  storageId: string | null,
): string {
  if (storageId === null) {
    return `ALTER USER '${escapeSqlLiteral(username)}' SET STORAGE_ID NULL`;
  }

  return `ALTER USER '${escapeSqlLiteral(username)}' SET STORAGE_ID '${escapeSqlLiteral(storageId.trim())}'`;
}

export function buildDeleteUserSql(username: string): string {
  return `DROP USER '${escapeSqlLiteral(username)}'`;
}
