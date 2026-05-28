import { useEffect, useState } from "react";
import { Plus } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { chromeLabelClassName } from "@/components/layout/typography";
import { cn } from "@/lib/utils";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { StudioTable, StudioColumn } from "../../shared/types";
import { classifyFieldKind, coerceFieldValue, type FieldKind } from "../../shared/value-validation";

interface InsertRowDialogProps {
  open: boolean;
  table: StudioTable;
  onSubmit: (values: Record<string, unknown>) => void;
  onClose: () => void;
}

function isSystemColumn(c: StudioColumn): boolean {
  return c.name.startsWith("_");
}

interface FieldState {
  raw: string;
  error: string | null;
}

function emptyField(): FieldState {
  return { raw: "", error: null };
}

export function InsertRowDialog({ open, table, onSubmit, onClose }: InsertRowDialogProps) {
  const insertableColumns = table.columns.filter((c) => !isSystemColumn(c));
  const [fields, setFields] = useState<Record<string, FieldState>>({});
  const [submitAttempted, setSubmitAttempted] = useState(false);
  const [globalError, setGlobalError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setFields({});
      setSubmitAttempted(false);
      setGlobalError(null);
    }
  }, [open]);

  const setRaw = (col: string, raw: string, kind: FieldKind) => {
    const { error } = coerceFieldValue(raw, kind);
    setFields((prev) => ({ ...prev, [col]: { raw, error } }));
  };

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    setSubmitAttempted(true);
    setGlobalError(null);

    const out: Record<string, unknown> = {};
    let hasError = false;
    for (const col of insertableColumns) {
      const kind = classifyFieldKind(col.dataType);
      const state = fields[col.name] ?? emptyField();
      const { value, error } = coerceFieldValue(state.raw, kind);
      if (error) {
        hasError = true;
        setFields((prev) => ({ ...prev, [col.name]: { raw: state.raw, error } }));
        continue;
      }
      if (value !== undefined) out[col.name] = value;
    }
    if (hasError) return;
    if (Object.keys(out).length === 0) {
      setGlobalError("Fill all required columns.");
      return;
    }
    onSubmit(out);
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Plus className="h-4 w-4" />
            Insert row into {table.namespace}.{table.name}
          </DialogTitle>
          <DialogDescription>
            Leave a field blank to use its DEFAULT (or NULL if nullable).
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="flex max-h-[60vh] flex-col gap-3 overflow-y-auto px-1">
          {insertableColumns.map((col) => {
            const kind = classifyFieldKind(col.dataType);
            const state = fields[col.name] ?? emptyField();
            const showError = submitAttempted && state.error;
            return (
              <label key={col.name} className="flex flex-col gap-1 text-xs">
                <span className="flex items-baseline gap-2">
                  <span className="font-medium text-foreground">{col.name}</span>
                  <span className="font-mono text-[10px] text-muted-foreground">{col.dataType}</span>
                  {col.isPrimaryKey && (
                    <span className={cn(chromeLabelClassName, "text-yellow-600 dark:text-yellow-500")}>PK</span>
                  )}
                  {!col.isNullable && (
                    <span className={chromeLabelClassName}>required</span>
                  )}
                </span>

                {kind === "boolean" ? (
                  <Select value={state.raw} onValueChange={(v) => setRaw(col.name, v, kind)}>
                    <SelectTrigger className="h-8 text-xs">
                      <SelectValue placeholder={col.isNullable ? "NULL" : "DEFAULT"} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="true" className="text-xs">true</SelectItem>
                      <SelectItem value="false" className="text-xs">false</SelectItem>
                    </SelectContent>
                  </Select>
                ) : kind === "json" || kind === "embedding" ? (
                  <textarea
                    value={state.raw}
                    onChange={(e) => setRaw(col.name, e.target.value, kind)}
                    placeholder={
                      col.isNullable
                        ? "NULL"
                        : kind === "embedding"
                          ? "[0.1, 0.2, 0.3]"
                          : '{"key": "value"}'
                    }
                    rows={3}
                    className="rounded-md border border-input bg-background px-3 py-2 font-mono text-xs ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                  />
                ) : (
                  <Input
                    type={
                      kind === "smallint" || kind === "int" || kind === "float"
                        ? "number"
                        : kind === "datetime"
                          ? "datetime-local"
                          : kind === "date"
                            ? "date"
                            : kind === "time"
                              ? "time"
                              : "text"
                    }
                    step={
                      kind === "float"
                        ? "any"
                        : kind === "smallint" || kind === "int"
                          ? "1"
                          : undefined
                    }
                    value={state.raw}
                    onChange={(e) => setRaw(col.name, e.target.value, kind)}
                    placeholder={col.isNullable ? "NULL" : "DEFAULT"}
                    className="h-8 font-mono text-xs"
                  />
                )}

                {showError && (
                  <span className="text-[11px] text-destructive">{state.error}</span>
                )}
              </label>
            );
          })}

          {globalError && (
            <p className="text-xs text-destructive">{globalError}</p>
          )}
          <DialogFooter className="gap-2 sm:gap-2">
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit">Insert row</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
