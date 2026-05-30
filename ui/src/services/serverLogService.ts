import { getDb } from "@/lib/db";
import type { SystemServerLogRow } from "@/lib/models";
import { system_server_logs } from "@/lib/schema";
import { eq, like, desc, and, lt, gt, type SQL } from "drizzle-orm";

export type ServerLog = SystemServerLogRow;

export interface ServerLogFilters {
  level?: string;
  target?: string;
  message?: string;
  limit?: number;
  beforeTimestamp?: string;
  afterTimestamp?: string;
}

export async function fetchServerLogs(filters?: ServerLogFilters) {
  const db = getDb();
  const conditions: SQL[] = [];

  if (filters?.level) {
    conditions.push(eq(system_server_logs.level, filters.level));
  }
  if (filters?.target) {
    conditions.push(like(system_server_logs.target, `%${filters.target}%`));
  }
  if (filters?.message) {
    conditions.push(like(system_server_logs.message, `%${filters.message}%`));
  }
  if (filters?.beforeTimestamp) {
    conditions.push(lt(system_server_logs.timestamp, filters.beforeTimestamp));
  }
  if (filters?.afterTimestamp) {
    conditions.push(gt(system_server_logs.timestamp, filters.afterTimestamp));
  }

  return db
    .select()
    .from(system_server_logs)
    .where(conditions.length > 0 ? and(...conditions) : undefined)
    .orderBy(desc(system_server_logs.timestamp))
    .limit(filters?.limit ?? 500);
}
