import { memo, useEffect, useMemo, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { StudioChromeLabel, StudioIconButton } from "../shared/StudioChrome";
import {
  FavoritesTableBlock,
  NamespaceSearchControls,
  TableColumnTree,
  namespaceNames,
  reconcileActiveNamespace,
  useNamespaceTables,
} from "../shared/NamespaceTableBrowser";
import type { SavedQuery, StudioNamespace, StudioTable } from "../shared/types";

interface StudioExplorerPanelProps {
  schema: StudioNamespace[];
  filter: string;
  savedQueries: SavedQuery[];
  favoritesExpanded: boolean;
  expandedTables: Record<string, boolean>;
  selectedTableKey: string | null;
  isRefreshing: boolean;
  onFilterChange: (value: string) => void;
  onRefresh: () => void;
  onToggleFavorites: () => void;
  onToggleTable: (tableKey: string) => void;
  onOpenSavedQuery: (queryId: string) => void;
  onSelectTable: (table: StudioTable) => void;
  onTableContextMenu: (table: StudioTable, position: { x: number; y: number }) => void;
}

const StudioExplorerPanelComponent = ({
  schema,
  filter,
  savedQueries,
  favoritesExpanded,
  expandedTables,
  selectedTableKey,
  isRefreshing,
  onFilterChange,
  onRefresh,
  onToggleFavorites,
  onToggleTable,
  onOpenSavedQuery,
  onSelectTable,
  onTableContextMenu,
}: StudioExplorerPanelProps) => {
  const namespaces = useMemo(() => namespaceNames(schema), [schema]);
  const [activeNamespace, setActiveNamespace] = useState("");
  const previousSelectedTableKeyRef = useRef<string | null>(null);

  useEffect(() => {
    setActiveNamespace((currentNamespace) =>
      reconcileActiveNamespace({
        activeNamespace: currentNamespace,
        namespaces,
        selectedTableKey,
        previousSelectedTableKey: previousSelectedTableKeyRef.current,
      }),
    );
    previousSelectedTableKeyRef.current = selectedTableKey;
  }, [namespaces, selectedTableKey]);

  const tablesInNamespace = useNamespaceTables({
    schema,
    activeNamespace,
    filter,
  });

  return (
    <TooltipProvider delayDuration={250}>
      <div className="flex h-full min-h-0 flex-col overflow-hidden border-r border-border bg-background text-foreground">
        <NamespaceSearchControls
          namespaces={namespaces}
          activeNamespace={activeNamespace}
          filter={filter}
          onNamespaceChange={setActiveNamespace}
          onFilterChange={onFilterChange}
          actions={
            <StudioIconButton
              onClick={onRefresh}
              disabled={isRefreshing}
              aria-label="Refresh explorer"
              tooltip="Refresh explorer"
            >
              <RefreshCw
                className={isRefreshing ? "animate-spin" : undefined}
                data-icon="only"
              />
            </StudioIconButton>
          }
          emptyAction={
            <div className="flex h-8 w-full items-center justify-center rounded-md border border-dashed border-border text-xs text-muted-foreground">
              No namespaces.
            </div>
          }
        />

        <FavoritesTableBlock
          savedQueries={savedQueries}
          expanded={favoritesExpanded}
          onToggle={onToggleFavorites}
          onOpenSavedQuery={onOpenSavedQuery}
        />

        <div className="flex shrink-0 items-center justify-between border-b border-border px-3 py-1.5">
          <StudioChromeLabel>
            Tables ({tablesInNamespace.length})
          </StudioChromeLabel>
        </div>

        <TableColumnTree
          tables={tablesInNamespace}
          activeNamespace={activeNamespace}
          filter={filter}
          isRefreshing={isRefreshing}
          selectedTableKey={selectedTableKey}
          expandedTables={expandedTables}
          onToggleTable={onToggleTable}
          onSelectTable={onSelectTable}
          onTableContextMenu={onTableContextMenu}
        />
      </div>
    </TooltipProvider>
  );
};

export const StudioExplorerPanel = memo(StudioExplorerPanelComponent);
