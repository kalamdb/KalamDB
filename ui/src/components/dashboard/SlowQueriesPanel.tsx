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
}

function formatDuration(value: unknown): string {
  const duration = typeof value === "number" ? value : Number(value ?? 0);
  if (!Number.isFinite(duration)) {
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

export function SlowQueriesPanel({ queries, isLoading }: SlowQueriesPanelProps) {
  return (
    <Card className="mt-6">
      <CardHeader>
        <CardTitle className="text-base font-medium">Slow Queries</CardTitle>
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
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[170px]">Time</TableHead>
                <TableHead className="w-[110px]">Duration</TableHead>
                <TableHead className="w-[140px]">User</TableHead>
                <TableHead className="w-[140px]">Table</TableHead>
                <TableHead>Query</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {queries.map((query) => (
                <TableRow key={`${query.timestamp_ms}-${query.query}`}>
                  <TableCell>{formatTimestamp(query.timestamp)}</TableCell>
                  <TableCell>{formatDuration(query.duration_ms)}</TableCell>
                  <TableCell>{query.user_id}</TableCell>
                  <TableCell>{query.table_name ?? query.table_type}</TableCell>
                  <TableCell className="max-w-[520px] whitespace-normal break-words font-mono text-[11px] leading-relaxed">
                    {query.query}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}