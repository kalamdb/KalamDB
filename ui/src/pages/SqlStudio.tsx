import { lazy, startTransition, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import {
  ChevronDown,
  Clock3,
  Copy,
  MoreHorizontal,
  PanelRightClose,
  PanelRightOpen,
  Play,
  RadioTower,
  Save,
  Settings2,
  Square,
  Trash2,
} from "lucide-react";
import { StudioExplorerPanel } from "@/components/sql-studio-v2/browser-tree/StudioExplorerPanel";
import { StudioTabsHeader } from "@/components/sql-studio-v2/studio-tabs/StudioTabsHeader";
import { EditorSidebar } from "@/components/sql-studio-v2/table-editor/EditorSidebar";
import { EditTableForm } from "@/components/sql-studio-v2/table-editor/EditTableForm";
import { isReadOnlyNamespace, tableToDraft } from "@/components/sql-studio-v2/table-editor/types";
import { generateCreateTableSql, generateDropTableSql } from "@/components/sql-studio-v2/table-editor/ddl-generator";
import { executeSqlPreviewStatement } from "@/components/sql-studio-v2/table-editor/run-sql";
import { startEditTable } from "@/features/sql-studio/state/editorTabSlice";
import { setActiveStudioTab } from "@/features/sql-studio/state/sqlStudioUiSlice";
import { useToast } from "@/components/ui/toaster-provider";
import { useSqlPreview } from "@/components/sql-preview";

import { QueryTabStrip } from "@/components/sql-studio-v2/input-form/QueryTabStrip";
import { EditorSkeleton } from "@/components/sql-studio-v2/input-form/EditorSkeleton";
const StudioEditorPanel = lazy(() =>
  import("@/components/sql-studio-v2/input-form/StudioEditorPanel").then((m) => ({
    default: m.StudioEditorPanel,
  })),
);
import { StudioInspectorPanel } from "@/components/sql-studio-v2/inspector/StudioInspectorPanel";
import { StudioResultsGrid } from "@/components/sql-studio-v2/preview/StudioResultsGrid";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import { useAuth } from "@/lib/auth";
import {
  subscribeWithHandle,
  setClientLogListener,
  setClientSendListener,
  setClientReceiveListener,
  setClientDisconnectListener,
  setClientErrorListener,
  type Unsubscribe,
} from "@/lib/kalam-client";
import type { ServerMessage, ChangeTypeRaw, SchemaField } from "@kalamdb/client";
import type {
  QueryRunSummary,
  QueryTab,
  SavedQuery,
  StudioTable,
} from "@/components/sql-studio-v2/shared/types";
import {
  executeSqlStudioQuery,
  normalizeSchema,
} from "@/services/sqlStudioService";
import {
  addWorkspaceTab,
  appendWorkspaceLiveRows,
  appendWorkspaceResultLog,
  closeWorkspaceTab,
  hydrateSqlStudioWorkspace,
  incrementWorkspaceTabUnreadChanges,
  prependWorkspaceHistory,
  setWorkspaceLiveSchema,
  setWorkspaceActiveTabId,
  setWorkspaceRunning,
  setWorkspaceSavedQueries,
  setWorkspaceTabResult,
  setWorkspaceTabs,
  updateWorkspaceTab,
} from "@/features/sql-studio/state/sqlStudioWorkspaceSlice";
import {
  hydrateSqlStudioUi,
  setHorizontalLayout,
  setInspectorCollapsed,
  setNamespaceExpanded,
  setSchemaFilter,
  setSelectedTableKey,
  setTableExpanded,
  setVerticalLayout,
  toggleFavoritesExpanded,
  toggleNamespaceSectionExpanded,
  toggleNamespaceExpanded,
  toggleTableExpanded,
} from "@/features/sql-studio/state/sqlStudioUiSlice";
import {
  selectExpandedNamespaces,
  selectExpandedTables,
  selectFavoritesExpanded,
  selectHorizontalLayout,
  selectIsInspectorCollapsed,
  selectNamespaceSectionExpanded,
  selectSchemaFilter,
  selectSelectedTableKey,
  selectVerticalLayout,
  selectWorkspaceActiveTabId,
  selectWorkspaceHistory,
  selectWorkspaceIsRunning,
  selectWorkspaceSavedQueries,
  selectWorkspaceTabResults,
  selectWorkspaceTabs,
  selectActiveStudioTab,
} from "@/features/sql-studio/state/selectors";
import { useGetSqlStudioSchemaTreeQuery } from "@/store/apiSlice";
import {
  buildSelectFromTableSql,
  createLogEntry,
  createQueryTab,
  createSavedQueryId,
  createTabId,
  resolveResultView,
  stripAutoSelectLimitForLiveSql,
} from "@/features/sql-studio/utils/workspaceHelpers";
import { ExplorerTableContextMenu } from "@/features/sql-studio/components/ExplorerTableContextMenu";
import {
  buildSyncedSqlStudioWorkspaceState,
  loadSyncedSqlStudioWorkspaceState,
  saveSyncedSqlStudioWorkspaceState,
  subscribeToSyncedSqlStudioWorkspaceState,
  type SqlStudioSyncedWorkspaceState,
} from "@/services/sqlStudioWorkspaceSyncService";

const WORKSPACE_PERSIST_DEBOUNCE_MS = 750;

type ExecuteMode = "all" | "selected";

interface LiveBatchState {
  hasMore: boolean;
  batchNum?: number;
  status?: string;
  subscriptionId?: string;
}

function normalizeLiveRows(rows: unknown[]): Record<string, unknown>[] {
  return rows.map((row) => {
    if (row instanceof Map) {
      return Object.fromEntries(row.entries()) as Record<string, unknown>;
    }
    if (typeof row === "object" && row !== null) {
      return row as Record<string, unknown>;
    }
    return { value: row };
  });
}

function extractWsMessageType(raw: string): string {
  try {
    const parsed = JSON.parse(raw);
    return typeof parsed?.type === "string" ? parsed.type : "unknown";
  } catch {
    return "raw";
  }
}

function extractWsSubscriptionId(raw: string): string | null {
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed?.subscription_id === "string") {
      return parsed.subscription_id;
    }
    return typeof parsed?.subscription?.id === "string" ? parsed.subscription.id : null;
  } catch {
    return null;
  }
}

function extractWsSubscriptionSql(raw: string): string | null {
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed?.subscription?.sql === "string") {
      return parsed.subscription.sql;
    }
    return typeof parsed?.sql === "string" ? parsed.sql : null;
  } catch {
    return null;
  }
}

function stripTrailingOptionsClause(sql: string): string {
  const cleanSql = sql.replace(/\s+OPTIONS\s*\([^)]+\)\s*;?\s*$/i, "").trim();
  return cleanSql.endsWith(";") ? cleanSql.slice(0, -1).trim() : cleanSql;
}

function displayBatchNum(batchNum: number | undefined): number | undefined {
  return Number.isFinite(batchNum) ? Number(batchNum) + 1 : undefined;
}

