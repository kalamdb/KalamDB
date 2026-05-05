import { useMemo, useState } from "react";
import { Plus, Search, Table2, FolderPlus, Trash2 } from "lucide-react";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { useToast } from "@/components/ui/toaster-provider";
import {
  selectEditorMode,
  selectEditorSelectedTableKey,
  selectEditorDraft,
  selectEditorOriginal,
} from "@/features/sql-studio/state/selectors";
import { startCreateTable, startEditTable } from "@/features/sql-studio/state/editorTabSlice";
import { emptyDraft, isReadOnlyNamespace, tableToDraft } from "./types";
import { CreateNamespaceDialog } from "./CreateNamespaceDialog";
import { DropNamespaceDialog } from "./DropNamespaceDialog";
import { ConfirmDialog } from "./ConfirmDialog";
import { executeSqlPreviewStatement } from "./run-sql";
import equal from "fast-deep-equal";
import { generateDropTableSql } from "./ddl-generator";
import { discardEdit } from "@/features/sql-studio/state/editorTabSlice";
import { useSqlPreview } from "@/components/sql-preview";
import type { StudioNamespace, StudioTable } from "@/components/sql-studio-v2/shared/types";

interface EditorSidebarProps {
  schema: StudioNamespace[];
  defaultNamespace?: string;
  onSchemaRefresh?: () => void;
}

