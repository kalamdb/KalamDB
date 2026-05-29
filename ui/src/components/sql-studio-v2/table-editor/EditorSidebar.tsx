import { useMemo, useState } from "react";
import { Plus, Search, Table2, FolderPlus, Trash2, MoreHorizontal, Upload } from "lucide-react";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { TooltipProvider } from "@/components/ui/tooltip";
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
import { StudioChromeLabel, StudioIconButton } from "../shared/StudioChrome";
import {
  startTableImport,
  getTableImportStatus,
  type TableTransferInput,
} from "@/services/tableTransferService";
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
    // Import table dialog state
    const [showImportDialog, setShowImportDialog] = useState(false);
    const [importNamespace, setImportNamespace] = useState<string>("");
    const [importTableName, setImportTableName] = useState("");
    const [importTableType, setImportTableType] = useState<"shared" | "user">("shared");
    const [importUserId, setImportUserId] = useState("");
    const [importFile, setImportFile] = useState<File | null>(null);
    const [isImporting, setIsImporting] = useState(false);

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

  const openImportDialog = () => {
    setImportNamespace(activeNamespace);
    setImportTableName("");
    setImportTableType("shared");
    setImportUserId("");
    setImportFile(null);
    setShowImportDialog(true);
  };

  const handleImportSubmit = async () => {
    if (!importFile || !importTableName.trim() || !importNamespace.trim()) return;
    setIsImporting(true);
    try {
      const input: TableTransferInput = {
        namespace_id: importNamespace,
        table_name: importTableName.trim(),
        table_type: importTableType,
        ...(importTableType === "user" && importUserId.trim()
          ? { user_id: importUserId.trim() }
          : {}),
      };
      const job = await startTableImport(input, importFile);
      // Poll until terminal
      const pollInterval = 2000;
      const timeout = 300_000;
      const deadline = Date.now() + timeout;
      let done = false;
      while (!done && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, pollInterval));
        const status = await getTableImportStatus(job.job_id);
        if (status.status === "completed") {
          done = true;
          notify({
            title: `Table "${importNamespace}.${importTableName.trim()}" imported successfully`,
            variant: "success",
          });
          setShowImportDialog(false);
          onSchemaRefresh?.();
        } else if (status.status === "failed" || status.status === "cancelled") {
          throw new Error(status.message ?? `Import ${status.status}`);
        }
      }
      if (!done) throw new Error("Import timed out");
    } catch (err) {
      notify({
        title: "Import failed",
        description: err instanceof Error ? err.message : String(err),
        variant: "destructive",
      });
    } finally {
      setIsImporting(false);
    }
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
          <StudioChromeLabel>Namespace</StudioChromeLabel>
          <div className="flex items-center gap-0.5">
            <StudioIconButton
              onClick={() => setShowCreateNamespace(true)}
              tooltip="Create namespace"
              aria-label="Create namespace"
            >
              <FolderPlus data-icon="only" />
            </StudioIconButton>
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
                <StudioIconButton
                  onClick={handleDropNamespaceClick}
                  disabled={disabled}
                  tone="destructive"
                  tooltip={tooltipLabel}
                  aria-label="Drop namespace"
                  className={cn(disabled && "cursor-not-allowed opacity-30")}
                >
                  <Trash2 data-icon="only" />
                </StudioIconButton>
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
          <StudioChromeLabel>Tables ({tablesInNamespace.length})</StudioChromeLabel>
          {!activeNamespaceIsReadOnly && (
            <StudioIconButton
              onClick={handleCreate}
              tooltip="New table"
              aria-label="New table"
            >
              <Plus data-icon="only" />
            </StudioIconButton>
          )}
            {!activeNamespaceIsReadOnly && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <StudioIconButton
                    tooltip="Table actions"
                    aria-label="Table actions"
                  >
                    <MoreHorizontal data-icon="only" />
                  </StudioIconButton>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="text-xs">
                  <DropdownMenuItem
                    className="gap-2 text-xs"
                    onSelect={openImportDialog}
                  >
                    <Upload className="h-3.5 w-3.5" />
                    Import table…
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
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
                  <div className="absolute right-1.5 top-1/2 -translate-y-1/2">
                    <StudioIconButton
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDropTable(table);
                      }}
                      tone="destructive"
                      tooltip="Drop table"
                      aria-label={`Drop table ${table.name}`}
                    >
                      <Trash2 data-icon="only" />
                    </StudioIconButton>
                  </div>
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

      {/* Import Table Dialog */}
      <Dialog open={showImportDialog} onOpenChange={(open) => !isImporting && setShowImportDialog(open)}>
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>Import table</DialogTitle>
          </DialogHeader>
          <div className="flex flex-col gap-3 py-1">
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">Namespace</label>
              <Select value={importNamespace} onValueChange={setImportNamespace}>
                <SelectTrigger className="h-8 text-xs">
                  <SelectValue placeholder="Select namespace" />
                </SelectTrigger>
                <SelectContent>
                  {namespaces
                    .filter((ns) => !isReadOnlyNamespace(ns))
                    .map((ns) => (
                      <SelectItem key={ns} value={ns} className="text-xs">
                        {ns}
                      </SelectItem>
                    ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">Table name</label>
              <Input
                value={importTableName}
                onChange={(e) => setImportTableName(e.target.value)}
                placeholder="my_table"
                className="h-8 text-xs"
              />
            </div>
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">Table type</label>
              <Select
                value={importTableType}
                onValueChange={(v) => setImportTableType(v as "shared" | "user")}
              >
                <SelectTrigger className="h-8 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="shared" className="text-xs">Shared</SelectItem>
                  <SelectItem value="user" className="text-xs">User</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {importTableType === "user" && (
              <div className="flex flex-col gap-1">
                <label className="text-xs font-medium text-muted-foreground">User ID (optional)</label>
                <Input
                  value={importUserId}
                  onChange={(e) => setImportUserId(e.target.value)}
                  placeholder="user-uuid"
                  className="h-8 text-xs"
                />
              </div>
            )}
            <div className="flex flex-col gap-1">
              <label className="text-xs font-medium text-muted-foreground">ZIP archive</label>
              <input
                type="file"
                accept=".zip"
                className="text-xs file:mr-2 file:rounded file:border-0 file:bg-muted file:px-2 file:py-1 file:text-xs file:font-medium"
                onChange={(e) => setImportFile(e.target.files?.[0] ?? null)}
              />
            </div>
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setShowImportDialog(false)}
              disabled={isImporting}
            >
              Cancel
            </Button>
            <Button
              type="button"
              size="sm"
              onClick={() => void handleImportSubmit()}
              disabled={isImporting || !importFile || !importTableName.trim() || !importNamespace.trim()}
            >
              {isImporting ? "Importing…" : "Import"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

    </div>
    </TooltipProvider>
  );
}
