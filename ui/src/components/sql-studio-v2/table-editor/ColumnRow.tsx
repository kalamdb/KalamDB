import { GripVertical, Key, KeyRound, Undo2, Trash2 } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { KALAMDB_TYPES, DEFAULT_NONE, DEFAULT_CUSTOM, defaultPresetsForType, type DraftColumn } from "./types";

interface ColumnRowProps {
  column: DraftColumn;
  isEditingExistingTable: boolean;
  readOnly?: boolean;
  autoFocusName?: boolean;
  error?: string | null;
  onChange: (next: DraftColumn) => void;
  onDelete: () => void;
  onDragStart?: () => void;
  onDragOver?: () => void;
  onDrop?: () => void;
  isDragging?: boolean;
  canReorder?: boolean;
}

export function ColumnRow({
  column,
  isEditingExistingTable,
  readOnly,
  autoFocusName,
  error,
  onChange,
  onDelete,
  onDragStart,
  onDragOver,
  onDrop,
  isDragging,
  canReorder,
}: ColumnRowProps) {
  const isExistingCol = isEditingExistingTable && !column.isNew;
  const lockName = readOnly || column.isDeleted;
  const lockPkUnique = isExistingCol || readOnly || column.isDeleted;
  const lockEverything = !!readOnly || column.isDeleted;

  const update = <K extends keyof DraftColumn>(key: K, value: DraftColumn[K]) => {
    onChange({ ...column, [key]: value });
  };

  const defaultPresets = defaultPresetsForType(column.type);
  const matchedPreset = defaultPresets.find(
    (p) => p.value === column.defaultExpr && p.value !== DEFAULT_NONE && p.value !== DEFAULT_CUSTOM,
  );
  const presetValue = matchedPreset
    ? matchedPreset.value
    : column.defaultExpr === DEFAULT_CUSTOM || (column.defaultExpr.length > 0 && !matchedPreset)
      ? DEFAULT_CUSTOM
      : DEFAULT_NONE;
  const showCustomInput = presetValue === DEFAULT_CUSTOM;

  return (
    <>
    <tr
      className={cn(
        "last:border-b-0",
        !error && "border-b border-border/50",
        column.isDeleted && "bg-destructive/10",
        isDragging && "opacity-50",
      )}
      onDragOver={(event) => {
        if (!canReorder) return;
        event.preventDefault();
        onDragOver?.();
      }}
      onDrop={(event) => {
        if (!canReorder) return;
        event.preventDefault();
        onDrop?.();
      }}
    >
      <td className="w-8 px-1 py-1 text-center align-middle">
        <button
          type="button"
          aria-label={`Reorder ${column.name || "column"}`}
          draggable={canReorder && !column.isDeleted}
          disabled={!canReorder || column.isDeleted}
          onDragStart={(event) => {
            if (event.dataTransfer) {
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", column.id);
            }
            onDragStart?.();
          }}
          className={cn(
            "cursor-grab rounded p-1 text-muted-foreground/60 transition-colors active:cursor-grabbing",
            "hover:bg-accent hover:text-foreground",
            (!canReorder || column.isDeleted) && "cursor-not-allowed opacity-30",
          )}
          title="Drag to reorder"
        >
          <GripVertical className="h-3.5 w-3.5" />
        </button>
      </td>

      <td className="px-2 py-1 text-center align-middle">
        <button
          type="button"
          onClick={() => update("isPrimaryKey", !column.isPrimaryKey)}
          disabled={lockPkUnique}
          className={cn(
            "rounded p-1 transition-colors",
            "hover:bg-accent",
            column.isPrimaryKey ? "text-yellow-500" : "text-muted-foreground/40",
            lockPkUnique && "cursor-not-allowed opacity-60",
          )}
          title={column.isPrimaryKey ? "Primary key" : "Not a primary key"}
        >
          {column.isPrimaryKey ? <KeyRound className="h-3.5 w-3.5" /> : <Key className="h-3.5 w-3.5" />}
        </button>
      </td>

      <td className="px-1 py-1 align-middle">
        <Input
          value={column.name}
          onChange={(e) => update("name", e.target.value)}
          placeholder="column_name"
          disabled={lockName}
          autoFocus={autoFocusName && !lockName}
          className="h-7 text-xs"
        />
      </td>

      <td className="px-1 py-1 align-middle">
        <Select
          value={column.type}
          onValueChange={(v) => {
            const nextDefaultPresets = defaultPresetsForType(v);
            const defaultStillFits =
              !column.defaultExpr ||
              column.defaultExpr === DEFAULT_CUSTOM ||
              nextDefaultPresets.some((preset) => preset.value === column.defaultExpr);
            onChange({
              ...column,
              type: v,
              defaultExpr: defaultStillFits ? column.defaultExpr : "",
            });
          }}
          disabled={lockEverything}
        >
          <SelectTrigger className="h-7 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {KALAMDB_TYPES.map((t) => (
              <SelectItem key={t} value={t} className="text-xs">
                {t}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </td>

      <td className="px-2 py-1 text-center align-middle">
        <input
          type="checkbox"
          checked={!column.isNotNull}
          onChange={(e) => update("isNotNull", !e.target.checked)}
          disabled={lockEverything}
          className="h-3.5 w-3.5 cursor-pointer rounded border-border accent-blue-500 disabled:cursor-not-allowed disabled:opacity-50"
          title="Nullable"
        />
      </td>

      <td className="px-2 py-1 text-center align-middle">
        <input
          type="checkbox"
          checked={column.isUnique}
          onChange={(e) => update("isUnique", e.target.checked)}
          disabled={lockPkUnique}
          className="h-3.5 w-3.5 cursor-pointer rounded border-border accent-blue-500 disabled:cursor-not-allowed disabled:opacity-50"
          title="Unique"
        />
      </td>

      <td className="px-1 py-1 align-middle">
        {showCustomInput ? (
          <Input
            value={column.defaultExpr === DEFAULT_CUSTOM ? "" : column.defaultExpr}
            onChange={(e) => update("defaultExpr", e.target.value)}
            placeholder="DEFAULT expr"
            disabled={lockEverything}
            className="h-7 text-xs font-mono"
            autoFocus
          />
        ) : (
          <Select
            value={presetValue}
            onValueChange={(v) => {
              if (v === DEFAULT_NONE) {
                update("defaultExpr", "");
              } else if (v === DEFAULT_CUSTOM) {
                update("defaultExpr", DEFAULT_CUSTOM);
              } else {
                update("defaultExpr", v);
              }
            }}
            disabled={lockEverything}
          >
            <SelectTrigger className="h-7 text-xs">
              <SelectValue placeholder="(none)" />
            </SelectTrigger>
            <SelectContent>
              {defaultPresets.map((p) => (
                <SelectItem key={p.value} value={p.value} className="text-xs">
                  {p.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </td>

      <td className="px-1 py-1 text-center align-middle">
        {!readOnly && (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className={cn(
              "h-7 w-7",
              column.isDeleted && "text-destructive hover:text-destructive",
            )}
            onClick={onDelete}
            title={column.isDeleted ? "Undo delete" : "Delete column"}
          >
            {column.isDeleted ? (
              <Undo2 className="h-3.5 w-3.5" />
            ) : (
              <Trash2 className="h-3.5 w-3.5" />
            )}
          </Button>
        )}
      </td>
    </tr>
    {error && (
      <tr className="border-b border-border/50">
        <td colSpan={8} className="px-3 py-1 text-[11px] text-destructive">
          {error}
        </td>
      </tr>
    )}
    </>
  );
}
