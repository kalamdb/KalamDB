import { useMemo, useState, useEffect, useRef } from "react";
import { Plus, AlertCircle, Trash2, Pencil } from "lucide-react";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toaster-provider";
import {
  selectEditorMode,
  selectEditorDraft,
  selectEditorOriginal,
  selectEditorSelectedTableKey,
} from "@/features/sql-studio/state/selectors";
import type {
  StudioNamespace,
  StudioTable,
} from "@/components/sql-studio-v2/shared/types";
import {
  discardEdit,
  setDraft,
  startEditTable,
} from "@/features/sql-studio/state/editorTabSlice";
import { ColumnRow } from "./ColumnRow";
import { ConfirmDialog } from "./ConfirmDialog";
import {
  ACCESS_LEVEL_OPTIONS,
  COMPRESSION_OPTIONS,
  EVICTION_STRATEGY_OPTIONS,
  FLUSH_POLICY_KINDS,
  TABLE_TYPE_OPTIONS,
  defaultTableOptions,
  isReadOnlyNamespace,
  newDraftColumn,
  tableToDraft,
  type DraftColumn,
  type DraftTable,
  type DraftTableOptions,
  type DraftTableType,
} from "./types";
import {
  generateAlterTableSql,
  generateCreateTableSql,
  generateDropTableSql,
  validateDraft,
} from "./ddl-generator";
import { executeSqlPreviewStatement } from "./run-sql";
import { useSqlPreview } from "@/components/sql-preview";
import {
  PanelHeader,
  chromeLabelClassName,
  fieldLabelClassName,
  sectionTitleClassName,
} from "@/components/layout/typography";
import { StudioIconButton } from "../shared/StudioChrome";
import equal from "fast-deep-equal";
import { executeSqlStudioQuery } from "@/services/sqlStudioService";
import { TableDataTransferActions } from "./TableDataTransferActions";

function MetaRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-baseline gap-2 overflow-hidden">
      <span className={`shrink-0 ${chromeLabelClassName}`}>
        {label}
      </span>
      <span
        className={`truncate ${mono ? "font-mono text-[11px]" : ""}`}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}

function titleCase(value: string): string {
  return `${value.slice(0, 1).toUpperCase()}${value.slice(1).replace(/_/g, " ")}`;
}

function SelectField({
  label,
  value,
  options,
  disabled,
  onChange,
  testId,
}: {
  label: string;
  value: string;
  options: readonly string[];
  disabled?: boolean;
  onChange: (value: string) => void;
  testId?: string;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className={fieldLabelClassName}>{label}</span>
      <Select value={value} onValueChange={onChange} disabled={disabled}>
        <SelectTrigger size="sm" className="h-8 text-xs" data-testid={testId}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option} value={option}>
              {titleCase(option)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </label>
  );
}

