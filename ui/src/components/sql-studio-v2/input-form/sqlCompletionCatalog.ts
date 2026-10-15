import type { StudioNamespace } from "../shared/types";
import type { ProcedureMetadata } from "@/features/functions/types";
import { procedureCompletionEntries } from "@/features/functions/sqlCompletions";
import type { StaticSqlCompletions } from "./sqlCompletionStatic";

type CompletionCategory = "function" | "keyword" | "operator" | "snippet" | "type";

export interface SqlCompletionEntry {
  label: string;
  detail: string;
  category: CompletionCategory;
  insertText?: string;
  isSnippet?: boolean;
  sortText?: string;
}

export interface SqlCompletionData {
  namespaces: string[];
  tablesByNamespace: Record<string, string[]>;
  columnsByTable: Record<string, string[]>;
  procedures: ProcedureMetadata[];
  procedureEntries: readonly SqlCompletionEntry[];
}

export type SqlSuggestionKind = CompletionCategory | "schema" | "table" | "column";

export interface SqlCompletionSuggestion {
  label: string;
  detail: string;
  insertText: string;
  isSnippet: boolean;
  sortText?: string;
  kind: SqlSuggestionKind;
}

export interface SqlContextualCompletionMatch {
  kind: "table" | "column";
  labels: string[];
  detail: string;
  partial: string;
}

const STATIC_SYSTEM_TABLES = [
  {
    namespace: "system",
    name: "live",
    columns: [
      "live_id",
      "connection_id",
      "subscription_id",
      "namespace_id",
      "table_name",
      "user_id",
      "query",
      "options",
      "status",
      "created_at",
      "last_update",
      "changes",
      "node_id",
      "last_ping_at",
    ],
  },
] as const;


const EMPTY_COMPLETION_DATA: SqlCompletionData = {
  namespaces: [],
  tablesByNamespace: {},
  columnsByTable: {},
  procedures: [],
  procedureEntries: [],
};

let staticSqlCompletionsPromise: Promise<StaticSqlCompletions> | null = null;

export function loadStaticSqlCompletions(): Promise<StaticSqlCompletions> {
  staticSqlCompletionsPromise ??= import("./sqlCompletionStatic").then(
    (module) => module.staticSqlCompletions,
  );
  return staticSqlCompletionsPromise;
}

function pushUnique(values: string[], value: string) {
  if (!values.some((item) => item.toLowerCase() === value.toLowerCase())) {
    values.push(value);
  }
}

function addTable(
  data: Pick<SqlCompletionData, "columnsByTable" | "namespaces" | "tablesByNamespace">,
  namespaceName: string,
  tableName: string,
  columns: readonly string[],
) {
  const namespaceKey = namespaceName.toLowerCase();
  const tableKey = tableName.toLowerCase();
  const qualifiedTable = `${namespaceKey}.${tableKey}`;

  pushUnique(data.namespaces, namespaceName);
  if (!data.tablesByNamespace[namespaceKey]) {
    data.tablesByNamespace[namespaceKey] = [];
  }
  pushUnique(data.tablesByNamespace[namespaceKey], tableName);

  if (!data.columnsByTable[qualifiedTable]) {
    data.columnsByTable[qualifiedTable] = [];
  }
  columns.forEach((column) => pushUnique(data.columnsByTable[qualifiedTable], column));
}

export function buildSqlCompletionData(
  schema: StudioNamespace[],
  procedures: ProcedureMetadata[] = [],
): SqlCompletionData {
  const data: SqlCompletionData = {
    ...EMPTY_COMPLETION_DATA,
    namespaces: [],
    tablesByNamespace: {},
    columnsByTable: {},
    procedures,
    procedureEntries: procedureCompletionEntries(procedures),
  };

  schema.forEach((namespace) => {
    namespace.tables.forEach((table) => {
      addTable(data, namespace.name, table.name, table.columns.map((column) => column.name));
    });
  });

  STATIC_SYSTEM_TABLES.forEach((table) => {
    addTable(data, table.namespace, table.name, table.columns);
  });

  return data;
}

