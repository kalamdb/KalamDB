import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useGetSlowQueriesQuery } from '@/store/apiSlice';
import { formatTimestamp } from '@/lib/formatters';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Loader2, Play, RefreshCw } from 'lucide-react';
import { PAGE_SIZE_OPTIONS } from '@/lib/config';

function formatDuration(durationMs: unknown): string {
  const value = typeof durationMs === 'number' ? durationMs : Number(durationMs ?? 0);
  if (!Number.isFinite(value) || value <= 0) {
    return '-';
  }
  return `${value.toFixed(value >= 100 ? 0 : 1)} ms`;
}

export function SlowQueriesLogList() {
  const navigate = useNavigate();
  const [limit, setLimit] = useState(50);

  const {
    data: queries = [],
    isLoading,
    error,
    refetch,
  } = useGetSlowQueriesQuery(limit, {
    pollingInterval: 5000,
  });

  const errorMessage = error && 'error' in error && typeof error.error === 'string'
    ? error.error
    : error
      ? 'Failed to fetch slow queries'
      : null;

  const pageSql = useMemo(() => {
    return [
      'SELECT timestamp, timestamp_ms, duration_ms, user_id, table_type, table_name, row_count, query',
      'FROM system.slow_queries',
      'ORDER BY timestamp_ms DESC',
      `LIMIT ${limit};`,
    ].join('\n');
  }, [limit]);

  const openSqlStudio = () => {
    navigate('/sql', {
      state: {
        prefillSql: pageSql,
        prefillTitle: 'Slow Queries',
      },
    });
  };

  return (
    <div className="flex h-full min-h-0 flex-col rounded-md border bg-background">
      <div className="flex items-center justify-between border-b bg-muted/10 px-4 py-3">
        <h3 className="text-sm font-medium">Slow Queries</h3>
        <div className="flex items-center gap-2">
          <Select value={String(limit)} onValueChange={(value) => setLimit(Number(value))}>
            <SelectTrigger className="h-9 w-[92px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PAGE_SIZE_OPTIONS.map((size) => (
                <SelectItem key={size} value={String(size)}>{size}</SelectItem>
              ))}
            </SelectContent>
          </Select>

          <Button variant="outline" size="sm" onClick={openSqlStudio} className="h-9">
            <Play className="mr-2 h-3.5 w-3.5" />
            Query
          </Button>
          <Button variant="outline" size="sm" onClick={() => refetch()} disabled={isLoading} className="h-9">
            <RefreshCw className={`mr-2 h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
            Refresh
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {errorMessage ? (
          <div className="p-4">
            <Alert variant="destructive">
              <AlertDescription className="mt-2 space-y-3">
                <p>{errorMessage}</p>
                <Button variant="outline" onClick={() => refetch()}>Retry</Button>
              </AlertDescription>
            </Alert>
          </div>
        ) : isLoading && queries.length === 0 ? (
          <div className="flex h-full items-center justify-center">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : queries.length === 0 ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            No slow queries recorded.
          </div>
        ) : (
          <Table className="border-separate border-spacing-0 [&_td]:border-b [&_td]:border-border">
            <TableHeader>
              <TableRow>
                <TableHead className="sticky top-0 z-10 bg-background border-b border-border">Timestamp</TableHead>
                <TableHead className="sticky top-0 z-10 bg-background border-b border-border">Duration</TableHead>
                <TableHead className="sticky top-0 z-10 bg-background border-b border-border">User</TableHead>
                <TableHead className="sticky top-0 z-10 bg-background border-b border-border">Table</TableHead>
                <TableHead className="sticky top-0 z-10 bg-background border-b border-border">Rows</TableHead>
                <TableHead className="sticky top-0 z-10 bg-background border-b border-border">SQL</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {queries.map((item) => {
                const tableName = item.table_name ? `${item.table_type}.${item.table_name}` : item.table_type;
                return (
                  <TableRow key={`${item.timestamp_ms}-${item.query.slice(0, 24)}`}>
                    <TableCell>{formatTimestamp(item.timestamp, 'Timestamp(Microsecond, None)')}</TableCell>
                    <TableCell>{formatDuration(item.duration_ms)}</TableCell>
                    <TableCell className="font-mono text-xs">{item.user_id || '-'}</TableCell>
                    <TableCell>{tableName || '-'}</TableCell>
                    <TableCell>{item.row_count}</TableCell>
                    <TableCell className="max-w-[760px] truncate font-mono text-xs" title={item.query}>
                      {item.query}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </div>
    </div>
  );
}
