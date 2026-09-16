import { executeSql } from "@/lib/api";

export interface ScheduleRow {
  schedule_id: string;
  namespace_id: string;
  name: string;
  routine_id: string;
  principal_user_id: string;
  cron: string | null;
  interval_ms: number | null;
  timezone: string;
  enabled: boolean;
  next_run_at: number;
  running_until: number | null;
  owner: string | null;
  last_started_at: number | null;
  last_finished_at: number | null;
  last_error: string | null;
  run_count: number;
  skip_count: number;
}
export type ScheduleFilter = "all" | "enabled" | "disabled" | "running";
export const SCHEDULE_PAGE_SIZE = 50;

export function scheduleSqlName(row: Pick<ScheduleRow, "namespace_id" | "name">): string {
  const quote = (value: string) => `"${value.replace(/"/g, '""')}"`;
  return `${quote(row.namespace_id)}.${quote(row.name)}`;
}
export function scheduleStatus(row: Pick<ScheduleRow, "enabled" | "running_until" | "last_error">, now = Date.now()): string {
  if (row.running_until != null) return row.running_until > now ? "Running" : "Recovering";
  if (!row.enabled) return "Disabled";
  return row.last_error ? "Failed" : "Scheduled";
}
export async function fetchSchedules(page: number, filter: ScheduleFilter): Promise<{ rows: ScheduleRow[]; hasMore: boolean }> {
  const clauses = {
    all: "",
    enabled: "WHERE enabled = true",
    disabled: "WHERE enabled = false",
    running: "WHERE running_until IS NOT NULL",
  } as const;
  const offset = Math.max(0, Math.floor(page)) * SCHEDULE_PAGE_SIZE;
  const result = await executeSql(
    `SELECT schedule_id, namespace_id, name, routine_id, principal_user_id, cron, interval_ms, timezone, enabled, next_run_at, running_until, owner, last_started_at, last_finished_at, last_error, run_count, skip_count FROM system.schedules ${clauses[filter]} ORDER BY schedule_id LIMIT ${SCHEDULE_PAGE_SIZE + 1} OFFSET ${offset}`,
  );
  const rows = result.rows.map((values) => Object.fromEntries(result.schema.map((field, index) => [field.name, values[index]?.toJson() ?? null])) as unknown as ScheduleRow);
  return { rows: rows.slice(0, SCHEDULE_PAGE_SIZE), hasMore: rows.length > SCHEDULE_PAGE_SIZE };
}
export async function setScheduleEnabled(row: ScheduleRow, enabled: boolean): Promise<void> {
  await executeSql(`ALTER SCHEDULE ${scheduleSqlName(row)} ${enabled ? "ENABLE" : "DISABLE"}`);
}
export async function dropSchedule(row: ScheduleRow): Promise<void> {
  await executeSql(`DROP SCHEDULE IF EXISTS ${scheduleSqlName(row)}`);
}
