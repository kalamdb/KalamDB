import { compileQuery } from "@kalamdb/orm";
import { getDb, sqlWithInlineParams } from "@/lib/db";
import type { SystemServerLogRow } from "@/lib/models";
import { system_server_logs } from "@/lib/schema";
import { eq, like, desc, and, lt, gt, sql, type SQL } from "drizzle-orm";

export type ServerLog = SystemServerLogRow;

export interface ServerLogFilters {
  level?: string;
  target?: string;
  message?: string;
  caseSensitive?: boolean;
  limit?: number;
  beforeTimestamp?: string;
  afterTimestamp?: string;
}

function serverLogsQuery(filters?: ServerLogFilters) {
  const db = getDb();
  const conditions: SQL[] = [];

  if (filters?.level) {
    conditions.push(eq(system_server_logs.level, filters.level));
  }
  if (filters?.target) {
    conditions.push(like(system_server_logs.target, `%${filters.target}%`));
  }
  if (filters?.message) {
    const pattern = `%${filters.message}%`;
    if (filters.caseSensitive === false) {
      conditions.push(sql`LOWER(${system_server_logs.message}) LIKE LOWER(${pattern})`);
    } else {
      conditions.push(like(system_server_logs.message, pattern));
    }
  }
  if (filters?.beforeTimestamp) {
    conditions.push(lt(system_server_logs.timestamp, filters.beforeTimestamp));
  }
  if (filters?.afterTimestamp) {
    conditions.push(gt(system_server_logs.timestamp, filters.afterTimestamp));
  }

  return db
    .select({
      timestamp: system_server_logs.timestamp,
      level: system_server_logs.level,
      thread: system_server_logs.thread,
      target: system_server_logs.target,
      line: system_server_logs.line,
      message: system_server_logs.message,
    })
    .from(system_server_logs)
    .where(conditions.length > 0 ? and(...conditions) : undefined)
    .orderBy(desc(system_server_logs.timestamp))
    .limit(filters?.limit ?? 200);
}

export async function fetchServerLogs(filters?: ServerLogFilters) {
  return serverLogsQuery(filters);
}

export function compileServerLogsSql(filters?: ServerLogFilters): string {
  const compiled = compileQuery(serverLogsQuery(filters));
  return sqlWithInlineParams(compiled.sql, compiled.params);
}