export function resolveSqlContextualCompletions(
  data: Pick<SqlCompletionData, "columnsByTable" | "tablesByNamespace">,
  textUntilPosition: string,
  aliasToTable: Record<string, string>,
): SqlContextualCompletionMatch | null {
  const tableColumnMatch = /([a-zA-Z_][\w]*)\.([a-zA-Z_][\w]*)\.([a-zA-Z_][\w]*)?$/.exec(textUntilPosition);
  if (tableColumnMatch) {
    const namespaceKey = tableColumnMatch[1].toLowerCase();
    const tableKey = tableColumnMatch[2].toLowerCase();
    const columns = data.columnsByTable[`${namespaceKey}.${tableKey}`] ?? [];
    return {
      kind: "column",
      labels: columns,
      detail: `${namespaceKey}.${tableKey} column`,
      partial: tableColumnMatch[3]?.toLowerCase() ?? "",
    };
  }

  const identifierMatch = /([a-zA-Z_][\w]*)\.([a-zA-Z_][\w]*)?$/.exec(textUntilPosition);
  if (!identifierMatch) {
    return null;
  }

  const qualifierKey = identifierMatch[1].toLowerCase();
  const partial = identifierMatch[2]?.toLowerCase() ?? "";
  const resolvedTable = aliasToTable[qualifierKey];
  if (resolvedTable) {
    return {
      kind: "column",
      labels: data.columnsByTable[resolvedTable] ?? [],
      detail: `${qualifierKey} alias column`,
      partial,
    };
  }

  const namespaceTables = data.tablesByNamespace[qualifierKey] ?? [];
  if (namespaceTables.length > 0) {
    return {
      kind: "table",
      labels: namespaceTables,
      detail: `${qualifierKey} table`,
      partial,
    };
  }

  return null;
}

function matchesPrefix(label: string, prefix: string): boolean {
  return prefix.length === 0 || label.toLowerCase().includes(prefix);
}

function entrySuggestion(entry: SqlCompletionEntry): SqlCompletionSuggestion {
  return {
    label: entry.label,
    detail: entry.detail,
    insertText: entry.insertText ?? entry.label,
    isSnippet: Boolean(entry.isSnippet),
    sortText: entry.sortText,
    kind: entry.category,
  };
}

export function collectSqlCompletions(input: {
  data: Pick<SqlCompletionData, "columnsByTable" | "namespaces" | "procedureEntries" | "tablesByNamespace">;
  catalog: StaticSqlCompletions;
  textUntilPosition: string;
  prefix: string;
  aliasToTable: Record<string, string>;
}): SqlCompletionSuggestion[] {
  const contextual = resolveSqlContextualCompletions(
    input.data,
    input.textUntilPosition,
    input.aliasToTable,
  );
  if (contextual) {
    const kind = contextual.kind === "column" ? "column" : "table";
    return contextual.labels
      .filter((label) => matchesPrefix(label, contextual.partial))
      .map((label) => ({
        label,
        detail: contextual.detail,
        insertText: label,
        isSnippet: false,
        kind,
      }));
  }

  const suggestions: SqlCompletionSuggestion[] = [];
  const seen = new Set<string>();
  const push = (suggestion: SqlCompletionSuggestion) => {
    if (!matchesPrefix(suggestion.label, input.prefix)) {
      return;
    }
    const key = `${suggestion.kind}:${suggestion.label}:${suggestion.detail}`;
    if (seen.has(key)) {
      return;
    }
    seen.add(key);
    suggestions.push(suggestion);
  };

  input.catalog.keywords.forEach((keyword) => {
    push({
      label: keyword,
      detail: "SQL keyword",
      insertText: keyword,
      isSnippet: false,
      sortText: `2_${keyword}`,
      kind: "keyword",
    });
  });
  input.catalog.snippets.forEach((entry) => push(entrySuggestion(entry)));
  if (input.prefix.length > 0) {
    input.catalog.functions.forEach((entry) => push(entrySuggestion(entry)));
    input.data.procedureEntries.forEach((entry) => push(entrySuggestion(entry)));
    input.catalog.types.forEach((entry) => push(entrySuggestion(entry)));
    input.catalog.operators.forEach((entry) => push(entrySuggestion(entry)));
  }
  input.data.namespaces.forEach((namespaceName) => {
    push({
      label: namespaceName,
      detail: "Schema",
      insertText: namespaceName,
      isSnippet: false,
      kind: "schema",
    });
  });
  Object.entries(input.data.tablesByNamespace).forEach(([namespaceName, tables]) => {
    tables.forEach((table) => {
      const qualified = `${namespaceName}.${table}`;
      push({
        label: qualified,
        detail: "Qualified table name",
        insertText: qualified,
        isSnippet: false,
        kind: "table",
      });
      push({
        label: table,
        detail: `Table in ${namespaceName}`,
        insertText: table,
        isSnippet: false,
        kind: "table",
      });
    });
  });

  return suggestions;
}