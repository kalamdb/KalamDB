import { useEffect, useMemo, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { SlowQuery } from "@/services/systemTableService";

interface SlowQueriesPanelProps {
  queries: SlowQuery[];
  isLoading?: boolean;
  pageSize?: number;
}

type SlowQueryPriority = "low" | "medium" | "high";

function normalizeDuration(value: unknown): number {
  const duration = typeof value === "number" ? value : Number(value ?? 0);
  return Number.isFinite(duration) ? duration : 0;
}

function getSlowQueryPriority(durationMs: unknown): SlowQueryPriority {
  const duration = normalizeDuration(durationMs);
  if (duration >= 5000) {
    return "high";
  }
  if (duration >= 1000) {
    return "medium";
  }
  return "low";
}

function priorityLabel(priority: SlowQueryPriority): string {
  return priority.charAt(0).toUpperCase() + priority.slice(1);
}

function priorityClassName(priority: SlowQueryPriority): string {
  if (priority === "high") {
    return "bg-red-100 text-red-800 hover:bg-red-100";
  }
  if (priority === "medium") {
    return "bg-yellow-100 text-yellow-800 hover:bg-yellow-100";
  }
  return "bg-green-100 text-green-800 hover:bg-green-100";
}

function formatDuration(value: unknown): string {
  const duration = normalizeDuration(value);
  if (duration <= 0) {
    return "-";
  }

  return `${duration.toFixed(duration >= 100 ? 0 : 1)} ms`;
}

function formatTimestamp(value: unknown): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    return "-";
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }

  return parsed.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function SlowQueriesPanel({ queries, isLoading, pageSize = 10 }: SlowQueriesPanelProps) {
  const sortedQueries = useMemo(
    () => [...queries].sort((left, right) => normalizeDuration(right.timestamp_ms) - normalizeDuration(left.timestamp_ms)),
    [queries],
  );
  const totalPages = Math.max(1, Math.ceil(sortedQueries.length / pageSize));
  const [page, setPage] = useState(1);

  useEffect(() => {
    setPage(1);
  }, [sortedQueries.length]);

  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

  const pageStart = (page - 1) * pageSize;
  const visibleQueries = sortedQueries.slice(pageStart, pageStart + pageSize);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Slow Queries</CardTitle>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="flex h-36 items-center justify-center text-sm text-muted-foreground">
            Loading slow queries...
          </div>
        ) : queries.length === 0 ? (
          <div className="flex h-36 items-center justify-center text-sm text-muted-foreground">
            No slow queries recorded.
          </div>
        ) : (
          <div className="overflow-x-auto">
            <Table className="min-w-[760px]">
              <TableHeader>
                <TableRow>
                  <TableHead className="w-[180px]">Timestamp</TableHead>
                  <TableHead>SQL statement</TableHead>
                  <TableHead className="w-[120px]">Time took</TableHead>
                  <TableHead className="w-[120px]">Priority</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {visibleQueries.map((query) => {
                  const priority = getSlowQueryPriority(query.duration_ms);
                  return (
                    <TableRow key={`${query.timestamp_ms}-${query.query}`}>
                      <TableCell>{formatTimestamp(query.timestamp)}</TableCell>
                      <TableCell className="max-w-[680px] whitespace-normal break-words font-mono text-[11px] leading-relaxed">
                        {query.query}
                      </TableCell>
                      <TableCell>{formatDuration(query.duration_ms)}</TableCell>
                      <TableCell>
                        <Badge className={priorityClassName(priority)}>{priorityLabel(priority)}</Badge>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
            <div className="mt-3 flex items-center justify-between gap-2 border-t pt-3 text-xs text-muted-foreground">
              <span>
                Showing {visibleQueries.length} of {sortedQueries.length} latest queries
              </span>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={page <= 1}
                  onClick={() => setPage((current) => Math.max(1, current - 1))}
                >
                  Previous
                </Button>
                <span>
                  Page {page} / {totalPages}
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={page >= totalPages}
                  onClick={() => setPage((current) => Math.min(totalPages, current + 1))}
                >
                  Next
                </Button>
              </div>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}