function formatLastSavedLabel(lastSavedAt: string | null): string {
  if (!lastSavedAt) {
    return "Never saved";
  }

  const savedAt = new Date(lastSavedAt);
  const diffMs = Date.now() - savedAt.getTime();
  if (Number.isNaN(savedAt.getTime()) || diffMs < 0) {
    return "Saved just now";
  }

  const diffMinutes = Math.floor(diffMs / (1000 * 60));
  if (diffMinutes < 1) {
    return "Saved just now";
  }
  if (diffMinutes < 60) {
    return `Saved ${diffMinutes}m ago`;
  }

  const diffHours = Math.floor(diffMinutes / 60);
  if (diffHours < 24) {
    return `Saved ${diffHours}h ago`;
  }

  return `Saved ${savedAt.toLocaleDateString()}`;
}

function isWsErrorFrame(messageType: string): boolean {
  return messageType === "error" || messageType === "auth_error";
}

function extractMessage(value: unknown, fallback: string): string {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "object" && value !== null && "message" in value) {
    const message = (value as { message?: unknown }).message;
    if (typeof message === "string") {
      return message;
    }
  }
  return fallback;
}

function mapPersistedTabToQueryTab(tab: SqlStudioSyncedWorkspaceState["tabs"][number]): QueryTab {
  return {
    id: tab.id,
    title: tab.name,
    sql: tab.query,
    isDirty: tab.settings.isDirty,
    unreadChangeCount: 0,
    isLive: tab.settings.isLive,
    liveStatus: tab.settings.liveStatus,
    resultView: tab.settings.resultView,
    lastSavedAt: tab.settings.lastSavedAt,
    savedQueryId: tab.settings.savedQueryId,
    subscriptionOptions: tab.settings.subscriptionOptions,
  };
}

function mapPersistedSavedQuery(item: SqlStudioSyncedWorkspaceState["savedQueries"][number]): SavedQuery {
  return {
    id: item.id,
    title: item.title,
    sql: item.sql,
    lastSavedAt: item.lastSavedAt,
    isLive: item.isLive,
    subscriptionOptions: item.subscriptionOptions,
  };
}

function serializeSyncedWorkspaceSnapshot(state: SqlStudioSyncedWorkspaceState): string {
  return JSON.stringify({
    ...state,
    updatedAt: "",
  });
}

function containsCreateTableStatement(sql: string): boolean {
  const normalized = sql
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/--[^\n\r]*/g, " ")
    .replace(/\s+/g, " ")
    .trim();

  return /(^|;)\s*create\s+(?:or\s+replace\s+)?(?:temp(?:orary)?\s+)?table\b/i.test(normalized);
}

