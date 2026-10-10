import type {
  QueryLogEntry,
  QueryResultData,
  QueryTab,
  SqlStudioResultView,
} from "@/components/sql-studio-v2/shared/types";
import type { SqlStudioPersistedQueryTab } from "@/components/sql-studio-v2/shared/workspaceState";

export const DEFAULT_SQL = "";

const AUTO_SELECT_LIMIT_SQL_PATTERN = /^\s*(SELECT\s+\*\s+FROM\s+(?:"[^"]+"|[A-Za-z_][\w$]*)\.(?:"[^"]+"|[A-Za-z_][\w$]*))\s+LIMIT\s+100\s*;\s*$/i;

export function createSavedQueryId(): string {
  return `saved-${Date.now()}-${Math.floor(Math.random() * 1000)}`;
}

export function createTabId(index: number): string {
  return `tab-${Date.now()}-${index}`;
}

export function createLogEntry(
  message: string,
  level: QueryLogEntry["level"] = "info",
  asUser?: string,
  response?: unknown,
): QueryLogEntry {
  const createdAt = new Date().toISOString();
  return {
    id: `${createdAt}-${Math.floor(Math.random() * 1000)}`,
    message,
    level,
    response,
    asUser,
    createdAt,
  };
}

export function resolveResultView(result: QueryResultData): SqlStudioResultView {
  const hasTableData = result.status === "success" && result.schema.length > 0;
  if (hasTableData) {
    return "results";
  }
  return result.logs.length > 0 ? "log" : "results";
}

export function createQueryTab(index: number): QueryTab {
  return {
    id: createTabId(index),
    title: index === 1 ? "Untitled query" : `Query ${index}`,
    sql: DEFAULT_SQL,
    isDirty: false,
    unreadChangeCount: 0,
    isLive: false,
    liveStatus: "idle",
    resultView: "results",
    lastSavedAt: null,
    savedQueryId: null,
    subscriptionOptions: undefined,
  };
}

export function toPersistedTab(tab: QueryTab): SqlStudioPersistedQueryTab {
  return {
    id: tab.id,
    name: tab.title,
    query: tab.sql,
    settings: {
      isDirty: tab.isDirty,
      isLive: tab.isLive,
      liveStatus: tab.liveStatus === "connected" ? "idle" : tab.liveStatus,
      resultView: tab.resultView,
      lastSavedAt: tab.lastSavedAt,
      savedQueryId: tab.savedQueryId,
      subscriptionOptions: tab.subscriptionOptions,
    },
  };
}

export function buildSelectFromTableSql(
  namespace: string,
  tableName: string,
  withLimit: boolean,
): string {
  const suffix = withLimit ? " LIMIT 100;" : ";";
  return `SELECT * FROM ${namespace}.${tableName}${suffix}`;
}

export function stripAutoSelectLimitForLiveSql(sql: string): string {
  const match = sql.match(AUTO_SELECT_LIMIT_SQL_PATTERN);
  if (!match) {
    return sql;
  }

  return `${match[1]};`;
}

export function prepareLiveSubscriptionSql(sql: string): { sql: string; notice: string | null } {
  const limited = stripAutoSelectLimitForLiveSql(sql).trim().replace(/;+\s*$/, "");
  const orderByAt = indexOfTopLevelKeyword(limited, "order by");
  if (orderByAt < 0) {
    return { sql: limited, notice: null };
  }

  return {
    sql: limited.slice(0, orderByAt).trim(),
    notice:
      "Live queries cannot use ORDER BY, so Kalam left it out of this subscription. Rows still update live. Sort the grid if you need an order.",
  };
}

function indexOfTopLevelKeyword(sql: string, keyword: string): number {
  const target = keyword.toLowerCase();
  const lower = sql.toLowerCase();
  let depth = 0;
  let quote: "'" | '"' | null = null;

  for (let index = 0; index < sql.length; index += 1) {
    const char = sql[index];
    if (quote) {
      if (char === quote && sql[index - 1] !== "\\") {
        quote = null;
      }
      continue;
    }
    if (char === "'" || char === '"') {
      quote = char;
      continue;
    }
    if (char === "(") {
      depth += 1;
      continue;
    }
    if (char === ")" && depth > 0) {
      depth -= 1;
      continue;
    }
    if (depth !== 0 || !lower.startsWith(target, index)) {
      continue;
    }
    const before = index === 0 ? " " : sql[index - 1];
    const after = sql[index + target.length] ?? " ";
    if (!/\w/.test(before) && !/\w/.test(after)) {
      return index;
    }
  }

  return -1;
}
