import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  KeyRound,
  Plus,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { CodeBlock } from "@/components/ui/code-block";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useTableChanges } from "@/hooks/useTableChanges";
import { CellContextMenu, type CellContextMenuState } from "./table/CellContextMenu";
import { InlineCellEditor, type InlineEditContext } from "./table/InlineCellEditor";
import { InsertRowDialog } from "./table/InsertRowDialog";
import { generateSqlStatements, generateInsertSql } from "./table/utils/sqlGenerator";
import { extractTableContext } from "./table/utils/sqlParser";
import { useSqlPreview } from "@/components/sql-preview";
import { CellDisplay } from "@/components/datatype-display";
import { KalamCellValue } from "@kalamdb/client";
import { executeSql } from "@/lib/kalam-client";
import { executeSqlPreviewStatement } from "@/components/sql-studio-v2/table-editor/run-sql";
import { useToast } from "@/components/ui/toaster-provider";
import { classifyFieldKind, coerceFieldValue } from "@/components/sql-studio-v2/shared/value-validation";
import { LIVE_META, LIVE_HIGHLIGHT_DURATION_MS } from "@/features/sql-studio/state/sqlStudioWorkspaceSlice";
import { cn } from "@/lib/utils";
import { StudioExecutionLog } from "./logs/StudioExecutionLog";
import { chromeLabelClassName } from "@/components/layout/typography";
import type { QueryResultData, SqlStudioResultView, StudioTable } from "../shared/types";

interface StudioResultsGridProps {
  result: QueryResultData | null;
  isRunning: boolean;
  isLiveMode: boolean;
  liveBatch?: { hasMore: boolean; batchNum?: number; status?: string };
  liveAutoFetchBatches?: boolean;
  activeSql: string;
  selectedTable: StudioTable | null;
  currentUsername: string;
  resultView: SqlStudioResultView;
  onResultViewChange: (view: SqlStudioResultView) => void;
  onFetchNextBatch?: () => void;
  onRefreshAfterCommit: () => void;
  actions?: ReactNode;
}

type SortDirection = "asc" | "desc";
type SortState = { columnName: string; direction: SortDirection } | null;
type RowData = Record<string, unknown>;
type SelectedCell = { rowIndex: number; columnName: string } | null;

interface CellViewerState {
  open: boolean;
  title: string;
  content: unknown;
  editedValue: unknown;
  isNull: boolean;
  dataType?: string;
  rowIndex?: number;
  columnName?: string;
  canEdit: boolean;
}

const DEFAULT_PAGE_SIZE = 100;
const PAGE_SIZE_OPTIONS = [25, 50, 100, 250] as const;
const DEFAULT_COLUMN_WIDTH = 220;
const MIN_COLUMN_WIDTH = 96;
const MAX_COLUMN_WIDTH = 640;
const SELECT_COLUMN_WIDTH = 44;
const LIVE_COLUMN_WIDTH = 148;
const MAX_RENDERED_ROWS = 1000;

function stringifyCellValue(value: unknown): string {
  // Unwrap KalamCellValue wrappers before stringifying
  const raw = value instanceof KalamCellValue ? value.toJson() : value;
  if (raw === null || raw === undefined) {
    return "null";
  }
  if (typeof raw === "string") {
    return raw;
  }
  if (typeof raw === "number" || typeof raw === "boolean" || typeof raw === "bigint") {
    return String(raw);
  }
  try {
    return JSON.stringify(raw, null, 2);
  } catch {
    return String(raw);
  }
}

function compareValues(left: unknown, right: unknown): number {
  if (left === right) {
    return 0;
  }
  if (left === null || left === undefined) {
    return 1;
  }
  if (right === null || right === undefined) {
    return -1;
  }

  const leftType = typeof left;
  const rightType = typeof right;

  if (leftType === "number" && rightType === "number") {
    return (left as number) - (right as number);
  }

  if (leftType === "boolean" && rightType === "boolean") {
    return Number(left) - Number(right);
  }

  return String(left).localeCompare(String(right), undefined, { numeric: true, sensitivity: "base" });
}

function unwrapCellValue(value: unknown): unknown {
  return value instanceof KalamCellValue ? value.toJson() : value;
}

function parseSeqValue(value: unknown): bigint | null {
  const raw = unwrapCellValue(value);
  if (typeof raw === "bigint") {
    return raw;
  }
  if (typeof raw === "number" && Number.isInteger(raw)) {
    return BigInt(raw);
  }
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    if (/^-?\d+$/.test(trimmed)) {
      try {
        return BigInt(trimmed);
      } catch {
        return null;
      }
    }
  }
  return null;
}