export function EditorSidebar({ schema, defaultNamespace = "default", onSchemaRefresh }: EditorSidebarProps) {
  const dispatch = useAppDispatch();
  const mode = useAppSelector(selectEditorMode);
  const selectedKey = useAppSelector(selectEditorSelectedTableKey);
  const draft = useAppSelector(selectEditorDraft);
  const original = useAppSelector(selectEditorOriginal);
  const [filter, setFilter] = useState("");
  const [showCreateNamespace, setShowCreateNamespace] = useState(false);
  const [showDropNamespace, setShowDropNamespace] = useState(false);
  const [pendingDiscardAction, setPendingDiscardAction] = useState<(() => void) | null>(null);
  const { notify } = useToast();
  const { openSqlPreview } = useSqlPreview();

  const isDirty = (() => {
    if (mode === "idle" || !draft || !original) return false;
    return !equal(draft, original);
  })();

  const guardDirty = (action: () => void) => {
    if (!isDirty) {
      action();
      return;
    }
    setPendingDiscardAction(() => action);
  };

  const namespaces = useMemo(() => {
    const names = schema.map((ns) => ns.name);
    return Array.from(new Set(names)).sort((a, b) => {
      const sysA = isReadOnlyNamespace(a);
      const sysB = isReadOnlyNamespace(b);
      if (sysA !== sysB) return sysA ? 1 : -1;
      return a.localeCompare(b);
    });
  }, [schema]);

  const [activeNamespace, setActiveNamespace] = useState<string>(() => {
    if (namespaces.includes(defaultNamespace)) return defaultNamespace;
    return namespaces[0] ?? defaultNamespace;
  });

  const activeNamespaceIsReadOnly = isReadOnlyNamespace(activeNamespace);

  const tablesInNamespace = useMemo(() => {
    const ns = schema.find((n) => n.name === activeNamespace);
    if (!ns) return [];
    const lower = filter.trim().toLowerCase();
    const list = lower
      ? ns.tables.filter((t) => t.name.toLowerCase().includes(lower))
      : ns.tables;
    return [...list].sort((a, b) => a.name.localeCompare(b.name));
  }, [schema, activeNamespace, filter]);

  const handleCreate = () => {
    guardDirty(() => {
      dispatch(
        startCreateTable({
          namespace: activeNamespace,
          emptyDraft: emptyDraft(activeNamespace),
        }),
      );
    });
  };

  const handleNamespaceChange = (value: string) => {
    setActiveNamespace(value);
  };

  const handleDropNamespaceClick = () => {
    if (activeNamespace.startsWith("system") || activeNamespace.startsWith("dba")) return;
    setShowDropNamespace(true);
  };

  const handleDropNamespaceConfirm = (cascade: boolean) => {
    setShowDropNamespace(false);
    const droppedName = activeNamespace;
    const sql = `DROP NAMESPACE ${droppedName}${cascade ? " CASCADE" : ""};`;
    openSqlPreview({
      sql,
      title: `Drop namespace "${droppedName}"`,
      description: cascade
        ? "CASCADE will drop the namespace and all of its tables."
        : "This will drop the namespace.",
      onExecute: executeSqlPreviewStatement,
      onComplete: () => {
        notify({ title: `Dropped namespace "${droppedName}"`, variant: "success" });
        const remaining = namespaces.filter((n) => n !== droppedName);
        const next = remaining.includes("default") ? "default" : remaining[0] ?? "default";
        setActiveNamespace(next);
        onSchemaRefresh?.();
      },
    });
  };

  const handleCreateNamespaceSubmit = (raw: string) => {
    setShowCreateNamespace(false);
    openSqlPreview({
      sql: `CREATE NAMESPACE ${raw};`,
      title: `Create namespace "${raw}"`,
      onExecute: executeSqlPreviewStatement,
      onComplete: () => {
        notify({ title: `Created namespace "${raw}"`, variant: "success" });
        setActiveNamespace(raw);
        onSchemaRefresh?.();
      },
    });
  };

  const handleDropTable = (table: StudioTable) => {
    const sql = generateDropTableSql(table.namespace, table.name);
    const fqn = `${table.namespace}.${table.name}`;
    openSqlPreview({
      sql,
      title: `Drop ${fqn}`,
      description: "This will permanently delete the table and all of its data.",
      onExecute: executeSqlPreviewStatement,
      onComplete: () => {
        notify({ title: `Dropped ${fqn}`, variant: "success" });
        if (selectedKey === fqn) dispatch(discardEdit());
        onSchemaRefresh?.();
      },
    });
  };

  const handleSelectTable = (table: StudioTable) => {
    const key = `${table.namespace}.${table.name}`;
    if (selectedKey === key && mode === "edit") return;
    guardDirty(() => {
      dispatch(
        startEditTable({
          tableKey: key,
          draft: tableToDraft(table),
        }),
      );
    });
  };

  return (
    <TooltipProvider delayDuration={250}>
    <div className="flex h-full min-h-0 flex-col">
      <div className="shrink-0 space-y-2 border-b border-border px-2 py-2">
        <div className="flex items-center justify-between">
          <label className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            Namespace
          </label>
          <div className="flex items-center gap-0.5">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={() => setShowCreateNamespace(true)}
                  className="rounded p-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
                >
                  <FolderPlus className="h-3 w-3" />
                </button>
              </TooltipTrigger>
              <TooltipContent>Create namespace</TooltipContent>
            </Tooltip>
            {(() => {
              const isSystem = activeNamespace.startsWith("system") || activeNamespace.startsWith("dba");
              const noNamespaces = namespaces.length === 0;
              const disabled = isSystem || noNamespaces;
              const tooltipLabel = noNamespaces
                ? "No namespace to drop"
                : isSystem
                  ? "System namespace — cannot drop"
                  : `Drop namespace "${activeNamespace}"`;
              return (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="block">
                      <button
                        type="button"
                        onClick={handleDropNamespaceClick}
                        disabled={disabled}
                        className={cn(
                          "rounded p-0.5 text-muted-foreground transition-colors",
                          !disabled && "hover:bg-destructive/10 hover:text-destructive",
                          disabled && "cursor-not-allowed opacity-30",
                        )}
                      >
                        <Trash2 className="h-3 w-3" />
                      </button>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>{tooltipLabel}</TooltipContent>
                </Tooltip>
              );
            })()}
          </div>
        </div>
        {namespaces.length > 0 ? (
          <Select value={activeNamespace} onValueChange={handleNamespaceChange}>
            <SelectTrigger className="h-8 w-full text-xs">
              <SelectValue placeholder="Select namespace" />
            </SelectTrigger>
            <SelectContent>
              {namespaces.map((ns) => (
                <SelectItem key={ns} value={ns} className="text-xs">
                  {ns}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <button
            type="button"
            onClick={() => setShowCreateNamespace(true)}
            className="flex h-8 w-full items-center justify-center gap-1.5 rounded-md border border-dashed border-border text-xs text-muted-foreground hover:border-foreground/40 hover:text-foreground"
          >
            <FolderPlus className="h-3.5 w-3.5" />
            No namespaces — create one
          </button>
        )}
        <div className="relative">
          <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter tables…"
            className="h-8 pl-7 text-xs"
          />
        </div>
      </div>

      {namespaces.length > 0 && (
        <div className="flex shrink-0 items-center justify-between border-b border-border px-3 py-1.5">
          <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
            Tables ({tablesInNamespace.length})
          </span>
          {!activeNamespaceIsReadOnly && (
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={handleCreate}
                  className="rounded p-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
                  aria-label="New table"
                >
                  <Plus className="h-3 w-3" />
                </button>
              </TooltipTrigger>
              <TooltipContent>New table</TooltipContent>
            </Tooltip>
          )}
        </div>
      )}

      <ScrollArea className="min-h-0 flex-1">
        {tablesInNamespace.length === 0 && !filter && !activeNamespaceIsReadOnly && (
          <div className="flex flex-col items-center gap-3 px-4 py-8 text-center">
            <p className="text-xs text-muted-foreground">
              No tables in <span className="font-mono">{activeNamespace}</span> yet.
            </p>
            <Button type="button" size="sm" onClick={handleCreate} className="gap-1.5">
              <Plus className="h-3.5 w-3.5" />
              Create your first table
            </Button>
          </div>
        )}
        {tablesInNamespace.length === 0 && !filter && activeNamespaceIsReadOnly && (
          <div className="px-3 py-6 text-center text-xs text-muted-foreground">
            No tables in this namespace.
          </div>
        )}
        {tablesInNamespace.length === 0 && filter && (
          <div className="px-3 py-6 text-center text-xs text-muted-foreground">
            No tables match the filter.
          </div>
        )}
        <ul className="flex flex-col py-1">
          {tablesInNamespace.map((table) => {
            const key = `${table.namespace}.${table.name}`;
            const isActive = mode === "edit" && selectedKey === key;
            return (
              <li key={key} className="group relative">
                <button
                  type="button"
                  onClick={() => handleSelectTable(table)}
                  className={cn(
                    "flex w-full items-center gap-2 px-3 py-1.5 pr-9 text-left text-xs transition-colors",
                    "hover:bg-accent hover:text-accent-foreground",
                    isActive && "bg-accent text-accent-foreground font-medium",
                  )}
                >
                  <Table2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate">{table.name}</span>
                </button>
                {!activeNamespaceIsReadOnly && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDropTable(table);
                        }}
                        className={cn(
                          "absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-1 text-muted-foreground/60 transition-colors",
                          "hover:bg-destructive/10 hover:text-destructive",
                        )}
                        aria-label={`Drop table ${table.name}`}
                      >
                        <Trash2 className="h-3 w-3" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent>Drop table</TooltipContent>
                  </Tooltip>
                )}
              </li>
            );
          })}
        </ul>
      </ScrollArea>

      <CreateNamespaceDialog
        open={showCreateNamespace}
        existingNames={namespaces}
        onSubmit={handleCreateNamespaceSubmit}
        onClose={() => setShowCreateNamespace(false)}
      />

      <DropNamespaceDialog
        open={showDropNamespace}
        namespace={activeNamespace}
        tableCount={tablesInNamespace.length}
        onSubmit={(cascade) => void handleDropNamespaceConfirm(cascade)}
        onClose={() => setShowDropNamespace(false)}
      />

      <ConfirmDialog
        open={pendingDiscardAction !== null}
        title="Discard unsaved changes?"
        description="You have unsaved edits. Switching will lose them."
        confirmLabel="Discard & continue"
        variant="destructive"
        onConfirm={() => {
          pendingDiscardAction?.();
          setPendingDiscardAction(null);
        }}
        onClose={() => setPendingDiscardAction(null)}
      />

    </div>
    </TooltipProvider>
  );
}