export default function SqlStudio() {
  const location = useLocation();
  const navigate = useNavigate();

  const dispatch = useAppDispatch();
  const { user } = useAuth();
  const tabs = useAppSelector(selectWorkspaceTabs);
  const savedQueries = useAppSelector(selectWorkspaceSavedQueries);
  const activeTabId = useAppSelector(selectWorkspaceActiveTabId);
  const {
    data: schema = [],
    isFetching: isSchemaRefreshing,
    refetch: refetchSchemaTree,
  } = useGetSqlStudioSchemaTreeQuery();
  const schemaFilter = useAppSelector(selectSchemaFilter);
  const favoritesExpanded = useAppSelector(selectFavoritesExpanded);
  const namespaceSectionExpanded = useAppSelector(selectNamespaceSectionExpanded);
  const expandedNamespaces = useAppSelector(selectExpandedNamespaces);
  const expandedTables = useAppSelector(selectExpandedTables);
  const selectedTableKey = useAppSelector(selectSelectedTableKey);
  const isRunning = useAppSelector(selectWorkspaceIsRunning);
  const tabResults = useAppSelector(selectWorkspaceTabResults);
  const history = useAppSelector(selectWorkspaceHistory);
  const isInspectorCollapsed = useAppSelector(selectIsInspectorCollapsed);
  const horizontalLayout = useAppSelector(selectHorizontalLayout);
  const verticalLayout = useAppSelector(selectVerticalLayout);
  const activeStudioTab = useAppSelector(selectActiveStudioTab);
  const [explorerContextMenu, setExplorerContextMenu] = useState<{
    x: number;
    y: number;
    table: StudioTable;
  } | null>(null);
  const { notify } = useToast();
  const { openSqlPreview } = useSqlPreview();
  const [isUiHydrated, setIsUiHydrated] = useState(false);
  const [isRemoteWorkspaceHydrated, setIsRemoteWorkspaceHydrated] = useState(false);
  const [liveBatchByTab, setLiveBatchByTab] = useState<Record<string, LiveBatchState>>({});
  const [selectedEditorSql, setSelectedEditorSql] = useState("");
  const [showSubscriptionOptions, setShowSubscriptionOptions] = useState(false);
  const liveUnsubscribeRef = useRef<Record<string, Unsubscribe>>({});
  const liveRequestNextBatchRef = useRef<Record<string, () => Promise<void>>>({});
  const liveGenRef = useRef<Record<string, number>>({});
  const liveSubscriptionIdRef = useRef<Record<string, string>>({});
  const consumedPrefillKeyRef = useRef<string | null>(null);
  const activeTabIdRef = useRef<string | null>(null);
  const tabsRef = useRef<QueryTab[]>(tabs);
  const lastSyncedWorkspaceSnapshotRef = useRef<string | null>(null);

  const activeTab = useMemo(
    () => {
      if (tabs.length === 0) {
        return null;
      }
      return tabs.find((item) => item.id === activeTabId) ?? tabs[0];
    },
    [tabs, activeTabId],
  );
  const activeResult = activeTab ? (tabResults[activeTab.id] ?? null) : null;
  const activeLiveBatch = activeTab ? liveBatchByTab[activeTab.id] : undefined;
  const selectedTable = useMemo(() => {
    if (!selectedTableKey) {
      return null;
    }
    const [namespaceName, tableName] = selectedTableKey.split(".");
    const namespace = schema.find((item) => item.name === namespaceName);
    return namespace?.tables.find((item) => item.name === tableName) ?? null;
  }, [schema, selectedTableKey]);

  useEffect(() => {
    tabsRef.current = tabs;
  }, [tabs]);

  useEffect(() => {
    activeTabIdRef.current = activeTabId;
  }, [activeTabId]);

  useEffect(() => {
    setSelectedEditorSql("");
    setShowSubscriptionOptions(false);
  }, [activeTab?.id]);

  const applySyncedWorkspace = useCallback((workspace: SqlStudioSyncedWorkspaceState) => {
    dispatch(hydrateSqlStudioWorkspace({
      tabs: workspace.tabs.map(mapPersistedTabToQueryTab),
      savedQueries: workspace.savedQueries.map(mapPersistedSavedQuery),
      activeTabId: workspace.activeTabId,
    }));
  }, [dispatch]);

  useEffect(() => {
    const username = user?.username?.trim();
    if (!username) {
      setIsUiHydrated(false);
      setIsRemoteWorkspaceHydrated(false);
      lastSyncedWorkspaceSnapshotRef.current = null;
      return;
    }

    setIsUiHydrated(false);
    setIsRemoteWorkspaceHydrated(false);
    lastSyncedWorkspaceSnapshotRef.current = null;

    let cancelled = false;
    let remoteUnsubscribe: Unsubscribe | null = null;

    const hydrateBlankWorkspace = () => {
      if (tabsRef.current.length > 0) {
        return;
      }

      const blankTab = createQueryTab(1);
      dispatch(hydrateSqlStudioWorkspace({
        tabs: [blankTab],
        savedQueries: [],
        activeTabId: blankTab.id,
      }));
    };

    void (async () => {
      try {
        const initialWorkspace = await loadSyncedSqlStudioWorkspaceState(username);
        if (cancelled) {
          return;
        }

        if (initialWorkspace) {
          lastSyncedWorkspaceSnapshotRef.current = serializeSyncedWorkspaceSnapshot(initialWorkspace);
          applySyncedWorkspace(initialWorkspace);
        } else {
          hydrateBlankWorkspace();
        }

        setIsUiHydrated(true);

        remoteUnsubscribe = await subscribeToSyncedSqlStudioWorkspaceState(username, (nextWorkspace) => {
          if (cancelled) {
            return;
          }

          if (!nextWorkspace) {
            return;
          }

          const nextSnapshot = serializeSyncedWorkspaceSnapshot(nextWorkspace);
          if (nextSnapshot === lastSyncedWorkspaceSnapshotRef.current) {
            return;
          }

          lastSyncedWorkspaceSnapshotRef.current = nextSnapshot;
          applySyncedWorkspace(nextWorkspace);
        });

        if (!cancelled) {
          setIsRemoteWorkspaceHydrated(true);
        }

        if (cancelled && remoteUnsubscribe) {
          void remoteUnsubscribe();
          remoteUnsubscribe = null;
        }
      } catch (error) {
        if (!cancelled) {
          console.error("Failed to initialize synced SQL Studio workspace subscription", error);
          hydrateBlankWorkspace();
          setIsUiHydrated(true);
          setIsRemoteWorkspaceHydrated(true);
        }
      }
    })();

    return () => {
      cancelled = true;
      if (remoteUnsubscribe) {
        void remoteUnsubscribe();
      }
    };
  }, [applySyncedWorkspace, dispatch, user?.username]);

  useEffect(() => {
    if (!selectedTableKey && schema.length > 0 && schema[0].tables.length > 0) {
      const firstTable = schema[0].tables[0];
      dispatch(setSelectedTableKey(`${firstTable.namespace}.${firstTable.name}`));
    }
  }, [dispatch, schema, selectedTableKey]);

  useEffect(() => {
    if (schema.length === 0 || Object.keys(expandedNamespaces).length > 0) {
      return;
    }

    const initialExpanded: Record<string, boolean> = {};
    schema.slice(0, 3).forEach((namespace) => {
      initialExpanded[namespace.name] = true;
    });
    dispatch(hydrateSqlStudioUi({ expandedNamespaces: initialExpanded }));
  }, [dispatch, schema, expandedNamespaces]);

  useEffect(() => {
    if (!selectedTableKey) {
      return;
    }

    const [namespaceName] = selectedTableKey.split(".");
    dispatch(setNamespaceExpanded({ namespaceName, expanded: true }));
    dispatch(setTableExpanded({ tableKey: selectedTableKey, expanded: true }));
  }, [dispatch, selectedTableKey]);

  useEffect(() => {
    if (!isUiHydrated) {
      return;
    }

    const state = location.state as { prefillSql?: string; prefillTitle?: string } | null;
    const prefillSql = state?.prefillSql?.trim();
    if (!prefillSql) {
      return;
    }

    const prefillKey = `${location.key}:${prefillSql}`;
    if (consumedPrefillKeyRef.current === prefillKey) {
      return;
    }
    consumedPrefillKeyRef.current = prefillKey;

    const tab: QueryTab = {
      id: createTabId(tabs.length + 1),
      title: state?.prefillTitle?.trim() || `Query ${tabs.length + 1}`,
      sql: prefillSql,
      isDirty: true,
      unreadChangeCount: 0,
      isLive: false,
      liveStatus: "idle",
      resultView: "results",
      lastSavedAt: null,
      savedQueryId: null,
    };
    dispatch(addWorkspaceTab(tab));
    dispatch(setWorkspaceActiveTabId(tab.id));
    navigate(location.pathname, { replace: true, state: null });
  }, [dispatch, isUiHydrated, location.key, location.pathname, location.state, navigate, tabs.length]);

  const updateTab = useCallback((tabId: string, updates: Partial<QueryTab>) => {
    dispatch(updateWorkspaceTab({ tabId, updates }));
  }, [dispatch]);

  const updateActiveTab = useCallback((updates: Partial<QueryTab>) => {
    if (!activeTab) {
      return;
    }
    updateTab(activeTab.id, updates);
  }, [activeTab, updateTab]);

  const refreshExplorerSchema = useCallback(async () => {
    await refetchSchemaTree();
  }, [refetchSchemaTree]);

  const addTab = useCallback(() => {
    const nextIndex = tabs.length + 1;
    const tab = createQueryTab(nextIndex);
    dispatch(addWorkspaceTab(tab));
    dispatch(setWorkspaceActiveTabId(tab.id));
  }, [dispatch, tabs.length]);

  const cleanupLiveSubscription = useCallback((tabId: string) => {
    const unsubscribe = liveUnsubscribeRef.current[tabId];
    if (unsubscribe) {
      void unsubscribe();
      delete liveUnsubscribeRef.current[tabId];
      delete liveRequestNextBatchRef.current[tabId];
      delete liveSubscriptionIdRef.current[tabId];
    }
    setLiveBatchByTab((previous) => {
      if (!(tabId in previous)) {
        return previous;
      }
      const next = { ...previous };
      delete next[tabId];
      return next;
    });
  }, []);

  const clearWsListeners = useCallback(() => {
    setClientLogListener(undefined);
    setClientSendListener(undefined);
    setClientReceiveListener(undefined);
    setClientDisconnectListener(undefined);
    setClientErrorListener(undefined);
  }, []);

  const closeTab = useCallback((tabId: string) => {
    if (tabs.length === 1) {
      return;
    }

    liveGenRef.current[tabId] = (liveGenRef.current[tabId] ?? 0) + 1;
    cleanupLiveSubscription(tabId);

    dispatch(closeWorkspaceTab(tabId));
  }, [cleanupLiveSubscription, dispatch, tabs]);

  const openQueryInNewTab = (query: string, title: string) => {
    const tab: QueryTab = {
      id: createTabId(tabs.length + 1),
      title,
      sql: query,
      isDirty: true,
      unreadChangeCount: 0,
      isLive: false,
      liveStatus: "idle",
      resultView: "results",
      lastSavedAt: null,
      savedQueryId: null,
    };
    dispatch(addWorkspaceTab(tab));
    dispatch(setWorkspaceActiveTabId(tab.id));
  };

  const executeQueryForTab = async (tabId: string, sql: string, tabTitle: string) => {
    if (!sql.trim()) {
      return;
    }

    dispatch(setWorkspaceRunning(true));
    const startedAt = Date.now();
    const shouldRefreshExplorer = containsCreateTableStatement(sql);

    // Safety net: always unblock the Execute button after 60 s even if the
    // underlying WASM call or network request stalls and never settles.
    const safetyTimeoutId = window.setTimeout(() => {
      dispatch(setWorkspaceRunning(false));
      dispatch(setWorkspaceTabResult({ tabId, result: {
        status: "error",
        rows: [],
        schema: [],
        tookMs: Math.round(Date.now() - startedAt),
        rowCount: 0,
        logs: [createLogEntry("Query timed out after 60 seconds.", "error", user?.username)],
        errorMessage: "Query timed out after 60 seconds.",
      } }));
      updateTab(tabId, { resultView: "log" });
    }, 60_000);

    try {
      const queryResult = await executeSqlStudioQuery(sql);
      const durationMs = Math.round(Date.now() - startedAt);
      const historyEntry: QueryRunSummary = {
        id: `${tabId}-${startedAt}`,
        tabTitle,
        sql,
        status: queryResult.status,
        executedAt: new Date().toISOString(),
        durationMs,
        rowCount: queryResult.rowCount,
        errorMessage: queryResult.errorMessage,
      };

      startTransition(() => {
        dispatch(setWorkspaceTabResult({ tabId, result: queryResult }));
        dispatch(prependWorkspaceHistory(historyEntry));
        updateTab(tabId, {
          isDirty: false,
          resultView: resolveResultView(queryResult),
        });
      });

      if (queryResult.status === "success" && shouldRefreshExplorer) {
        void refreshExplorerSchema();
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : "Query execution failed";
      dispatch(setWorkspaceTabResult({ tabId, result: {
        status: "error",
        rows: [],
        schema: [],
        tookMs: 0,
        rowCount: 0,
        logs: [createLogEntry(message, "error", user?.username, error)],
        errorMessage: message,
      } }));
      updateTab(tabId, { resultView: "log" });
    } finally {
      window.clearTimeout(safetyTimeoutId);
      dispatch(setWorkspaceRunning(false));
    }
  };

  const stopLiveQuery = useCallback((tabId: string) => {
    liveGenRef.current[tabId] = (liveGenRef.current[tabId] ?? 0) + 1;
    cleanupLiveSubscription(tabId);
    clearWsListeners();
    dispatch(setWorkspaceRunning(false));
    updateTab(tabId, { liveStatus: "idle" });
  }, [cleanupLiveSubscription, clearWsListeners, dispatch, updateTab]);

  const fetchNextLiveBatch = useCallback(async (tabId: string) => {
    const requestNextBatch = liveRequestNextBatchRef.current[tabId];
    if (!requestNextBatch) {
      return;
    }

    try {
      await requestNextBatch();
    } catch (error) {
      dispatch(appendWorkspaceResultLog({
        tabId,
        entry: createLogEntry(
          error instanceof Error ? error.message : "Failed to fetch next live batch",
          "error",
          user?.username,
          error,
        ),
        statusOverride: "error",
      }));
    }
  }, [dispatch, user?.username]);

  const startLiveQuery = useCallback(async (tab: QueryTab, sqlOverride?: string) => {
    const sqlToRun = stripAutoSelectLimitForLiveSql(sqlOverride ?? tab.sql);
    const tracedSubscribeSql = stripTrailingOptionsClause(sqlToRun);

    if (!sqlToRun.trim()) {
      return;
    }

    // Cancel any existing subscription for this tab
    liveGenRef.current[tab.id] = (liveGenRef.current[tab.id] ?? 0) + 1;
    cleanupLiveSubscription(tab.id);

    const gen = liveGenRef.current[tab.id];

    dispatch(setWorkspaceRunning(true));
    dispatch(setWorkspaceTabResult({
      tabId: tab.id,
      result: {
        status: "success",
        rows: [],
        schema: [],
        tookMs: 0,
        rowCount: 0,
        logs: [],
      },
    }));
    updateTab(tab.id, { liveStatus: "connecting", isLive: true });

    // Wire up WS message tracing — every frame in/out appears in the log panel
    setClientSendListener((message: string) => {
      if (liveGenRef.current[tab.id] !== gen) return;
      const messageType = extractWsMessageType(message);
      const messageSql = extractWsSubscriptionSql(message);
      if (
        messageType === "subscribe"
        && messageSql
        && stripTrailingOptionsClause(messageSql) !== tracedSubscribeSql
      ) return;
      const msgSubId = extractWsSubscriptionId(message);
      const tabSubId = liveSubscriptionIdRef.current[tab.id];
      if (msgSubId !== null && tabSubId !== undefined && msgSubId !== tabSubId) return;
      dispatch(appendWorkspaceResultLog({
        tabId: tab.id,
        entry: createLogEntry(
          `WS SEND · ${messageType}`,
          "info",
          user?.username,
          { raw: message },
        ),
      }));
    });

    setClientReceiveListener((message: string) => {
      if (liveGenRef.current[tab.id] !== gen) return;
      // Only log frames that belong to this tab's subscription (or have no
      // subscription_id — e.g. auth, ping). This prevents the log panel from
      // showing raw frames from other concurrent subscriptions (e.g. the
      // internal workspace-sync subscription on the same WebSocket connection).
      const msgSubId = extractWsSubscriptionId(message);
      const tabSubId = liveSubscriptionIdRef.current[tab.id];
      if (msgSubId !== null && tabSubId !== undefined && msgSubId !== tabSubId) return;
      const messageType = extractWsMessageType(message);
      const isErrorFrame = isWsErrorFrame(messageType);
      dispatch(appendWorkspaceResultLog({
        tabId: tab.id,
        entry: createLogEntry(
          `WS RECEIVE · ${messageType}`,
          isErrorFrame ? "error" : "info",
          user?.username,
          { raw: message },
        ),
        statusOverride: isErrorFrame ? "error" : undefined,
      }));
    });

    setClientDisconnectListener((reason) => {
      if (liveGenRef.current[tab.id] !== gen) return;
      dispatch(appendWorkspaceResultLog({
        tabId: tab.id,
        entry: createLogEntry(
          `WebSocket disconnected: ${extractMessage(reason, "unknown reason")}`,
          "error",
          user?.username,
          reason,
        ),
        statusOverride: "error",
      }));
      updateTab(tab.id, { liveStatus: "error" });
    });

    setClientErrorListener((error) => {
      if (liveGenRef.current[tab.id] !== gen) return;
      dispatch(appendWorkspaceResultLog({
        tabId: tab.id,
        entry: createLogEntry(
          `Connection error: ${extractMessage(error, "unknown error")}`,
          "error",
          user?.username,
          error,
        ),
      }));
    });

    setClientLogListener(undefined);

    let hasConnected = false;

    try {
      const handle = await subscribeWithHandle(sqlToRun, (msg: ServerMessage) => {
        if (liveGenRef.current[tab.id] !== gen) return;

        switch (msg.type) {
          case "error": {
            updateTab(tab.id, { liveStatus: "error" });
            dispatch(setWorkspaceRunning(false));
            return;
          }

          case "auth_success": {
            return;
          }

          case "auth_error": {
            updateTab(tab.id, { liveStatus: "error" });
            dispatch(setWorkspaceRunning(false));
            return;
          }

          case "subscription_ack": {
            hasConnected = true;
            // Record this tab's subscription ID so the receive listener can
            // filter out frames from other concurrent subscriptions.
            liveSubscriptionIdRef.current[tab.id] = msg.subscription_id;
            setLiveBatchByTab((previous) => ({
              ...previous,
              [tab.id]: {
                hasMore: msg.batch_control.has_more,
                batchNum: displayBatchNum(msg.batch_control.batch_num),
                status: msg.batch_control.status,
                subscriptionId: msg.subscription_id,
              },
            }));
            if (msg.schema.length > 0) {
              dispatch(setWorkspaceLiveSchema({
                tabId: tab.id,
                schema: normalizeSchema(msg.schema as SchemaField[]),
              }));
            }
            updateTab(tab.id, { liveStatus: "connected", resultView: "results" });
            dispatch(setWorkspaceRunning(false));
            return;
          }

          case "initial_data_batch": {
            if (!hasConnected) {
              hasConnected = true;
            }
            setLiveBatchByTab((previous) => ({
              ...previous,
              [tab.id]: {
                hasMore: msg.batch_control.has_more,
                batchNum: displayBatchNum(msg.batch_control.batch_num),
                status: msg.batch_control.status,
                subscriptionId: msg.subscription_id,
              },
            }));
            const rows = normalizeLiveRows(msg.rows);
            dispatch(appendWorkspaceLiveRows({
              tabId: tab.id,
              rows,
              changeType: "initial",
              schema: undefined,
              batchNum: displayBatchNum(msg.batch_control.batch_num),
            }));
            updateTab(tab.id, { liveStatus: "connected", resultView: "results" });
            dispatch(setWorkspaceRunning(false));
            return;
          }

          case "change": {
            const changeType: ChangeTypeRaw = msg.change_type;
            let changedRowCount = 1;

            if (changeType === "delete") {
              const deletedRows = normalizeLiveRows(msg.old_values ?? []);
              changedRowCount = Math.max(1, deletedRows.length);
              dispatch(appendWorkspaceLiveRows({
                tabId: tab.id,
                rows: deletedRows,
                changeType: "delete",
              }));
            } else if (changeType === "update") {
              const updatedRows = normalizeLiveRows(msg.rows ?? []);
              changedRowCount = Math.max(1, updatedRows.length);
              dispatch(appendWorkspaceLiveRows({
                tabId: tab.id,
                rows: updatedRows,
                changeType: "update",
              }));
            } else {
              const insertedRows = normalizeLiveRows(msg.rows ?? []);
              changedRowCount = Math.max(1, insertedRows.length);
              dispatch(appendWorkspaceLiveRows({
                tabId: tab.id,
                rows: insertedRows,
                changeType: "insert",
              }));
            }
            if (activeTabIdRef.current !== tab.id) {
              dispatch(incrementWorkspaceTabUnreadChanges({
                tabId: tab.id,
                amount: changedRowCount,
              }));
            }
            updateTab(tab.id, { liveStatus: "connected", resultView: "results" });
            return;
          }

          default:
            break;
        }
      }, tab.subscriptionOptions);

      // If cancelled while awaiting subscribe, unsubscribe immediately
      if (liveGenRef.current[tab.id] !== gen) {
        void handle.unsubscribe();
        return;
      }

      liveUnsubscribeRef.current[tab.id] = handle.unsubscribe;
      liveRequestNextBatchRef.current[tab.id] = handle.requestNextBatch;
      liveSubscriptionIdRef.current[tab.id] = handle.id;
    } catch (error) {
      if (liveGenRef.current[tab.id] !== gen) return;

      console.error("Failed to subscribe to live query", error);
      dispatch(appendWorkspaceResultLog({
        tabId: tab.id,
        entry: createLogEntry(
          error instanceof Error ? error.message : "Failed to subscribe to live query",
          "error",
          user?.username,
          error,
        ),
        statusOverride: "error",
      }));
      updateTab(tab.id, { liveStatus: "error" });
      dispatch(setWorkspaceRunning(false));
    }
  }, [cleanupLiveSubscription, dispatch, updateTab, user?.username]);

  const runActiveQuery = async (sqlToRun: string) => {
    if (!activeTab) {
      return;
    }

    if (activeTab.isLive) {
      if (activeTab.liveStatus === "connected" || activeTab.liveStatus === "connecting") {
        stopLiveQuery(activeTab.id);
      } else {
        await startLiveQuery(activeTab, sqlToRun);
      }
      return;
    }

    await executeQueryForTab(activeTab.id, sqlToRun, activeTab.title);
  };

  const saveTab = useCallback((tabId: string, openAsCopy: boolean) => {
    const tab = tabs.find((item) => item.id === tabId);
    if (!tab) {
      return;
    }

    const nowIso = new Date().toISOString();
    const saveId = openAsCopy || !tab.savedQueryId ? createSavedQueryId() : tab.savedQueryId;
    const saveTitle = openAsCopy ? `${tab.title} Copy` : tab.title;

    const nextSavedQueries = (() => {
      const previous = savedQueries;
      const existing = previous.find((item) => item.id === saveId);
      if (existing) {
        return previous.map((item) =>
          item.id === saveId
            ? { ...item, title: saveTitle, sql: tab.sql, isLive: tab.isLive, lastSavedAt: nowIso, subscriptionOptions: tab.subscriptionOptions }
            : item,
        );
      }
      return [
        {
          id: saveId,
          title: saveTitle,
          sql: tab.sql,
          isLive: tab.isLive,
          lastSavedAt: nowIso,
          subscriptionOptions: tab.subscriptionOptions,
        },
        ...previous,
      ];
    })();
    dispatch(setWorkspaceSavedQueries(nextSavedQueries));

    if (openAsCopy) {
      const copiedTab: QueryTab = {
        ...tab,
        id: createTabId(tabs.length + 1),
        title: saveTitle,
        isDirty: false,
        unreadChangeCount: 0,
        savedQueryId: saveId,
        lastSavedAt: nowIso,
        liveStatus: "idle",
        resultView: tab.resultView,
      };
      dispatch(addWorkspaceTab(copiedTab));
      dispatch(setWorkspaceActiveTabId(copiedTab.id));
      if (tabResults[tab.id]) {
        dispatch(setWorkspaceTabResult({ tabId: copiedTab.id, result: tabResults[tab.id] ?? null }));
      }
      return;
    }

    updateTab(tab.id, {
      savedQueryId: saveId,
      lastSavedAt: nowIso,
      isDirty: false,
    });
  }, [dispatch, savedQueries, tabs, tabResults, updateTab]);

  const saveActiveTab = useCallback(() => {
    if (!activeTab) {
      return;
    }

    saveTab(activeTab.id, false);
  }, [activeTab, saveTab]);

  useEffect(() => {
    const handleGlobalShortcut = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat) {
        return;
      }

      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey) {
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "t") {
        event.preventDefault();
        event.stopPropagation();
        addTab();
        return;
      }

      if (key === "s") {
        if (!activeTab) {
          return;
        }

        event.preventDefault();
        event.stopPropagation();
        saveActiveTab();
      }
    };

    window.addEventListener("keydown", handleGlobalShortcut, true);
    return () => {
      window.removeEventListener("keydown", handleGlobalShortcut, true);
    };
  }, [activeTab, addTab, saveActiveTab]);

  const renameTab = useCallback((tabId: string, title: string) => {
    const tab = tabs.find((item) => item.id === tabId);
    if (!tab) {
      return;
    }

    updateTab(tab.id, { title });
    if (tab.savedQueryId) {
      dispatch(setWorkspaceSavedQueries(
        savedQueries.map((item) =>
          item.id === tab.savedQueryId ? { ...item, title } : item,
        ),
      ));
    }
  }, [dispatch, savedQueries, tabs, updateTab]);

  const deleteActiveTab = useCallback(() => {
    if (!activeTab) {
      return;
    }

    const deletedSavedQueryId = activeTab.savedQueryId;
    closeTab(activeTab.id);
    if (deletedSavedQueryId) {
      dispatch(setWorkspaceSavedQueries(
        savedQueries.filter((item) => item.id !== deletedSavedQueryId),
      ));
      dispatch(setWorkspaceTabs(
        tabs.map((item) =>
          item.savedQueryId === deletedSavedQueryId
            ? { ...item, savedQueryId: null }
            : item,
        ),
      ));
    }
  }, [activeTab, closeTab, dispatch, savedQueries, tabs]);

  const openSavedQuery = useCallback((queryId: string) => {
    const savedQuery = savedQueries.find((item) => item.id === queryId);
    if (!savedQuery) {
      return;
    }

    const existingTab = tabs.find((item) => item.savedQueryId === queryId);
    if (existingTab) {
      dispatch(setWorkspaceActiveTabId(existingTab.id));
      return;
    }

    const tab: QueryTab = {
      id: createTabId(tabs.length + 1),
      title: savedQuery.title,
      sql: savedQuery.sql,
      isDirty: false,
      unreadChangeCount: 0,
      isLive: savedQuery.isLive,
      liveStatus: "idle",
      resultView: "results",
      lastSavedAt: savedQuery.lastSavedAt,
      savedQueryId: savedQuery.id,
      subscriptionOptions: savedQuery.subscriptionOptions,
    };
    dispatch(addWorkspaceTab(tab));
    dispatch(setWorkspaceActiveTabId(tab.id));
  }, [dispatch, savedQueries, tabs]);

  const toggleInspector = () => {
    if (isInspectorCollapsed) {
      dispatch(setInspectorCollapsed(false));
      return;
    }
    dispatch(setInspectorCollapsed(true));
  };

  useEffect(() => {
    if (!explorerContextMenu) {
      return;
    }

    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setExplorerContextMenu(null);
      }
    };
    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [explorerContextMenu]);

  useEffect(() => {
    return () => {
      for (const tabId of Object.keys(liveGenRef.current)) {
        liveGenRef.current[tabId] = (liveGenRef.current[tabId] ?? 0) + 1;
      }
      Object.values(liveUnsubscribeRef.current).forEach((unsubscribe) => unsubscribe());
      liveUnsubscribeRef.current = {};
      setClientLogListener(undefined);
      setClientSendListener(undefined);
      setClientReceiveListener(undefined);
      setClientDisconnectListener(undefined);
      setClientErrorListener(undefined);
    };
  }, []);

  useEffect(() => {
    if (!isUiHydrated || !isRemoteWorkspaceHydrated || tabs.length === 0) {
      return;
    }

    const nextRemoteState = buildSyncedSqlStudioWorkspaceState(tabs, savedQueries, activeTabId);
    const nextSnapshot = serializeSyncedWorkspaceSnapshot(nextRemoteState);
    if (nextSnapshot === lastSyncedWorkspaceSnapshotRef.current) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      void (async () => {
        try {
          await saveSyncedSqlStudioWorkspaceState(nextRemoteState, user?.username);
          lastSyncedWorkspaceSnapshotRef.current = nextSnapshot;
        } catch (error) {
          console.error("Failed to save synced SQL Studio workspace", error);
        }
      })();
    }, WORKSPACE_PERSIST_DEBOUNCE_MS);

    return () => window.clearTimeout(timeoutId);
  }, [
    activeTabId,
    isRemoteWorkspaceHydrated,
    isUiHydrated,
    savedQueries,
    tabs,
    user?.username,
  ]);

  const hasSelectedSql = selectedEditorSql.trim().length > 0;
  const executeLabel = hasSelectedSql ? "Run selected" : "Run";
  const isConnectingLive = activeTab?.isLive && activeTab.liveStatus === "connecting";
  const isExecuteDisabled = !activeTab
    || (activeTab.isLive ? !activeTab.sql.trim() : isRunning || !activeTab.sql.trim());
  const lastSavedLabel = formatLastSavedLabel(activeTab?.lastSavedAt ?? null);

  const executeFromToolbar = (mode: ExecuteMode | "auto") => {
    if (!activeTab) {
      return;
    }

    const resolvedMode: ExecuteMode = mode === "auto"
      ? (hasSelectedSql ? "selected" : "all")
      : mode;
    const sqlToRun = resolvedMode === "selected" ? selectedEditorSql : activeTab.sql;
    if (!sqlToRun.trim()) {
      return;
    }

    void runActiveQuery(sqlToRun);
  };

  const toggleActiveTabLive = (checked: boolean) => {
    if (!activeTab) {
      return;
    }

    if (!checked && (activeTab.liveStatus === "connected" || activeTab.liveStatus === "connecting")) {
      stopLiveQuery(activeTab.id);
    }
    const nextSql = checked
      ? stripAutoSelectLimitForLiveSql(activeTab.sql)
      : activeTab.sql;
    updateActiveTab({ isLive: checked, liveStatus: "idle", sql: nextSql });
    if (!checked) {
      setShowSubscriptionOptions(false);
    }
  };

  const sqlResultActions = activeTab ? (
    <>
      <div className="hidden items-center gap-2 lg:flex">
        <div className="flex items-center gap-2 border-r border-border pr-3">
          <Switch checked={activeTab.isLive} onCheckedChange={toggleActiveTabLive} />
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
            <RadioTower className="h-3 w-3" />
            Live
          </span>
          {activeTab.isLive && (
            <div className="relative">
              <Button
                variant="secondary"
                size="icon-xs"
                onClick={() => setShowSubscriptionOptions((prev) => !prev)}
                aria-label="Subscription options"
                title="Subscription options"
              >
                <Settings2 data-icon="only" />
              </Button>
              {showSubscriptionOptions && (
                <div
                  data-testid="subscription-options-panel"
                  className="absolute right-0 top-[calc(100%+6px)] z-50 w-[520px] rounded-md border border-border bg-popover p-3 text-popover-foreground shadow-lg"
                >
                  <div className="mb-3 flex items-center justify-between">
                    <span className="text-xs font-medium text-foreground">Subscription options</span>
                    <Button
                      variant="ghost"
                      size="xs"
                      className="h-6 text-xs text-muted-foreground hover:text-foreground"
                      onClick={() => updateActiveTab({ subscriptionOptions: undefined, isDirty: true })}
                    >
                      Clear
                    </Button>
                  </div>
                  <div className="grid grid-cols-3 gap-3">
                    <div className="space-y-1.5">
                      <label className="block text-xs text-muted-foreground" htmlFor={`opt-last-rows-${activeTab.id}`}>last_rows</label>
                      <Input
                        id={`opt-last-rows-${activeTab.id}`}
                        type="number"
                        min={0}
                        placeholder="–"
                        value={activeTab.subscriptionOptions?.last_rows ?? ""}
                        onChange={(event) => {
                          const val = event.target.value ? parseInt(event.target.value, 10) : undefined;
                          updateActiveTab({
                            subscriptionOptions: {
                              ...activeTab.subscriptionOptions,
                              last_rows: Number.isFinite(val) ? val : undefined,
                            },
                            isDirty: true,
                          });
                        }}
                        className="block h-7 border-border bg-background text-xs"
                      />
                    </div>
                    <div className="space-y-1.5">
                      <label className="block text-xs text-muted-foreground" htmlFor={`opt-batch-size-${activeTab.id}`}>batch_size</label>
                      <Input
                        id={`opt-batch-size-${activeTab.id}`}
                        type="number"
                        min={0}
                        placeholder="–"
                        value={activeTab.subscriptionOptions?.batch_size ?? ""}
                        onChange={(event) => {
                          const val = event.target.value ? parseInt(event.target.value, 10) : undefined;
                          updateActiveTab({
                            subscriptionOptions: {
                              ...activeTab.subscriptionOptions,
                              batch_size: Number.isFinite(val) ? val : undefined,
                            },
                            isDirty: true,
                          });
                        }}
                        className="block h-7 border-border bg-background text-xs"
                      />
                    </div>
                    <div className="space-y-1.5">
                      <label className="block text-xs text-muted-foreground" htmlFor={`opt-from-${activeTab.id}`}>from</label>
                      <Input
                        id={`opt-from-${activeTab.id}`}
                        type="text"
                        placeholder="–"
                        value={activeTab.subscriptionOptions?.from ?? ""}
                        onChange={(event) => {
                          const raw = event.target.value.trim();
                          updateActiveTab({
                            subscriptionOptions: {
                              ...activeTab.subscriptionOptions,
                              from: raw || undefined,
                            },
                            isDirty: true,
                          });
                        }}
                        className="block h-7 border-border bg-background text-xs"
                      />
                    </div>
                  </div>
                  <label className="mt-3 flex items-center gap-2 text-xs text-muted-foreground" htmlFor={`opt-auto-fetch-batches-${activeTab.id}`}>
                    <input
                      id={`opt-auto-fetch-batches-${activeTab.id}`}
                      type="checkbox"
                      checked={activeTab.subscriptionOptions?.auto_fetch_batches ?? false}
                      onChange={(event) => {
                        updateActiveTab({
                          subscriptionOptions: {
                            ...activeTab.subscriptionOptions,
                            auto_fetch_batches: event.target.checked,
                          },
                          isDirty: true,
                        });
                      }}
                      className="h-3.5 w-3.5 rounded border-border accent-primary"
                    />
                    auto fetch batches
                  </label>
                </div>
              )}
            </div>
          )}
        </div>
        <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
          <Clock3 className="h-3 w-3" />
          {lastSavedLabel}
        </span>
      </div>
      <Button
        variant="secondary"
        size="xs"
        className="h-[26px]"
        onClick={() => saveTab(activeTab.id, false)}
      >
        <Save data-icon="inline-start" />
        Save
      </Button>
      {activeTab.isLive ? (
        <Button
          size="xs"
          className="h-[26px]"
          onClick={() => executeFromToolbar("auto")}
          disabled={isExecuteDisabled}
        >
          {activeTab.liveStatus === "connected" || isConnectingLive ? (
            <Square data-icon="inline-start" />
          ) : (
            <Play data-icon="inline-start" />
          )}
          {activeTab.liveStatus === "connected"
            ? "Stop"
            : isConnectingLive
              ? "Connecting..."
              : "Subscribe"}
        </Button>
      ) : (
        <div className="flex shrink-0">
          <Button
            size="xs"
            className="h-[26px] rounded-r-none"
            onClick={() => executeFromToolbar("auto")}
            disabled={isExecuteDisabled}
          >
            <Play data-icon="inline-start" />
            {isRunning ? "Running..." : executeLabel}
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                size="xs"
                className="h-[26px] rounded-l-none border-l border-primary-foreground/20 px-2"
                disabled={isExecuteDisabled}
                aria-label="Execute options"
              >
                <ChevronDown className="h-3.5 w-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => executeFromToolbar("all")}>
                Execute all
                <DropdownMenuShortcut>Cmd/Ctrl+Enter</DropdownMenuShortcut>
              </DropdownMenuItem>
              <DropdownMenuItem
                onSelect={() => executeFromToolbar("selected")}
                disabled={!hasSelectedSql}
              >
                Selected
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      )}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="secondary" size="icon-xs" className="h-[26px]" aria-label="More query actions">
            <MoreHorizontal data-icon="only" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => saveTab(activeTab.id, true)}>
            <Copy className="mr-2 h-3.5 w-3.5" />
            Save a copy
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={deleteActiveTab} className="text-destructive">
            <Trash2 className="mr-2 h-3.5 w-3.5" />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  ) : null;

  if (!activeTab) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-sm text-muted-foreground">
        Loading SQL workspace...
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-background text-foreground">
      <div className="flex min-h-0 flex-1 overflow-hidden">
        <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
          <ResizablePanel
            defaultSize={horizontalLayout[0]}
            minSize="12%"
            maxSize="36%"
            className="min-h-0 overflow-hidden"
            onResize={(size) => {
              const numericSize = Number(size);
              if (!Number.isFinite(numericSize)) {
                return;
              }
              const safeSize = Math.max(12, Math.min(36, numericSize));
              dispatch(setHorizontalLayout([safeSize, 100 - safeSize]));
            }}
          >
            <div className="flex h-full min-h-0 flex-col">
              <StudioTabsHeader />
              <div className="min-h-0 flex-1 overflow-hidden">
                {activeStudioTab === "explorer" && (
                  <StudioExplorerPanel
                    schema={schema}
                    filter={schemaFilter}
                    savedQueries={savedQueries}
                    favoritesExpanded={favoritesExpanded}
                    namespaceSectionExpanded={namespaceSectionExpanded}
                    expandedNamespaces={expandedNamespaces}
                    expandedTables={expandedTables}
                    selectedTableKey={selectedTableKey}
                    isRefreshing={isSchemaRefreshing}
                    onFilterChange={(value) => dispatch(setSchemaFilter(value))}
                    onRefresh={() => void refreshExplorerSchema()}
                    onToggleFavorites={() => dispatch(toggleFavoritesExpanded())}
                    onToggleNamespaceSection={() => dispatch(toggleNamespaceSectionExpanded())}
                    onToggleNamespace={(namespaceName) => dispatch(toggleNamespaceExpanded(namespaceName))}
                    onToggleTable={(tableKey) => dispatch(toggleTableExpanded(tableKey))}
                    onOpenSavedQuery={openSavedQuery}
                    onSelectTable={(table) => dispatch(setSelectedTableKey(`${table.namespace}.${table.name}`))}
                    onTableContextMenu={(table, position) => {
                      setExplorerContextMenu({
                        x: position.x,
                        y: position.y,
                        table,
                      });
                    }}
                  />
                )}
                {activeStudioTab === "editor" && (
                  <EditorSidebar
                    schema={schema}
                    onSchemaRefresh={() => void refreshExplorerSchema()}
                  />
                )}
              </div>
            </div>
          </ResizablePanel>

          <ResizableHandle withHandle />

          <ResizablePanel defaultSize={horizontalLayout[1]} minSize="40%" className="min-h-0 overflow-hidden">
            <div className="relative flex h-full min-h-0 flex-col overflow-hidden bg-background">
              {activeStudioTab === "editor" ? (
                <EditTableForm
                  schema={schema}
                  isSchemaRefreshing={isSchemaRefreshing}
                  onAfterSave={refreshExplorerSchema}
                  onOpenQueryInNewTab={openQueryInNewTab}
                />
              ) : (
                <>
                  <div className="absolute right-2 top-1.5 z-20">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="text-muted-foreground hover:text-foreground"
                      onClick={toggleInspector}
                      title={isInspectorCollapsed ? "Expand details panel" : "Collapse details panel"}
                    >
                      {isInspectorCollapsed ? (
                        <PanelRightOpen className="h-4 w-4" />
                      ) : (
                        <PanelRightClose className="h-4 w-4" />
                      )}
                    </Button>
                  </div>

                  <QueryTabStrip
                tabs={tabs}
                activeTabId={activeTab.id}
                onTabSelect={(tabId) => dispatch(setWorkspaceActiveTabId(tabId))}
                onAddTab={addTab}
                onCloseTab={closeTab}
                onRenameTab={renameTab}
              />

              <ResizablePanelGroup orientation="vertical" className="min-h-0 flex-1">
                <ResizablePanel
                  defaultSize={verticalLayout[0]}
                  minSize="20%"
                  className="min-h-0 overflow-hidden"
                  onResize={(size) => {
                    const numericSize = Number(size);
                    if (!Number.isFinite(numericSize)) {
                      return;
                    }
                    const safeSize = Math.max(20, Math.min(80, numericSize));
                    dispatch(setVerticalLayout([safeSize, 100 - safeSize]));
                  }}
                >
                  <Suspense fallback={<EditorSkeleton />}>
                  <StudioEditorPanel
                    schema={schema}
                    sql={activeTab.sql}
                    onSqlChange={(value) => updateActiveTab({ sql: value, isDirty: true })}
                    onRun={(runSql) => runActiveQuery(runSql)}
                    onSelectedSqlChange={setSelectedEditorSql}
                  />
                  </Suspense>
                </ResizablePanel>

                <ResizableHandle withHandle />

                <ResizablePanel defaultSize={verticalLayout[1]} minSize="20%" className="min-h-0 overflow-hidden">
                  <StudioResultsGrid
                    result={activeResult}
                    isRunning={isRunning}
                    isLiveMode={activeTab.isLive}
                    liveBatch={activeLiveBatch}
                    liveAutoFetchBatches={activeTab.subscriptionOptions?.auto_fetch_batches ?? false}
                    activeSql={activeTab.sql}
                    selectedTable={selectedTable}
                    currentUsername={user?.username ?? "admin"}
                    resultView={activeTab.resultView}
                    onResultViewChange={(view) => updateActiveTab({ resultView: view })}
                    onFetchNextBatch={() => fetchNextLiveBatch(activeTab.id)}
                    onRefreshAfterCommit={() => executeQueryForTab(activeTab.id, activeTab.sql, activeTab.title)}
                    actions={sqlResultActions}
                  />
                </ResizablePanel>
              </ResizablePanelGroup>
                </>
              )}
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>

        {!isInspectorCollapsed && (
          <div className="min-h-0 w-[320px] min-w-[240px] max-w-[420px] overflow-hidden border-l border-border">
            <StudioInspectorPanel selectedTable={selectedTable} history={history} />
          </div>
        )}
      </div>

      <ExplorerTableContextMenu
        contextMenu={explorerContextMenu}
        onClose={() => setExplorerContextMenu(null)}
        onOpenQueryInNewTab={(table) => {
          const sql = buildSelectFromTableSql(table.namespace, table.name, true);
          openQueryInNewTab(sql, table.name);
          dispatch(setSelectedTableKey(`${table.namespace}.${table.name}`));
          setExplorerContextMenu(null);
        }}
        onSelectFromTable={(table) => {
          const sql = buildSelectFromTableSql(table.namespace, table.name, true);
          updateActiveTab({ sql, isDirty: true });
          dispatch(setSelectedTableKey(`${table.namespace}.${table.name}`));
          setExplorerContextMenu(null);
          executeQueryForTab(activeTab.id, sql, activeTab.title).catch(console.error);
        }}
        onInsertSelectQuery={(table) => {
          const sql = buildSelectFromTableSql(table.namespace, table.name, false);
          updateActiveTab({ sql, isDirty: true });
          dispatch(setSelectedTableKey(`${table.namespace}.${table.name}`));
          setExplorerContextMenu(null);
        }}
        onViewProperties={(table) => {
          dispatch(setSelectedTableKey(`${table.namespace}.${table.name}`));
          dispatch(setInspectorCollapsed(false));
          setExplorerContextMenu(null);
        }}
        onCopyQualifiedName={(table) => {
          navigator.clipboard
            .writeText(`${table.namespace}.${table.name}`)
            .catch(console.error);
          setExplorerContextMenu(null);
        }}
        onEditSchema={(table) => {
          if (isReadOnlyNamespace(table.namespace)) {
            notify({
              title: "Read-only namespace",
              description: `${table.namespace} is managed by KalamDB and can't be edited.`,
              variant: "destructive",
            });
            setExplorerContextMenu(null);
            return;
          }
          dispatch(
            startEditTable({
              tableKey: `${table.namespace}.${table.name}`,
              draft: tableToDraft(table),
            }),
          );
          dispatch(setActiveStudioTab("editor"));
          setExplorerContextMenu(null);
        }}
        onDropTable={(table) => {
          if (isReadOnlyNamespace(table.namespace)) {
            notify({
              title: "Read-only namespace",
              description: `${table.namespace} is managed by KalamDB and can't be dropped from here.`,
              variant: "destructive",
            });
            setExplorerContextMenu(null);
            return;
          }
          setExplorerContextMenu(null);
          const sql = generateDropTableSql(table.namespace, table.name);
          const fqn = `${table.namespace}.${table.name}`;
          openSqlPreview({
            sql,
            title: `Drop ${fqn}`,
            description: "This will permanently delete the table and all of its data.",
            onExecute: executeSqlPreviewStatement,
            onComplete: () => {
              notify({ title: `Dropped ${fqn}`, variant: "success" });
              if (selectedTableKey === fqn) {
                dispatch(setSelectedTableKey(null));
              }
              void refreshExplorerSchema();
            },
          });
        }}
        onCopyCreateSql={(table) => {
          const sql = generateCreateTableSql(tableToDraft(table));
          navigator.clipboard
            .writeText(sql)
            .then(() => notify({ title: "CREATE TABLE SQL copied", variant: "success" }))
            .catch(() =>
              notify({
                title: "Couldn't copy to clipboard",
                variant: "destructive",
              }),
            );
          setExplorerContextMenu(null);
        }}
      />

    </div>
  );
}