function TableTypeSelectField({
  value,
  disabled,
  onChange,
}: {
  value: DraftTableType;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  const selected = TABLE_TYPE_OPTIONS.find((option) => option.value === value);
  return (
    <label className="flex flex-col gap-1.5">
      <span className={fieldLabelClassName}>Table type</span>
      <Select value={value} onValueChange={onChange} disabled={disabled}>
        <SelectTrigger
          size="sm"
          className="h-10 text-xs"
          data-testid="table-type-select"
        >
          <SelectValue>
            {selected ? (
              <span className="flex min-w-0 items-center gap-2">
                <selected.icon
                  className={`h-3.5 w-3.5 shrink-0 ${selected.iconClassName}`}
                />
                <span>{selected.label}</span>
              </span>
            ) : null}
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          {TABLE_TYPE_OPTIONS.map((option) => (
            <SelectItem
              key={option.value}
              value={option.value}
              className="py-2 text-xs"
            >
              <span className="flex min-w-0 items-start gap-2">
                <option.icon
                  className={`mt-0.5 h-3.5 w-3.5 shrink-0 ${option.iconClassName}`}
                />
                <span className="min-w-0">
                  <span className="block font-medium">{option.label}</span>
                  <span className="block text-[11px] leading-snug text-muted-foreground">
                    {option.description}
                  </span>
                </span>
              </span>
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </label>
  );
}

function TableOptionsEditor({
  draft,
  disabled,
  onChange,
}: {
  draft: DraftTable;
  disabled?: boolean;
  onChange: (options: DraftTableOptions) => void;
}) {
  const options = draft.options;
  const update = (patch: Partial<DraftTableOptions>) =>
    onChange({ ...options, ...patch });
  const showFlushRows =
    options.flushPolicyKind === "rows" ||
    options.flushPolicyKind === "combined";
  const showFlushInterval =
    options.flushPolicyKind === "interval" ||
    options.flushPolicyKind === "combined";

  return (
    <section className="space-y-3">
      <h3 className={sectionTitleClassName}>Options</h3>
      <div className="rounded-md border border-border bg-muted/10 p-3">
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {(draft.tableType === "user" || draft.tableType === "shared") && (
            <label className="flex flex-col gap-1.5">
              <span className={fieldLabelClassName}>Storage ID</span>
              <Input
                value={options.storageId}
                onChange={(e) => update({ storageId: e.target.value })}
                disabled={disabled}
                className="h-8 text-xs"
                data-testid="table-option-storage-id"
              />
            </label>
          )}

          {draft.tableType === "user" && (
            <div className="flex items-center justify-between gap-3 rounded-md border border-border bg-background px-3 py-2">
              <span className={fieldLabelClassName}>Use user storage</span>
              <Switch
                size="sm"
                checked={options.useUserStorage}
                disabled={disabled}
                onCheckedChange={(checked) =>
                  update({ useUserStorage: checked })
                }
                aria-label="Use user storage"
                data-testid="table-option-use-user-storage"
              />
            </div>
          )}

          {draft.tableType === "shared" && (
            <SelectField
              label="Access level"
              value={options.accessLevel}
              options={ACCESS_LEVEL_OPTIONS}
              disabled={disabled}
              onChange={(value) =>
                update({
                  accessLevel: value as DraftTableOptions["accessLevel"],
                })
              }
              testId="table-option-access-level"
            />
          )}

          {(draft.tableType === "user" || draft.tableType === "shared") && (
            <div className="flex flex-col gap-2 rounded-md border border-border bg-background p-3 sm:col-span-2 lg:col-span-3">
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-[minmax(9rem,0.35fr)_minmax(12rem,0.65fr)]">
                <div>
                  <h4 className="text-xs font-semibold text-foreground">
                    Flush
                  </h4>
                  <p className="mt-0.5 text-[11px] leading-snug text-muted-foreground">
                    Choose when buffered rows are persisted to cold storage.
                  </p>
                </div>
                <SelectField
                  label="Policy"
                  value={options.flushPolicyKind}
                  options={FLUSH_POLICY_KINDS}
                  disabled={disabled}
                  onChange={(value) =>
                    update({
                      flushPolicyKind:
                        value as DraftTableOptions["flushPolicyKind"],
                    })
                  }
                  testId="table-option-flush-policy"
                />
              </div>
              {(showFlushRows || showFlushInterval) && (
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  {showFlushRows && (
                    <label className="flex flex-col gap-1.5">
                      <span className={fieldLabelClassName}>Rows</span>
                      <Input
                        type="number"
                        min={1}
                        max={999999}
                        value={options.flushRows}
                        onChange={(e) => update({ flushRows: e.target.value })}
                        disabled={disabled}
                        className="h-8 text-xs"
                        data-testid="table-option-flush-rows"
                      />
                    </label>
                  )}
                  {showFlushInterval && (
                    <label className="flex flex-col gap-1.5">
                      <span className={fieldLabelClassName}>
                        Interval seconds
                      </span>
                      <Input
                        type="number"
                        min={1}
                        max={86399}
                        value={options.flushIntervalSeconds}
                        onChange={(e) =>
                          update({ flushIntervalSeconds: e.target.value })
                        }
                        disabled={disabled}
                        className="h-8 text-xs"
                        data-testid="table-option-flush-interval"
                      />
                    </label>
                  )}
                </div>
              )}
            </div>
          )}

          {draft.tableType === "stream" && (
            <>
              <label className="flex flex-col gap-1.5">
                <span className={fieldLabelClassName}>TTL seconds</span>
                <Input
                  type="number"
                  min={1}
                  value={options.ttlSeconds}
                  onChange={(e) => update({ ttlSeconds: e.target.value })}
                  disabled={disabled}
                  className="h-8 text-xs"
                  data-testid="table-option-ttl-seconds"
                />
              </label>
              <SelectField
                label="Eviction strategy"
                value={options.evictionStrategy}
                options={EVICTION_STRATEGY_OPTIONS}
                disabled={disabled}
                onChange={(value) =>
                  update({
                    evictionStrategy:
                      value as DraftTableOptions["evictionStrategy"],
                  })
                }
                testId="table-option-eviction-strategy"
              />
              <label className="flex flex-col gap-1.5">
                <span className={fieldLabelClassName}>Max stream size</span>
                <Input
                  type="number"
                  min={0}
                  value={options.maxStreamSizeBytes}
                  onChange={(e) =>
                    update({ maxStreamSizeBytes: e.target.value })
                  }
                  disabled={disabled}
                  className="h-8 text-xs"
                  data-testid="table-option-max-stream-size"
                />
              </label>
            </>
          )}

          {draft.tableType !== "stream" && (
            <SelectField
              label="Compression"
              value={options.compression}
              options={COMPRESSION_OPTIONS}
              disabled={disabled}
              onChange={(value) =>
                update({
                  compression: value as DraftTableOptions["compression"],
                })
              }
              testId="table-option-compression"
            />
          )}
        </div>
      </div>
    </section>
  );
}

const YEAR_3000_MS = 3.25e13;

function formatTimestamp(value: string | number | null | undefined): string {
  if (value == null) return "—";
  let ms: number;
  if (typeof value === "string") {
    ms = Date.parse(value);
  } else {
    ms = value > YEAR_3000_MS ? value / 1000 : value;
  }
  if (!Number.isFinite(ms)) return String(value);
  return new Date(ms).toLocaleString();
}

interface EditTableFormProps {
  schema: StudioNamespace[];
  onAfterSave?: () => void | Promise<void>;
  isSchemaRefreshing?: boolean;
  onOpenQueryInNewTab?: (query: string, title: string) => void;
}

export function EditTableForm({
  schema,
  onAfterSave,
  isSchemaRefreshing,
  onOpenQueryInNewTab,
}: EditTableFormProps) {
  const dispatch = useAppDispatch();
  const mode = useAppSelector(selectEditorMode);
  const draft = useAppSelector(selectEditorDraft);
  const original = useAppSelector(selectEditorOriginal);
  const selectedTableKey = useAppSelector(selectEditorSelectedTableKey);

  const selectedTable: StudioTable | null = useMemo(() => {
    if (!selectedTableKey) return null;
    const [ns, name] = selectedTableKey.split(".");
    if (!ns || !name) return null;
    const namespace = schema.find((n) => n.name === ns);
    return namespace?.tables.find((t) => t.name === name) ?? null;
  }, [schema, selectedTableKey]);

  const [rowCount, setRowCount] = useState<number | null>(null);
  const [rowCountLoading, setRowCountLoading] = useState(false);

  useEffect(() => {
    if (!selectedTable) {
      setRowCount(null);
      return;
    }
    let cancelled = false;
    setRowCount(null);
    setRowCountLoading(true);
    void executeSqlStudioQuery(
      `SELECT COUNT(*) AS c FROM ${selectedTable.namespace}.${selectedTable.name}`,
    )
      .then((result) => {
        if (cancelled) return;
        if (result.status === "error") {
          setRowCount(null);
          return;
        }
        const raw = result.rows?.[0]?.c ?? result.rows?.[0]?.[0];
        const n = typeof raw === "number" ? raw : Number(raw);
        setRowCount(Number.isFinite(n) ? n : null);
      })
      .finally(() => {
        if (!cancelled) setRowCountLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedTable]);

  const validation = useMemo(
    () => (draft ? validateDraft(draft) : null),
    [draft],
  );
  const isDirty = useMemo(() => {
    if (!draft) return false;
    if (mode === "create") return true;
    if (!original) return false;
    return !equal(draft, original);
  }, [draft, original, mode]);
  const [showErrors, setShowErrors] = useState(false);
  const [showDiscardConfirm, setShowDiscardConfirm] = useState(false);
  const [focusColumnId, setFocusColumnId] = useState<string | null>(null);
  const [draggingColumnId, setDraggingColumnId] = useState<string | null>(null);
  const draggingColumnIdRef = useRef<string | null>(null);
  const [reloadKeyAfterRefresh, setReloadKeyAfterRefresh] = useState<
    string | null
  >(null);
  const { notify } = useToast();
  const { openSqlPreview } = useSqlPreview();
  useEffect(() => {
    setShowErrors(false);
  }, [draft?.namespace, draft?.name, mode]);

  useEffect(() => {
    if (!reloadKeyAfterRefresh) return;
    if (isSchemaRefreshing) return;
    const [ns, name] = reloadKeyAfterRefresh.split(".");
    const fresh = schema
      .find((n) => n.name === ns)
      ?.tables.find((t) => t.name === name);
    if (fresh) {
      dispatch(
        startEditTable({
          tableKey: reloadKeyAfterRefresh,
          draft: tableToDraft(fresh),
        }),
      );
    } else {
      dispatch(discardEdit());
    }
    setReloadKeyAfterRefresh(null);
  }, [reloadKeyAfterRefresh, schema, isSchemaRefreshing, dispatch]);

  if (mode === "idle" || !draft) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
        <div className="rounded-full bg-muted p-4">
          <Pencil className="h-6 w-6 text-muted-foreground" />
        </div>
        <p className={sectionTitleClassName}>No table selected</p>
        <p className="max-w-md text-sm text-muted-foreground">
          Pick a table from the sidebar to edit its schema, or use the{" "}
          <span className="font-medium">+</span> next to{" "}
          <span className="font-medium">Tables</span> in the sidebar to create
          one.
        </p>
      </div>
    );
  }

  const isCreating = mode === "create";
  const isEditing = mode === "edit";
  const isReadOnly = isReadOnlyNamespace(draft.namespace);

  const updateDraftColumn = (next: DraftColumn) => {
    dispatch(
      setDraft({
        ...draft,
        columns: draft.columns.map((c) => (c.id === next.id ? next : c)),
      }),
    );
  };

  const addColumn = () => {
    const newCol = newDraftColumn();
    setFocusColumnId(newCol.id);
    dispatch(
      setDraft({
        ...draft,
        columns: [...draft.columns, newCol],
      }),
    );
  };

  const deleteColumn = (col: DraftColumn) => {
    if (col.isNew) {
      dispatch(
        setDraft({
          ...draft,
          columns: draft.columns.filter((c) => c.id !== col.id),
        }),
      );
    } else {
      updateDraftColumn({ ...col, isDeleted: !col.isDeleted });
    }
  };

  const moveColumn = (draggedId: string, targetId: string) => {
    if (draggedId === targetId || isEditing || isReadOnly) return;
    const from = draft.columns.findIndex((column) => column.id === draggedId);
    const to = draft.columns.findIndex((column) => column.id === targetId);
    if (from < 0 || to < 0) return;
    const nextColumns = [...draft.columns];
    const [moved] = nextColumns.splice(from, 1);
    if (!moved) return;
    nextColumns.splice(to, 0, moved);
    dispatch(setDraft({ ...draft, columns: nextColumns }));
  };

  const handleTableTypeChange = (value: string) => {
    const tableType = value as DraftTableType;
    const defaults = defaultTableOptions(tableType);
    dispatch(
      setDraft({
        ...draft,
        tableType,
        options: {
          ...defaults,
          storageId: draft.options.storageId || defaults.storageId,
          compression: draft.options.compression,
        },
      }),
    );
  };

  const handleSave = () => {
    if (validation?.hasAny) {
      setShowErrors(true);
      return;
    }
    let sql: string;
    if (isCreating) {
      sql = generateCreateTableSql(draft);
    } else if (isEditing && original) {
      sql = generateAlterTableSql(original, draft);
      if (!sql.trim()) return;
    } else {
      return;
    }

    const targetKey = isCreating
      ? `${draft.namespace}.${draft.name}`
      : (selectedTableKey ?? `${draft.namespace}.${draft.name}`);

    openSqlPreview({
      sql,
      title: isCreating
        ? `Create ${draft.namespace}.${draft.name}`
        : `Alter ${draft.namespace}.${draft.name}`,
      description: isCreating
        ? "Review the CREATE TABLE statement before committing."
        : "Review the changes before committing.",
      onExecute: executeSqlPreviewStatement,
      onComplete: async () => {
        notify({
          title: isCreating
            ? `Created ${draft.namespace}.${draft.name}`
            : `Updated ${draft.namespace}.${draft.name}`,
          variant: "success",
        });
        await onAfterSave?.();
        setReloadKeyAfterRefresh(targetKey);
      },
    });
  };

  const handleDiscard = () => {
    if (isDirty) {
      setShowDiscardConfirm(true);
      return;
    }
  };

  const handleDropTable = () => {
    const sql = generateDropTableSql(draft.namespace, draft.name);
    const fqn = `${draft.namespace}.${draft.name}`;
    openSqlPreview({
      sql,
      title: `Drop ${fqn}`,
      description:
        "This will permanently delete the table and all of its data.",
      onExecute: executeSqlPreviewStatement,
      onComplete: async () => {
        notify({ title: `Dropped ${fqn}`, variant: "success" });
        dispatch(discardEdit());
        await onAfterSave?.();
      },
    });
  };

  const liveColumns = draft.columns.filter((c) => !c.isDeleted || !c.isNew);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between border-b border-border bg-background px-4 py-3">
        <PanelHeader
          title={isReadOnly ? "View Table" : isCreating ? "New Table" : "Edit Table"}
          description={
            isCreating
              ? `Define a new table under ${draft.namespace}.`
              : `${isReadOnly ? "Viewing" : "Editing"} ${draft.namespace}.${draft.name}`
          }
        />
        <div className="flex items-center gap-2">
            {isEditing && !isReadOnly && (
              <StudioIconButton
                onClick={handleDropTable}
                tone="destructive"
                tooltip="Drop table"
                aria-label="Drop table"
              >
                <Trash2 data-icon="only" />
              </StudioIconButton>
            )}
            {!isReadOnly && (
              <>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={handleDiscard}
                  disabled={!isDirty}
                  title={!isDirty ? "Nothing to discard" : undefined}
                >
                  Discard
                </Button>
                <Button
                  type="button"
                  size="sm"
                  onClick={handleSave}
                  disabled={!isDirty}
                  title={
                    !isDirty
                      ? "No changes to save"
                      : validation?.hasAny
                        ? "Fix the highlighted errors first"
                        : undefined
                  }
                >
                  {isCreating ? "Review & Create" : "Review & Save"}
                </Button>
              </>
            )}
          </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        <div className="space-y-6 px-6 py-4">
          {isReadOnly && (
            <div className="rounded-md border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
              <strong>Read-only:</strong> {draft.namespace} is a KalamDB-managed
              namespace. You can browse the schema but not modify it.
            </div>
          )}

          {showErrors && validation && validation.table.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <ul className="space-y-0.5">
                {validation.table.map((err, idx) => (
                  <li key={`${err}-${idx}`}>{err}</li>
                ))}
              </ul>
            </div>
          )}

          <section className="space-y-4">
            <div className="flex items-baseline gap-2">
              <h3 className={sectionTitleClassName}>Namespace</h3>
              <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground">
                {draft.namespace}
              </code>
              {!isEditing && (
                <span className="text-[11px] text-muted-foreground">
                  (pick a different one from the sidebar before creating)
                </span>
              )}
            </div>
            <TableTypeSelectField
              value={draft.tableType}
              disabled={isEditing || isReadOnly}
              onChange={handleTableTypeChange}
            />
            <label className="flex flex-col gap-1.5">
              <h3 className={sectionTitleClassName}>Table name</h3>
              <Input
                value={draft.name}
                onChange={(e) =>
                  dispatch(setDraft({ ...draft, name: e.target.value }))
                }
                disabled={isEditing}
                placeholder="e.g. users"
                className="h-9 text-sm"
                autoFocus={isCreating}
              />
              {showErrors && validation?.name && (
                <span className="text-[11px] text-destructive">
                  {validation.name}
                </span>
              )}
            </label>
            <TableOptionsEditor
              draft={draft}
              disabled={isReadOnly}
              onChange={(options) => dispatch(setDraft({ ...draft, options }))}
            />
            {isEditing && selectedTable && !isReadOnly && (
              <TableDataTransferActions
                table={selectedTable}
                onOpenQueryInNewTab={onOpenQueryInNewTab}
              />
            )}
          </section>

          <section className="space-y-2">
            <div className="flex items-center justify-between">
              <h3 className={sectionTitleClassName}>Columns</h3>
              {!isReadOnly && (
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={addColumn}
                  className="gap-1.5 text-xs"
                >
                  <Plus data-icon="inline-start" />
                  Add column
                </Button>
              )}
            </div>
            <div className="overflow-hidden rounded-md border border-border">
              <table className="w-full text-xs">
                <thead className="bg-muted/40">
                  <tr className={`border-b border-border ${chromeLabelClassName}`}>
                    <th className="w-8 px-1 py-2"></th>
                    <th className="w-8 px-2 py-2 text-center">PK</th>
                    <th className="px-2 py-2 text-left">Name</th>
                    <th className="w-32 px-2 py-2 text-left">Type</th>
                    <th className="w-12 px-2 py-2 text-center">Null</th>
                    <th className="w-12 px-2 py-2 text-center">Uniq</th>
                    <th className="w-44 px-2 py-2 text-left">Default</th>
                    <th className="w-9 px-1 py-2"></th>
                  </tr>
                </thead>
                <tbody>
                  {liveColumns.map((col) => (
                    <ColumnRow
                      key={col.id}
                      column={col}
                      isEditingExistingTable={isEditing}
                      readOnly={isReadOnly}
                      autoFocusName={col.id === focusColumnId}
                      error={
                        showErrors
                          ? (validation?.columns[col.id] ?? null)
                          : null
                      }
                      onChange={updateDraftColumn}
                      onDelete={() => deleteColumn(col)}
                      canReorder={isCreating && !isReadOnly}
                      isDragging={draggingColumnId === col.id}
                      onDragStart={() => {
                        draggingColumnIdRef.current = col.id;
                        setDraggingColumnId(col.id);
                      }}
                      onDragOver={() => {}}
                      onDrop={() => {
                        const draggedId = draggingColumnIdRef.current;
                        if (draggedId) moveColumn(draggedId, col.id);
                        draggingColumnIdRef.current = null;
                        setDraggingColumnId(null);
                      }}
                    />
                  ))}
                  {liveColumns.length === 0 && !isReadOnly && (
                    <tr>
                      <td colSpan={8} className="px-3 py-8">
                        <div className="flex flex-col items-center gap-2 text-center">
                          <p className="text-xs text-muted-foreground">
                            This table has no columns yet.
                          </p>
                          <Button
                            type="button"
                            size="sm"
                            onClick={addColumn}
                            className="gap-1.5"
                          >
                            <Plus data-icon="inline-start" />
                            Add your first column
                          </Button>
                        </div>
                      </td>
                    </tr>
                  )}
                  {liveColumns.length === 0 && isReadOnly && (
                    <tr>
                      <td
                        colSpan={8}
                        className="px-3 py-6 text-center text-muted-foreground"
                      >
                        No columns.
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
            {isEditing && (
              <p className="text-[11px] text-muted-foreground">
                Note: primary-key and unique flags can't be changed on existing
                columns (KalamDB doesn't support those ALTERs). You can rename
                columns, change type, nullable and default, or add/drop columns.
              </p>
            )}
          </section>

          {isEditing && selectedTable && (
            <section className="space-y-2">
              <h3 className={sectionTitleClassName}>Metadata</h3>
              <div className="grid grid-cols-2 gap-x-6 gap-y-2 rounded-md border border-border bg-muted/20 px-4 py-3 text-xs">
                <MetaRow label="Type" value={selectedTable.tableType ?? "—"} />
                <MetaRow
                  label="Rows"
                  value={
                    rowCountLoading
                      ? "loading…"
                      : rowCount !== null
                        ? rowCount.toLocaleString()
                        : "—"
                  }
                />
                <MetaRow
                  label="Version"
                  value={
                    selectedTable.version != null
                      ? String(selectedTable.version)
                      : "—"
                  }
                />
                <MetaRow
                  label="Storage ID"
                  value={selectedTable.storageId ?? "—"}
                  mono
                />
                <MetaRow
                  label="Created"
                  value={formatTimestamp(selectedTable.createdAt)}
                />
                <MetaRow
                  label="Updated"
                  value={formatTimestamp(selectedTable.updatedAt)}
                />
              </div>
            </section>
          )}
        </div>
      </div>

      <ConfirmDialog
        open={showDiscardConfirm}
        title="Discard changes?"
        description="You have unsaved edits. They will be lost."
        confirmLabel="Discard"
        variant="destructive"
        onConfirm={() => {
          setShowDiscardConfirm(false);
          if (isEditing && selectedTableKey && selectedTable) {
            dispatch(
              startEditTable({
                tableKey: selectedTableKey,
                draft: tableToDraft(selectedTable),
              }),
            );
          } else {
            dispatch(discardEdit());
          }
        }}
        onClose={() => setShowDiscardConfirm(false)}
      />
    </div>
  );
}