export function StudioResultsGrid({
  result,
  isRunning,
  isLiveMode,
  liveBatch,
  liveAutoFetchBatches = false,
  activeSql,
  selectedTable,
  currentUsername,
  resultView,
  onResultViewChange,
  onFetchNextBatch,
  onRefreshAfterCommit,
  actions,
}: StudioResultsGridProps) {
  const [sortState, setSortState] = useState<SortState>(null);
  const [pageIndex, setPageIndex] = useState(0);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [columnWidths, setColumnWidths] = useState<Record<string, number>>({});
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const [selectedCell, setSelectedCell] = useState<SelectedCell>(null);
  const [cellContextMenu, setCellContextMenu] = useState<CellContextMenuState | null>(null);
  const [inlineEditContext, setInlineEditContext] = useState<InlineEditContext | null>(null);
  const [cellViewer, setCellViewer] = useState<CellViewerState>({
    open: false,
    title: "",
    content: "",
    editedValue: "",
    isNull: false,
    dataType: undefined,
    rowIndex: undefined,
    columnName: undefined,
    canEdit: false,
  });
  const [hasUnseenResults, setHasUnseenResults] = useState(false);
  const [hasUnseenLogs, setHasUnseenLogs] = useState(false);
  const previousResultRef = useRef<QueryResultData | null>(null);

  const {
    edits,
    deletions,
    changeCount,
    getRowStatus,
    isCellEdited,
    getCellEditedValue,
    editCell,
    deleteRow,
    undeleteRow,
    undoRowEdits,
    discardAll,
  } = useTableChanges();
  const { openSqlPreview } = useSqlPreview();
  const { notify } = useToast();
  const [showInsertRow, setShowInsertRow] = useState(false);

  useEffect(() => {
    discardAll();
    setSortState(null);
    setPageIndex(0);
    setColumnWidths({});
    setSelectedRows(new Set());
    setSelectedCell(null);
    setCellContextMenu(null);
    setInlineEditContext(null);
    setCellViewer({
      open: false,
      title: "",
      content: "",
      editedValue: "",
      isNull: false,
      dataType: undefined,
      rowIndex: undefined,
      columnName: undefined,
      canEdit: false,
    });
  }, [result, discardAll]);

  useEffect(() => {
    if (resultView === "results") {
      setHasUnseenResults(false);
    }
    if (resultView === "log") {
      setHasUnseenLogs(false);
    }
  }, [resultView]);

  useEffect(() => {
    const previous = previousResultRef.current;
    if (result) {
      const rowsChanged =
        !previous ||
        previous.rows !== result.rows ||
        previous.schema !== result.schema ||
        previous.rowCount !== result.rowCount;
      const logsChanged = !previous || previous.logs.length !== result.logs.length;

      if (rowsChanged && resultView !== "results" && (result.rows.length > 0 || result.schema.length > 0)) {
        setHasUnseenResults(true);
      }
      if (logsChanged && resultView !== "log" && result.logs.length > 0) {
        setHasUnseenLogs(true);
      }
    }
    previousResultRef.current = result;
  }, [result, resultView]);

  const rawSchema = result?.schema ?? [];
  // In live mode, hide internal _live_* metadata columns (last_change is visible, not prefixed with _live_)
  const schema = isLiveMode
    ? rawSchema.filter((f) => !f.name.startsWith("_live_"))
    : rawSchema;
  const resultRows =
    result?.status === "success"
      ? result.rows
      : [];
  const sourceRows =
    isLiveMode
      ? resultRows
      : resultRows.slice(0, MAX_RENDERED_ROWS);
  const parsedTableContext = useMemo(() => extractTableContext(activeSql), [activeSql]);
  const cellNamespace = parsedTableContext?.namespace ?? selectedTable?.namespace;
  const cellTableName = parsedTableContext?.tableName ?? selectedTable?.name;
  const insertTargetTable = useMemo(() => {
    if (parsedTableContext) {
      const selectedMatchesParsed =
        selectedTable
        && selectedTable.namespace.toLowerCase() === parsedTableContext.namespace
        && selectedTable.name.toLowerCase() === parsedTableContext.tableName;

      if (selectedMatchesParsed && selectedTable) {
        return selectedTable;
      }

      return {
        database: selectedTable?.database ?? "kalamdb",
        namespace: parsedTableContext.namespace,
        name: parsedTableContext.tableName,
        tableType: selectedTable?.tableType ?? "user",
        columns: schema.map((field) => ({
          name: field.name,
          dataType: field.dataType,
          isNullable: true,
          isPrimaryKey: Boolean(field.isPrimaryKey),
          ordinal: field.index,
        })),
      };
    }

    return selectedTable;
  }, [parsedTableContext, selectedTable, schema]);
  const isSystemTable = cellNamespace?.toLowerCase() === "system";
  const isSuccess = !isRunning && result?.status === "success";
  const hasTabularResults = isSuccess && schema.length > 0;
  const canMutateRows = hasTabularResults && !isLiveMode && !isSystemTable;

  const sortedRowIndices = useMemo(() => {
    const indices = sourceRows.map((_, rowIndex) => rowIndex);
    
    if (sortState) {
      // Manual sort active - use it
      const { columnName, direction } = sortState;
      indices.sort((leftIndex, rightIndex) => {
        const leftValue = getCellEditedValue(leftIndex, columnName) ?? sourceRows[leftIndex]?.[columnName];
        const rightValue = getCellEditedValue(rightIndex, columnName) ?? sourceRows[rightIndex]?.[columnName];
        const comparison = compareValues(leftValue, rightValue);
        return direction === "asc" ? comparison : comparison * -1;
      });
      return indices;
    }

    if (isLiveMode) {
      // Live mode: sort newest-first by _seq when present.
      indices.sort((leftIndex, rightIndex) => {
        const leftRow = sourceRows[leftIndex];
        const rightRow = sourceRows[rightIndex];
        const leftSeq = parseSeqValue(leftRow?._seq);
        const rightSeq = parseSeqValue(rightRow?._seq);

        if (leftSeq !== null && rightSeq !== null) {
          if (leftSeq === rightSeq) return 0;
          return leftSeq > rightSeq ? -1 : 1;
        }
        if (leftSeq !== null) return -1;
        if (rightSeq !== null) return 1;

        const leftChangeType = leftRow?.[LIVE_META.CHANGE_TYPE] as string | undefined;
        const rightChangeType = rightRow?.[LIVE_META.CHANGE_TYPE] as string | undefined;
        if (leftChangeType !== rightChangeType) {
          return String(rightChangeType ?? "").localeCompare(String(leftChangeType ?? ""));
        }
        return 0;
      });
    }
    
    return indices;
  }, [sourceRows, sortState, edits, getCellEditedValue, isLiveMode, schema]);

  const displayRowIndices = useMemo(
    () => isLiveMode ? sortedRowIndices.slice(0, MAX_RENDERED_ROWS) : sortedRowIndices,
    [isLiveMode, sortedRowIndices],
  );
  const pageCount = Math.max(1, Math.ceil(displayRowIndices.length / pageSize));
  const currentPageStart = pageIndex * pageSize;
  const currentPageRows = displayRowIndices.slice(currentPageStart, currentPageStart + pageSize);
  const columnNames = useMemo(() => schema.map((field) => field.name), [schema]);
  const selectedCellKey = selectedCell ? `${selectedCell.rowIndex}:${selectedCell.columnName}` : null;

  useEffect(() => {
    if (pageIndex > pageCount - 1) {
      setPageIndex(Math.max(0, pageCount - 1));
    }
  }, [pageIndex, pageCount]);

  useEffect(() => {
    if (!selectedCell) {
      return;
    }

    if (!currentPageRows.includes(selectedCell.rowIndex)) {
      setSelectedCell(null);
    }
  }, [selectedCell, currentPageRows]);

  useEffect(() => {
    if (canMutateRows) {
      return;
    }

    setSelectedRows(new Set());
  }, [canMutateRows]);

  useEffect(() => {
    if (!isLiveMode) {
      return;
    }
    setSelectedRows(new Set());
  }, [isLiveMode]);

  const selectedVisibleRowCount = currentPageRows.filter((rowIndex) => selectedRows.has(rowIndex)).length;
  const allVisibleRowsSelected =
    currentPageRows.length > 0 && selectedVisibleRowCount === currentPageRows.length;

  const getPrimaryKeyValues = (rowIndex: number): Record<string, unknown> => {
    const row = sourceRows[rowIndex];
    if (!row) {
      return {};
    }

    const schemaPkColumns = schema
      .filter((field) => field.isPrimaryKey)
      .sort((left, right) => left.index - right.index)
      .map((field) => field.name);

    const selectedPkColumns = (selectedTable?.columns ?? [])
      .filter((column) => column.isPrimaryKey)
      .map((column) => column.name);

    const pkColumns =
      schemaPkColumns.length > 0
        ? schemaPkColumns
        : selectedPkColumns.length > 0
          ? selectedPkColumns
          : schema
              .map((field) => field.name)
              .filter((name) => name.toLowerCase() === "id" || name.endsWith("_id"));

    const fallbackColumn = schema[0]?.name;
    const keyColumns = pkColumns.length > 0 ? pkColumns : fallbackColumn ? [fallbackColumn] : [];

    return keyColumns.reduce<Record<string, unknown>>((acc, columnName) => {
      acc[columnName] = row[columnName];
      return acc;
    }, {});
  };

  const handleSortColumn = (columnName: string) => {
    setSortState((previous) => {
      if (!previous || previous.columnName !== columnName) {
        return { columnName, direction: "asc" };
      }
      if (previous.direction === "asc") {
        return { columnName, direction: "desc" };
      }
      return null;
    });
    setPageIndex(0);
  };

  const columnWidth = useCallback(
    (columnName: string) => columnWidths[columnName] ?? DEFAULT_COLUMN_WIDTH,
    [columnWidths],
  );

  const handleResizeColumn = useCallback(
    (event: ReactMouseEvent, columnName: string) => {
      event.preventDefault();
      event.stopPropagation();

      const startX = event.clientX;
      const startWidth = columnWidth(columnName);

      const handleMouseMove = (moveEvent: MouseEvent) => {
        const nextWidth = Math.max(
          MIN_COLUMN_WIDTH,
          Math.min(MAX_COLUMN_WIDTH, startWidth + moveEvent.clientX - startX),
        );
        setColumnWidths((current) => ({
          ...current,
          [columnName]: nextWidth,
        }));
      };

      const handleMouseUp = () => {
        window.removeEventListener("mousemove", handleMouseMove);
        window.removeEventListener("mouseup", handleMouseUp);
      };

      window.addEventListener("mousemove", handleMouseMove);
      window.addEventListener("mouseup", handleMouseUp);
    },
    [columnWidth],
  );

  const handleEditCell = (rowIndex: number, columnName: string, currentValue: unknown) => {
    if (!canMutateRows) {
      return;
    }
    openCellViewer(currentValue, columnName, rowIndex, true);
  };

  const handleSaveInlineEdit = (
    rowIndex: number,
    columnName: string,
    oldValue: unknown,
    newValue: unknown,
  ) => {
    if (!canMutateRows) {
      setInlineEditContext(null);
      return;
    }

    const pkValues = getPrimaryKeyValues(rowIndex);
    editCell(rowIndex, columnName, oldValue, newValue, pkValues);
    setInlineEditContext(null);
  };

  const handleDeleteRow = (rowIndex: number) => {
    if (!canMutateRows) {
      return;
    }

    const row = sourceRows[rowIndex];
    if (!row) {
      return;
    }

    const pkValues = getPrimaryKeyValues(rowIndex);
    deleteRow(rowIndex, pkValues, row);
    setSelectedRows((previous) => {
      const next = new Set(previous);
      next.delete(rowIndex);
      return next;
    });
  };

  const handleDeleteSelectedRows = () => {
    if (!canMutateRows) {
      return;
    }

    const targetRows = Array.from(selectedRows);
    targetRows.forEach((rowIndex) => {
      const row = sourceRows[rowIndex];
      if (!row) {
        return;
      }
      const pkValues = getPrimaryKeyValues(rowIndex);
      deleteRow(rowIndex, pkValues, row);
    });
    setSelectedRows(new Set());
  };

  const openCellViewer = useCallback(
    (value: unknown, columnName: string, rowIndex: number, editable = false) => {
      const dataType = schema.find((field) => field.name === columnName)?.dataType;
      setCellViewer({
        open: true,
        title: `${columnName} · Row ${rowIndex + 1}${dataType ? ` (${dataType})` : ""}`,
        content: value,
        editedValue: value === null ? "" : stringifyCellValue(value),
        isNull: value === null,
        dataType,
        rowIndex,
        columnName,
        canEdit: editable && canMutateRows,
      });
    },
    [canMutateRows, schema],
  );

  const moveSelectionByArrow = useCallback(
    (rowDelta: number, colDelta: number) => {
      if (!selectedCell || currentPageRows.length === 0 || columnNames.length === 0) {
        return;
      }

      const currentRowPosition = currentPageRows.indexOf(selectedCell.rowIndex);
      const currentColumnPosition = columnNames.findIndex((name) => name === selectedCell.columnName);

      if (currentRowPosition < 0 || currentColumnPosition < 0) {
        return;
      }

      const nextRowPosition = Math.max(0, Math.min(currentPageRows.length - 1, currentRowPosition + rowDelta));
      const nextColumnPosition = Math.max(0, Math.min(columnNames.length - 1, currentColumnPosition + colDelta));
      const nextRowIndex = currentPageRows[nextRowPosition];
      const nextColumnName = columnNames[nextColumnPosition];
      setSelectedCell({
        rowIndex: nextRowIndex,
        columnName: nextColumnName,
      });

      const nextCell = document.querySelector(
        `[data-row-index="${nextRowIndex}"][data-column-name="${nextColumnName}"]`,
      ) as HTMLElement | null;
      nextCell?.scrollIntoView({ block: "nearest", inline: "nearest" });
      nextCell?.focus();
    },
    [selectedCell, currentPageRows, columnNames],
  );

  useEffect(() => {
    if (!selectedCell) {
      return;
    }

    const handleArrowNavigation = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }

      if (event.key === "ArrowUp") {
        event.preventDefault();
        moveSelectionByArrow(-1, 0);
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        moveSelectionByArrow(1, 0);
      } else if (event.key === "ArrowLeft") {
        event.preventDefault();
        moveSelectionByArrow(0, -1);
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        moveSelectionByArrow(0, 1);
      }
    };

    window.addEventListener("keydown", handleArrowNavigation);
    return () => window.removeEventListener("keydown", handleArrowNavigation);
  }, [selectedCell, moveSelectionByArrow]);

  const handleReviewChanges = () => {
    if (!canMutateRows) {
      return;
    }

    const parsed = extractTableContext(activeSql);
    const namespace = parsed?.namespace ?? selectedTable?.namespace;
    const tableName = parsed?.tableName ?? selectedTable?.name;

    if (!namespace || !tableName) {
      alert("Unable to determine target table for commit. Select a table and run a simple SELECT ... FROM namespace.table query.");
      return;
    }

    const generated = generateSqlStatements(namespace, tableName, edits, deletions);
    if (generated.statements.length === 0) {
      return;
    }

    openSqlPreview({
      title: "Review Changes",
      description: `${generated.updateCount} update(s), ${generated.deleteCount} delete(s)${generated.isTransactional ? "; wrapped in BEGIN/COMMIT" : ""}`,
      sql: generated.fullSql,
      statements: generated.statements,
      editable: false,
      onExecute: async (batchSql: string) => {
        await executeSql(batchSql);
      },
      onComplete: async () => {
        discardAll();
        onRefreshAfterCommit();
      },
      onDiscard: () => {
        discardAll();
      },
    });
  };

  const editCount = edits.size;
  const deleteCount = deletions.size;
  const logCount = result?.logs.length ?? 0;
  const showResultsTable = hasTabularResults && resultView === "results";
  const canFetchNextLiveBatch = Boolean(
    isLiveMode
    && resultView === "results"
    && liveBatch?.hasMore
    && !liveAutoFetchBatches
    && onFetchNextBatch,
  );

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-background">
      <div
        data-testid="sql-results-header"
        className="flex h-11 shrink-0 items-end justify-between gap-3 border-b border-border px-3"
      >
        <Tabs
          value={resultView}
          onValueChange={(value) => onResultViewChange(value as SqlStudioResultView)}
          className="h-full shrink-0"
        >
          <TabsList variant="line" className="h-full gap-5 border-b-0">
            <TabsTrigger
              value="results"
              className="relative h-full rounded-none border-b-2 border-transparent px-0 pt-0 pb-3 text-xs font-medium text-muted-foreground shadow-none data-[state=active]:border-sky-500 data-[state=active]:bg-transparent data-[state=active]:text-foreground"
            >
              <span>Results</span>
              {hasUnseenResults && (
                <span className="ml-1.5 inline-block h-1.5 w-1.5 rounded-full bg-sky-500" />
              )}
            </TabsTrigger>
            <TabsTrigger
              value="log"
              className="relative h-full rounded-none border-b-2 border-transparent px-0 pt-0 pb-3 text-xs font-medium text-muted-foreground shadow-none data-[state=active]:border-sky-500 data-[state=active]:bg-transparent data-[state=active]:text-foreground"
            >
              <span>Log ({logCount})</span>
              {hasUnseenLogs && (
                <span className="ml-1.5 inline-block h-1.5 w-1.5 rounded-full bg-sky-500" />
              )}
            </TabsTrigger>
          </TabsList>
        </Tabs>
        <div className="ml-auto flex h-full min-w-0 flex-nowrap items-center gap-2 overflow-visible whitespace-nowrap">
          {isSuccess && (
            <div className="flex min-w-0 flex-nowrap items-center gap-2 text-[11px] text-muted-foreground">
              <span>{result.rowCount.toLocaleString()} rows</span>
              <span>took {Math.round(result.tookMs)} ms</span>
              <span>as user: {currentUsername}</span>
              {result.rowCount > MAX_RENDERED_ROWS && (
                <span className="rounded bg-amber-500/20 px-1.5 py-0.5 text-[10px] text-amber-700">
                  showing first {MAX_RENDERED_ROWS.toLocaleString()}
                </span>
              )}
              {canMutateRows && resultView === "results" && selectedRows.size > 0 && (
                <span className="rounded bg-sky-500/20 px-1.5 py-0.5 text-[10px] text-sky-700">
                  {selectedRows.size} selected
                </span>
              )}
              {resultView === "results" && sortState && (
                <span className="rounded bg-border px-1.5 py-0.5 text-[10px] text-foreground">
                  {sortState.columnName} ({sortState.direction})
                </span>
              )}
              {canFetchNextLiveBatch && (
                <Button
                  size="sm"
                  variant="secondary"
                  className="h-7 gap-1.5"
                  onClick={onFetchNextBatch}
                >
                  <ChevronRight className="h-3.5 w-3.5" />
                  Fetch next batch
                </Button>
              )}
              <Button
                size="sm"
                variant="destructive"
                className={cn("h-7 gap-1.5", (!canMutateRows || resultView !== "results" || selectedRows.size === 0) && "invisible pointer-events-none")}
                onClick={handleDeleteSelectedRows}
                disabled={!canMutateRows || resultView !== "results" || selectedRows.size === 0}
              >
                <Trash2 className="h-3.5 w-3.5" />
                Delete selected
              </Button>
            </div>
          )}
          {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
        </div>
      </div>

      {isRunning && (
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
          Running query...
        </div>
      )}

      {!isRunning && !result && (
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
          Run your query to view results.
        </div>
      )}

      {!isRunning && result?.status === "error" && resultView === "results" && (
        <Alert variant="destructive" className="m-3">
          <AlertTitle>Execution failed</AlertTitle>
          <AlertDescription>
            {result.errorMessage ?? "The query could not be completed. Open the Log tab for statement-level details."}
          </AlertDescription>
        </Alert>
      )}

      {!isRunning && result?.status === "error" && resultView === "log" && (
        <StudioExecutionLog logs={result.logs} status={result.status} />
      )}

      {!isRunning && result?.status === "success" && resultView === "log" && (
        <StudioExecutionLog logs={result.logs} status={result.status} />
      )}

      {!isRunning && result?.status === "success" && resultView === "results" && !hasTabularResults && (
        <div className="flex flex-1 items-center justify-center px-4 text-sm text-muted-foreground">
          No tabular result set for this execution. Open the Log tab to inspect statement output.
        </div>
      )}

      {!isRunning && showResultsTable && (
        <>
          {canMutateRows && (
            <div className="flex h-10 items-center justify-between border-b border-border bg-amber-50/70 px-3 /20">
              <div className="flex items-center gap-3">
                {insertTargetTable && (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setShowInsertRow(true)}
                    className="h-7 gap-1.5 text-amber-700 hover:bg-amber-100 hover:text-amber-900"
                  >
                    <Plus className="h-3.5 w-3.5" />
                    Insert row
                  </Button>
                )}
                <div className="truncate text-xs text-amber-700">
                  {changeCount === 0
                    ? "No pending table changes"
                    : `${changeCount} change${changeCount === 1 ? "" : "s"} • ${editCount} edit${editCount === 1 ? "" : "s"} • ${deleteCount} delete${deleteCount === 1 ? "" : "s"}`}
                </div>
              </div>
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={discardAll}
                  disabled={changeCount === 0}
                  className="h-7 gap-1.5 text-amber-700 hover:bg-amber-100 hover:text-amber-900 :bg-amber-900/40 :text-amber-200"
                >
                  Discard
                </Button>
                <Button
                  size="sm"
                  onClick={handleReviewChanges}
                  disabled={changeCount === 0}
                  className="h-7 gap-1.5"
                >
                  Review & Commit
                </Button>
              </div>
            </div>
          )}

          <ScrollArea className="min-h-0 min-w-0 flex-1 bg-muted/10">
            <div className="min-w-max">
              <table className="table-fixed border-collapse">
              <colgroup>
                <col style={{ width: isLiveMode ? LIVE_COLUMN_WIDTH : SELECT_COLUMN_WIDTH }} />
                {schema.map((field) => (
                  <col
                    key={field.name}
                    style={{ width: columnWidth(field.name) }}
                  />
                ))}
              </colgroup>
              <thead className="sticky top-0 z-10">
                <tr>
                  {isLiveMode ? (
                    <th className="h-10 border-r border-border bg-muted/30 px-2 text-left align-middle">
                      <span className={chromeLabelClassName}>
                        Live
                      </span>
                    </th>
                  ) : (
                    <th className="h-10 border-r border-border bg-muted/30 px-2 text-left align-middle">
                      {canMutateRows ? (
                      <div className="flex items-center justify-center">
                        <input
                          type="checkbox"
                          checked={allVisibleRowsSelected}
                          onChange={(event) => {
                            setSelectedRows((previous) => {
                              const next = new Set(previous);
                              if (event.target.checked) {
                                currentPageRows.forEach((rowIndex) => next.add(rowIndex));
                              } else {
                                currentPageRows.forEach((rowIndex) => next.delete(rowIndex));
                              }
                              return next;
                            });
                          }}
                          className="h-3.5 w-3.5 rounded border-border bg-transparent disabled:opacity-40"
                        />
                      </div>
                      ) : (
                        <div className="h-3.5 w-3.5" />
                      )}
                    </th>
                  )}
                  {schema.map((field) => {
                    const isSorted = sortState?.columnName === field.name;
                    return (
                      <th
                        key={field.name}
                        className="relative h-10 border-r border-border bg-muted/30 px-0 text-left align-middle"
                      >
                        <button
                          type="button"
                          data-testid={`results-column-header-${field.name}`}
                          className="flex h-10 w-full min-w-0 items-center gap-1.5 px-2 text-left"
                          onClick={() => handleSortColumn(field.name)}
                        >
                          {field.isPrimaryKey && (
                            <KeyRound
                              className="h-3.5 w-3.5 shrink-0 text-amber-500"
                              aria-label="Primary key column"
                            />
                          )}
                          <span className="min-w-0 truncate text-xs font-semibold text-foreground">
                            {field.name}
                          </span>
                          {" "}
                          <span className="shrink-0 truncate font-mono text-[11px] font-normal text-muted-foreground">
                            {field.dataType}
                          </span>
                          <span className="ml-auto shrink-0 text-muted-foreground/70">
                            {isSorted ? (
                              sortState?.direction === "asc" ? (
                                <ArrowUp className="h-3.5 w-3.5 text-primary" />
                              ) : (
                                <ArrowDown className="h-3.5 w-3.5 text-primary" />
                              )
                            ) : (
                              <ChevronDown className="h-3.5 w-3.5" />
                            )}
                          </span>
                        </button>
                        <button
                          type="button"
                          aria-label={`Resize ${field.name}`}
                          title="Resize column"
                          onMouseDown={(event) => handleResizeColumn(event, field.name)}
                          className="absolute right-0 top-0 h-full w-1 cursor-col-resize touch-none border-r border-transparent hover:border-sky-500 hover:bg-sky-500/30"
                        />
                      </th>
                    );
                  })}
                </tr>
              </thead>

              <tbody>
                {currentPageRows.map((rowIndex) => {
                  const row = sourceRows[rowIndex] as RowData | undefined;
                  if (!row) {
                    return null;
                  }

                  const rowStatus = getRowStatus(rowIndex);
                  const rowSelected = selectedRows.has(rowIndex);

                  // Live change metadata
                  const liveChangeType = isLiveMode
                    ? (row[LIVE_META.CHANGE_TYPE] as string | undefined)
                    : undefined;
                  const liveChangedAt = isLiveMode
                    ? (row[LIVE_META.CHANGED_AT] as string | undefined)
                    : undefined;
                  const liveBatchNum = isLiveMode
                    ? (row[LIVE_META.BATCH_NUM] as string | number | undefined)
                    : undefined;
                  const liveChangeLabel = liveChangeType === "initial" && liveBatchNum
                    ? `initial #${liveBatchNum}`
                    : liveChangeType;
                  const liveChangedCols = isLiveMode && liveChangeType === "update"
                    ? new Set((row[LIVE_META.CHANGED_COLS] as string | undefined)?.split(",").filter(Boolean) ?? [])
                    : null;
                  // Determine if the change highlight is still fresh
                  const isRecentChange = (() => {
                    if (!liveChangedAt || !liveChangeType || liveChangeType === "initial") return false;
                    const elapsed = Date.now() - new Date(liveChangedAt).getTime();
                    return elapsed < LIVE_HIGHLIGHT_DURATION_MS;
                  })();
                  const isLiveDelete = isLiveMode && liveChangeType === "delete";
                  const isLiveInsert = isLiveMode && isRecentChange && liveChangeType === "insert";
                  const isLiveUpdate = isLiveMode && isRecentChange && liveChangeType === "update";

                  return (
                    <tr
                      key={rowIndex}
                      className={cn(
                        "border-b border-border transition-colors duration-500 ",
                        rowSelected && "bg-sky-500/10",
                        rowStatus === "edited" && "bg-amber-500/5",
                        rowStatus === "deleted" && "bg-red-500/10 opacity-60",
                        // Live mode row indicators
                        isLiveDelete && "bg-red-500/10 line-through opacity-60",
                        isLiveInsert && "bg-sky-500/10",
                        isLiveUpdate && "bg-amber-500/5",
                      )}
                    >
                      {isLiveMode ? (
                        <td className="border-r border-border bg-muted/25 px-2 py-1 align-middle">
                          <div className="flex min-h-[32px] min-w-[132px] items-center gap-2 whitespace-nowrap">
                            {liveChangedAt && (
                              <span className="shrink-0 text-[9px] text-muted-foreground">
                                {new Date(liveChangedAt).toLocaleTimeString()}
                              </span>
                            )}
                            {liveChangeLabel && (
                              <span
                                className={cn(
                                  "inline-flex w-fit items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase",
                                  liveChangeType === "initial" && "bg-sky-500/12 text-sky-700",
                                  liveChangeType === "insert" && "bg-sky-500/20 text-sky-700",
                                  liveChangeType === "update" && "bg-amber-500/20 text-amber-700",
                                  liveChangeType === "delete" && "bg-red-500/20 text-red-700",
                                )}
                              >
                                {liveChangeLabel}
                              </span>
                            )}
                          </div>
                        </td>
                      ) : (
                        <td className="border-r border-border bg-background px-2 py-1">
                          {canMutateRows ? (
                          <div className="flex items-center justify-center">
                            <input
                              type="checkbox"
                              checked={rowSelected}
                              onChange={(event) => {
                                setSelectedRows((previous) => {
                                  const next = new Set(previous);
                                  if (event.target.checked) {
                                    next.add(rowIndex);
                                  } else {
                                    next.delete(rowIndex);
                                  }
                                  return next;
                                });
                              }}
                              className="h-3.5 w-3.5 rounded border-border bg-transparent disabled:opacity-40"
                            />
                          </div>
                          ) : (
                            <div className="h-3.5 w-3.5" />
                          )}
                        </td>
                      )}

                      {schema.map((field) => {
                        const value = getCellEditedValue(rowIndex, field.name) ?? row[field.name];
                        const cellEdited = isCellEdited(rowIndex, field.name);
                        const cellKey = `${rowIndex}:${field.name}`;

                        return (
                          <td
                            key={`${rowIndex}-${field.name}`}
                            className={cn(
                              "border-r border-border bg-background px-0 py-0 align-middle",
                              selectedCellKey === cellKey && "ring-2 ring-inset ring-sky-500/80 bg-sky-500/10",
                            )}
                          >
                            <div
                              data-row-index={rowIndex}
                              data-column-name={field.name}
                              onMouseDownCapture={() => {
                                setSelectedCell({
                                  rowIndex,
                                  columnName: field.name,
                                });
                              }}
                              onClick={() => {
                                setSelectedCell({
                                  rowIndex,
                                  columnName: field.name,
                                });
                              }}
                              onDoubleClick={() => {
                                openCellViewer(value, field.name, rowIndex, canMutateRows);
                              }}
                              onContextMenu={(event) => {
                                event.preventDefault();
                                setSelectedCell({
                                  rowIndex,
                                  columnName: field.name,
                                });
                                setCellContextMenu({
                                  x: event.clientX,
                                  y: event.clientY,
                                  rowIndex,
                                  columnName: field.name,
                                  value,
                                  rowStatus,
                                  cellEdited,
                                  canMutate: canMutateRows,
                                });
                              }}
                              className={cn(
                                "h-7 truncate overflow-hidden whitespace-nowrap px-2 py-1 text-[11px] leading-4 outline-none transition-colors duration-500 [&_span]:inline-block [&_span]:max-w-full [&_span]:truncate [&_span]:align-middle [&_span]:text-[11px] [&_button]:max-w-full [&_button]:truncate [&_button]:text-[11px]",
                                value === null && "italic text-muted-foreground",
                                cellEdited && "bg-amber-500/20",
                                // Highlight changed cells during live updates
                                isLiveUpdate && liveChangedCols?.has(field.name) && "bg-amber-400/25 ring-1 ring-amber-400/40",
                              )}
                              tabIndex={0}
                            >
                              <CellDisplay
                                value={value}
                                dataType={field.dataType}
                                namespace={cellNamespace}
                                tableName={cellTableName}
                              />
                            </div>
                          </td>
                        );
                      })}
                    </tr>
                  );
                })}
              </tbody>
              </table>
            </div>
            <ScrollBar
              orientation="horizontal"
              className="data-[orientation=horizontal]:h-1.5 data-[orientation=horizontal]:p-0"
            />
          </ScrollArea>

          <div className="flex h-9 shrink-0 items-center border-t border-border bg-background px-2 text-[11px] text-muted-foreground">
            <div className="flex items-center gap-2">
              <Button
                size="icon"
                variant="outline"
                className="h-7 w-7"
                onClick={() => setPageIndex((previous) => Math.max(0, previous - 1))}
                disabled={pageIndex === 0}
              >
                <ChevronLeft className="h-3.5 w-3.5" />
              </Button>
              <span>Page</span>
              <Input
                aria-label="Results page"
                type="number"
                min={1}
                max={pageCount}
                value={pageIndex + 1}
                onChange={(event) => {
                  const parsedPage = Number(event.target.value);
                  if (!Number.isFinite(parsedPage)) {
                    return;
                  }
                  setPageIndex(Math.max(0, Math.min(pageCount - 1, parsedPage - 1)));
                }}
                className="h-7 w-14 text-center text-[11px]"
              />
              <span>of {pageCount}</span>
              <Button
                size="icon"
                variant="outline"
                className="h-7 w-7"
                onClick={() => setPageIndex((previous) => Math.min(pageCount - 1, previous + 1))}
                disabled={pageIndex >= pageCount - 1}
              >
                <ChevronRight className="h-3.5 w-3.5" />
              </Button>
              <Select
                value={String(pageSize)}
                onValueChange={(value) => {
                  setPageSize(Number(value));
                  setPageIndex(0);
                }}
              >
                <SelectTrigger className="h-7 w-[86px] text-[11px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {PAGE_SIZE_OPTIONS.map((option) => (
                    <SelectItem key={option} value={String(option)} className="text-[11px]">
                      {option} rows
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <span>{sortedRowIndices.length.toLocaleString()} records</span>
            </div>
          </div>

          <CellContextMenu
            context={cellContextMenu}
            onEdit={handleEditCell}
            onDelete={handleDeleteRow}
            onUndoEdit={undoRowEdits}
            onUndoDelete={undeleteRow}
            onViewData={(value) => {
              const columnName = cellContextMenu?.columnName ?? "value";
              const rowIndex = cellContextMenu?.rowIndex ?? 0;
              openCellViewer(value, columnName, rowIndex, true);
            }}
            onCopyValue={(value) => {
              navigator.clipboard.writeText(stringifyCellValue(value)).catch(console.error);
            }}
            onClose={() => setCellContextMenu(null)}
          />

          <InlineCellEditor
            context={inlineEditContext}
            onSave={handleSaveInlineEdit}
            onCancel={() => setInlineEditContext(null)}
          />

          <Dialog
            open={cellViewer.open}
            onOpenChange={(open) => {
              if (!open) {
                setCellViewer((prev) => ({ ...prev, open: false }));
              }
            }}
          >
            <DialogContent className="flex max-h-[85vh] max-w-4xl flex-col overflow-hidden">
              <DialogHeader className="shrink-0">
                <DialogTitle>{cellViewer.title}</DialogTitle>
              </DialogHeader>

              {cellViewer.canEdit ? (
                <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden">
                  {/* Null toggle */}
                  <label className="flex shrink-0 cursor-pointer items-center gap-2 text-sm text-muted-foreground">
                    <input
                      type="checkbox"
                      checked={cellViewer.isNull}
                      onChange={(e) => {
                        const checked = e.target.checked;
                        setCellViewer((prev) => ({
                          ...prev,
                          isNull: checked,
                          editedValue: checked ? "" : prev.editedValue,
                        }));
                      }}
                      className="h-4 w-4 rounded border-border bg-transparent"
                    />
                    Set to <span className="font-mono italic text-muted-foreground">NULL</span>
                  </label>

                  {/* Editor textarea or NULL placeholder */}
                  {cellViewer.isNull ? (
                    <div className="flex min-h-[120px] flex-1 items-center justify-center rounded-md border border-border bg-black font-mono text-sm italic text-slate-500">
                      NULL
                    </div>
                  ) : (
                    <>
                      <textarea
                        className="min-h-[120px] flex-1 resize-none rounded-md border border-border bg-black p-3 font-mono text-xs leading-5 text-slate-200 outline-none focus:border-sky-500 focus:ring-1 focus:ring-ring"
                        value={typeof cellViewer.editedValue === "string" ? cellViewer.editedValue : stringifyCellValue(cellViewer.editedValue)}
                        onChange={(e) => {
                          setCellViewer((prev) => ({ ...prev, editedValue: e.target.value }));
                        }}
                        spellCheck={false}
                      />
                      {/* Per-type validation message (live) */}
                      {(() => {
                        if (!cellViewer.dataType) return null;
                        const raw =
                          typeof cellViewer.editedValue === "string"
                            ? cellViewer.editedValue
                            : stringifyCellValue(cellViewer.editedValue);
                        const kind = classifyFieldKind(cellViewer.dataType);
                        const { error } = coerceFieldValue(raw, kind);
                        if (!error) return null;
                        return (
                          <div className="shrink-0 text-[11px] text-destructive">{error}</div>
                        );
                      })()}
                    </>
                  )}
                </div>
              ) : (
                <div className="min-h-0 flex-1 overflow-hidden">
                  <CodeBlock
                    value={cellViewer.content}
                    jsonPreferred={(cellViewer.dataType?.toLowerCase() ?? "").includes("json")}
                    maxHeightClassName="max-h-full h-full"
                  />
                </div>
              )}

              <DialogFooter className="shrink-0 border-t border-border pt-3">
                <Button
                  variant="ghost"
                  onClick={() => setCellViewer((prev) => ({ ...prev, open: false }))}
                >
                  Cancel
                </Button>
                {cellViewer.canEdit && (() => {
                  // Validate before allowing save
                  const raw = cellViewer.isNull
                    ? ""
                    : typeof cellViewer.editedValue === "string"
                      ? cellViewer.editedValue
                      : stringifyCellValue(cellViewer.editedValue);
                  const kind = cellViewer.dataType ? classifyFieldKind(cellViewer.dataType) : "text";
                  const validation = cellViewer.isNull ? { value: null, error: null } : coerceFieldValue(raw, kind);
                  const blocked = !cellViewer.isNull && validation.error !== null;
                  return (
                    <Button
                      disabled={blocked}
                      title={blocked ? validation.error ?? undefined : undefined}
                      onClick={() => {
                        const { rowIndex, columnName, content, isNull } = cellViewer;
                        if (rowIndex === undefined || !columnName) return;

                        let newValue: unknown;
                        if (isNull) {
                          newValue = null;
                        } else {
                          // Use the coerced typed value (number/boolean/etc) when validation passed.
                          // Falls back to the raw string for text/datetime where coerce returns the string.
                          newValue = validation.value;
                        }

                        const pkValues = getPrimaryKeyValues(rowIndex);
                        editCell(rowIndex, columnName, content, newValue, pkValues);
                        setCellViewer((prev) => ({ ...prev, open: false }));
                      }}
                    >
                      Save
                    </Button>
                  );
                })()}
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </>
      )}

      {insertTargetTable && (
        <InsertRowDialog
          open={showInsertRow}
          table={insertTargetTable}
          onSubmit={(values) => {
            setShowInsertRow(false);
            const fqn = `${insertTargetTable.namespace}.${insertTargetTable.name}`;
            const sql = generateInsertSql(insertTargetTable.namespace, insertTargetTable.name, values);
            openSqlPreview({
              sql,
              title: `Insert row into ${fqn}`,
              description: "Review the INSERT statement before committing.",
              onExecute: executeSqlPreviewStatement,
              onComplete: () => {
                notify({ title: `Inserted row into ${fqn}`, variant: "success" });
                onRefreshAfterCommit();
              },
            });
          }}
          onClose={() => setShowInsertRow(false)}
        />
      )}
    </div>
  );
}
