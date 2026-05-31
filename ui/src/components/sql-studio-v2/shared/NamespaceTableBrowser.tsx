import { type ReactNode, useMemo } from "react";
import {
  ChevronDown,
  ChevronRight,
  Database,
  KeyRound,
  Loader2,
  Radio,
  Search,
  Star,
  Type,
  User,
  Users,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { StudioChromeLabel } from "./StudioChrome";
import type { SavedQuery, StudioNamespace, StudioTable } from "./types";

export function namespaceNames(schema: StudioNamespace[]): string[] {
  return Array.from(new Set(schema.map((namespace) => namespace.name))).sort(
    (a, b) => a.localeCompare(b),
  );
}

export function reconcileActiveNamespace({
  activeNamespace,
  namespaces,
  selectedTableKey,
  previousSelectedTableKey,
}: {
  activeNamespace: string;
  namespaces: string[];
  selectedTableKey: string | null;
  previousSelectedTableKey: string | null;
}): string {
  const selectedNamespace = selectedTableKey?.split(".")[0];
  if (
    selectedTableKey !== previousSelectedTableKey &&
    selectedNamespace &&
    namespaces.includes(selectedNamespace)
  ) {
    return selectedNamespace;
  }
  if (!activeNamespace || !namespaces.includes(activeNamespace)) {
    return namespaces[0] ?? "";
  }
  return activeNamespace;
}

export function tablesForNamespace(
  schema: StudioNamespace[],
  namespaceName: string,
  filter: string,
): StudioTable[] {
  const namespace = schema.find((item) => item.name === namespaceName);
  if (!namespace) return [];
  const normalizedFilter = filter.trim().toLowerCase();
  const tables = normalizedFilter
    ? namespace.tables.filter(
        (table) =>
          table.name.toLowerCase().includes(normalizedFilter) ||
          table.columns.some((column) =>
            column.name.toLowerCase().includes(normalizedFilter),
          ),
      )
    : namespace.tables;
  return [...tables].sort((a, b) => a.name.localeCompare(b.name));
}

export function tableTypeMeta(tableType: string): {
  icon: ReactNode;
  tooltip: string;
} {
  const normalized = tableType.toLowerCase();
  if (normalized === "stream") {
    return {
      icon: <Radio className="h-3.5 w-3.5 text-violet-400" />,
      tooltip: "Stream table",
    };
  }
  if (normalized === "shared") {
    return {
      icon: <Users className="h-3.5 w-3.5 text-cyan-400" />,
      tooltip: "Shared table",
    };
  }
  if (normalized === "system") {
    return {
      icon: <Database className="h-3.5 w-3.5 text-amber-400" />,
      tooltip: "System table",
    };
  }
  return {
    icon: <User className="h-3.5 w-3.5 text-emerald-400" />,
    tooltip: "User table",
  };
}

function columnIcon(isPrimaryKey: boolean) {
  if (isPrimaryKey) {
    return <KeyRound className="h-3 w-3 text-amber-500" />;
  }
  return <Type className="h-3 w-3 text-muted-foreground" />;
}

export function NamespaceSearchControls({
  namespaces,
  activeNamespace,
  filter,
  onNamespaceChange,
  onFilterChange,
  actions,
  emptyAction,
}: {
  namespaces: string[];
  activeNamespace: string;
  filter: string;
  onNamespaceChange: (namespace: string) => void;
  onFilterChange: (filter: string) => void;
  actions?: ReactNode;
  emptyAction?: ReactNode;
}) {
  return (
    <div className="shrink-0 space-y-2 border-b border-border px-2 py-2">
      <div className="flex items-center justify-between">
        <StudioChromeLabel>Namespace</StudioChromeLabel>
        {actions ? <div className="flex items-center gap-0.5">{actions}</div> : null}
      </div>
      {namespaces.length > 0 ? (
        <Select value={activeNamespace} onValueChange={onNamespaceChange}>
          <SelectTrigger className="h-8 w-full text-xs">
            <SelectValue placeholder="Select namespace" />
          </SelectTrigger>
          <SelectContent>
            {namespaces.map((namespace) => (
              <SelectItem
                key={namespace}
                value={namespace}
                className="text-xs"
              >
                {namespace}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : (
        emptyAction
      )}
      <div className="relative">
        <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={filter}
          onChange={(event) => onFilterChange(event.target.value)}
          placeholder="Filter tables..."
          className="h-8 pl-7 text-xs"
        />
      </div>
    </div>
  );
}

export function FavoritesTableBlock({
  savedQueries,
  expanded,
  onToggle,
  onOpenSavedQuery,
}: {
  savedQueries: SavedQuery[];
  expanded: boolean;
  onToggle: () => void;
  onOpenSavedQuery: (queryId: string) => void;
}) {
  return (
    <div className="border-b border-border px-2 py-2">
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-1.5 rounded-sm px-1 py-1.5 text-left text-xs font-semibold uppercase text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <div className="flex w-5 shrink-0 items-center justify-center">
          {expanded ? (
            <ChevronDown className="h-4 w-4" />
          ) : (
            <ChevronRight className="h-4 w-4" />
          )}
        </div>
        <Star className="h-3.5 w-3.5 text-primary" />
        <span className="ml-1 font-semibold">Favorites</span>
        <span className="ml-auto inline-flex items-center justify-center rounded bg-muted px-1.5 py-0.5 text-[9px] font-medium text-foreground">
          {savedQueries.length}
        </span>
      </button>
      {expanded && (
        <div className="mb-1 ml-2.5 mt-0.5 border-l border-border/40 pl-2">
          {savedQueries.length === 0 && (
            <p className="px-5 py-1.5 text-xs text-muted-foreground">
              No saved queries yet.
            </p>
          )}
          {savedQueries.map((savedQuery) => (
            <Tooltip key={savedQuery.id}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={() => onOpenSavedQuery(savedQuery.id)}
                  className="flex w-full items-center gap-2 rounded-sm px-2 py-1 text-left text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                >
                  <div className="w-5 shrink-0" />
                  <Star className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate">{savedQuery.title}</span>
                </button>
              </TooltipTrigger>
              <TooltipContent>{savedQuery.title}</TooltipContent>
            </Tooltip>
          ))}
        </div>
      )}
    </div>
  );
}

export function TableColumnTree({
  tables,
  activeNamespace,
  filter,
  isRefreshing,
  selectedTableKey,
  expandedTables,
  onToggleTable,
  onSelectTable,
  onTableContextMenu,
  renderTableActions,
  createEmptyState,
  readOnlyEmptyState,
}: {
  tables: StudioTable[];
  activeNamespace: string;
  filter: string;
  isRefreshing?: boolean;
  selectedTableKey: string | null;
  expandedTables: Record<string, boolean>;
  onToggleTable: (tableKey: string) => void;
  onSelectTable: (table: StudioTable) => void;
  onTableContextMenu?: (
    table: StudioTable,
    position: { x: number; y: number },
  ) => void;
  renderTableActions?: (table: StudioTable) => ReactNode;
  createEmptyState?: ReactNode;
  readOnlyEmptyState?: ReactNode;
}) {
  const hasFilter = filter.trim().length > 0;

  return (
    <TooltipProvider delayDuration={250}>
      <ScrollArea className="min-h-0 flex-1">
        {tables.length === 0 && !hasFilter && createEmptyState}
        {tables.length === 0 && !hasFilter && !createEmptyState && (
          <div className="px-3 py-6 text-center text-xs text-muted-foreground">
            {isRefreshing ? (
              <span className="inline-flex items-center gap-2">
                <Loader2 className="h-3 w-3 animate-spin" />
                Loading schema...
              </span>
            ) : (
              readOnlyEmptyState ?? "No tables in this namespace."
            )}
          </div>
        )}
        {tables.length === 0 && hasFilter && (
          <div className="px-3 py-6 text-center text-xs text-muted-foreground">
            No tables match the filter in{" "}
            <span className="font-mono">{activeNamespace}</span>.
          </div>
        )}
        <ul className="flex flex-col py-1">
          {tables.map((table) => {
            const tableKey = `${table.namespace}.${table.name}`;
            const tableOpen =
              expandedTables[tableKey] ?? tableKey === selectedTableKey;
            const isSelected = selectedTableKey === tableKey;
            const tableMeta = tableTypeMeta(table.tableType);

            return (
              <li key={tableKey}>
                <div
                  className={cn(
                    "group/table relative flex w-full items-center gap-2 rounded-sm py-1 pr-2 text-left text-xs transition-colors",
                    isSelected
                      ? "bg-sky-500/15 font-medium text-sky-400"
                      : "text-muted-foreground hover:bg-accent hover:text-foreground",
                  )}
                  onClick={() => onSelectTable(table)}
                  onContextMenu={(event) => {
                    if (!onTableContextMenu) return;
                    event.preventDefault();
                    event.stopPropagation();
                    onSelectTable(table);
                    onTableContextMenu(table, {
                      x: event.clientX,
                      y: event.clientY,
                    });
                  }}
                >
                  <button
                    type="button"
                    onClick={(event) => {
                      event.stopPropagation();
                      onToggleTable(tableKey);
                    }}
                    className="ml-2 flex w-5 shrink-0 items-center justify-center rounded hover:bg-muted"
                    aria-label={`${tableOpen ? "Collapse" : "Expand"} ${table.name}`}
                  >
                    {tableOpen ? (
                      <ChevronDown className="h-4 w-4" />
                    ) : (
                      <ChevronRight className="h-4 w-4" />
                    )}
                  </button>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <div className="shrink-0">{tableMeta.icon}</div>
                    </TooltipTrigger>
                    <TooltipContent>{tableMeta.tooltip}</TooltipContent>
                  </Tooltip>
                  <span className="min-w-0 flex-1 truncate">{table.name}</span>
                  <span className="text-[10px] tabular-nums text-muted-foreground/60">
                    {table.columns.length}
                  </span>
                  {renderTableActions ? renderTableActions(table) : null}
                </div>
                {tableOpen && (
                  <div className="space-y-0.5">
                    {table.columns.map((column) => (
                      <Tooltip key={`${tableKey}.${column.name}`}>
                        <TooltipTrigger asChild>
                          <div className="flex cursor-default items-center gap-2 rounded-sm py-0.5 pr-2 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
                            <div className="w-7 shrink-0" />
                            <div className="flex w-5 shrink-0 items-center justify-center">
                              {columnIcon(column.isPrimaryKey)}
                            </div>
                            <span className="min-w-0 flex-1 truncate">
                              {column.name}
                            </span>
                            <span className="ml-auto truncate font-mono text-[9px] lowercase opacity-70">
                              {column.dataType}
                            </span>
                          </div>
                        </TooltipTrigger>
                        <TooltipContent>
                          {column.name} ({column.dataType})
                        </TooltipContent>
                      </Tooltip>
                    ))}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      </ScrollArea>
    </TooltipProvider>
  );
}

export function useNamespaceTables({
  schema,
  activeNamespace,
  filter,
}: {
  schema: StudioNamespace[];
  activeNamespace: string;
  filter: string;
}) {
  return useMemo(
    () => tablesForNamespace(schema, activeNamespace, filter),
    [schema, activeNamespace, filter],
  );
}
