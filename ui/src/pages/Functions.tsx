import { useMemo, useState } from "react";
import { RefreshCw, Search } from "lucide-react";
import { PageLayout } from "@/components/layout/PageLayout";
import { FunctionsTable } from "@/components/functions/FunctionsTable";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { formatCount, rtkErrorMessage, statusLabel } from "@/features/functions/display";
import { listItemsFromCatalog } from "@/services/functionsService";
import { summarizeProcedureStatuses } from "@/features/functions/status";
import type { ProcedureStatus } from "@/features/functions/types";
import { useGetProcedureCatalogQuery } from "@/store/apiSlice";

const STATUS_FILTERS = ["all", "ready", "warning", "error"] as const;

export default function Functions() {
  const { data: catalog, isFetching, error, refetch } = useGetProcedureCatalogQuery();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<(typeof STATUS_FILTERS)[number]>("all");

  const items = useMemo(() => (catalog ? listItemsFromCatalog(catalog) : []), [catalog]);
  const summary = summarizeProcedureStatuses(items);
  const filtered = useMemo(() => {
    const query = search.trim().toLowerCase();
    return items.filter((item) => {
      if (status !== "all" && item.status !== status) {
        return false;
      }
      if (!query) {
        return true;
      }
      return (
        item.id.toLowerCase().includes(query) ||
        (item.comment ?? "").toLowerCase().includes(query) ||
        (item.moduleId ?? "").toLowerCase().includes(query)
      );
    });
  }, [items, search, status]);

  const errorMessage = rtkErrorMessage(error, "Failed to load functions");

  return (
    <PageLayout
      title="Functions"
      description="Manage server functions."
      actions={(
        <Button variant="outline" size="sm" onClick={() => void refetch()} disabled={isFetching}>
          <RefreshCw className={`h-4 w-4 ${isFetching ? "animate-spin" : ""}`} />
          Refresh
        </Button>
      )}
    >
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border px-4 py-3 text-sm">
        <span>
          {formatCount(summary.total)} function{summary.total === 1 ? "" : "s"}
        </span>
        <span className="text-muted-foreground">
          {summary.ready} ready · {summary.warning} warning · {summary.error} errors
        </span>
      </div>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="relative max-w-sm flex-1">
          <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search functions..."
            className="pl-9"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </div>
        <Select value={status} onValueChange={(value) => setStatus(value as ProcedureStatus | "all")}>
          <SelectTrigger className="w-full sm:w-40">
            <SelectValue placeholder="All status" />
          </SelectTrigger>
          <SelectContent>
            {STATUS_FILTERS.map((item) => (
              <SelectItem key={item} value={item}>
                {item === "all" ? "All status" : statusLabel(item)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {errorMessage ? <p className="text-sm text-destructive">{errorMessage}</p> : null}

      <FunctionsTable items={filtered} isLoading={isFetching && items.length === 0} />
    </PageLayout>
  );
}
