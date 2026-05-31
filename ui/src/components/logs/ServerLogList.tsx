import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useGetServerLogsQuery } from '@/store/apiSlice';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Loader2, Search, Pause, Info, AlertTriangle, AlertCircle, Bug, RefreshCw, Play, ChevronLeft, ChevronRight } from 'lucide-react';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { formatTimestamp } from '@/lib/formatters';
import { DEFAULT_PAGE_SIZE, PAGE_SIZE_OPTIONS } from '@/lib/config';

const LOG_GRID_TEMPLATE_COLUMNS = 'minmax(190px, 220px) minmax(74px, 90px) minmax(180px, 240px) minmax(360px, 1fr)';
const GRAPH_BUCKET_COUNT = 48;

const LEVEL_CONFIG: Record<string, { color: string; icon: typeof AlertCircle }> = {
  'ERROR': { color: 'text-red-500', icon: AlertCircle },
  'WARN': { color: 'text-yellow-500', icon: AlertTriangle },
  'INFO': { color: 'text-blue-500', icon: Info },
  'DEBUG': { color: 'text-gray-500', icon: Bug },
  'TRACE': { color: 'text-purple-500', icon: Bug },
};

function getLevelConfig(level: string) {
  return LEVEL_CONFIG[level.toUpperCase()] || { color: 'text-gray-500', icon: Info };
}

function sqlLiteral(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

function parseTimestamp(value: string): number | null {
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

function formatBucketLabel(value: number): string {
  return new Date(value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
}

export function ServerLogList() {
  const navigate = useNavigate();
  const [isPaused, setIsPaused] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [logType, setLogType] = useState('all');
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [pageIndex, setPageIndex] = useState(0);
  const [pageCursors, setPageCursors] = useState<Array<string | null>>([null]);

  const cursor = pageCursors[pageIndex] ?? null;
  const pollingInterval = !isPaused && pageIndex === 0 ? 2000 : 0;

  const {
    data: logs = [],
    isLoading,
    error: queryError,
    refetch
  } = useGetServerLogsQuery(
    {
      limit: pageSize,
      level: logType !== 'all' ? logType.toUpperCase() : undefined,
      beforeTimestamp: cursor ?? undefined,
    },
    {
      pollingInterval,
    }
  );

  const error = queryError && 'error' in queryError && typeof queryError.error === 'string'
    ? queryError.error
    : queryError
      ? 'Failed to fetch server logs'
      : null;

  useEffect(() => {
    setPageIndex(0);
    setPageCursors([null]);
  }, [pageSize, logType]);

  const filteredLogs = useMemo(() => {
    return logs.filter((log) => {
      if (searchQuery) {
        const message = caseSensitive ? log.message : log.message.toLowerCase();
        const query = caseSensitive ? searchQuery : searchQuery.toLowerCase();
        if (!message.includes(query)) {
          return false;
        }
      }
      return true;
    });
  }, [logs, searchQuery, caseSensitive]);

  const histogram = useMemo(() => {
    const timestamps = logs
      .map((log) => parseTimestamp(log.timestamp))
      .filter((timestamp): timestamp is number => timestamp !== null)
      .sort((a, b) => a - b);

    if (timestamps.length === 0) {
      return { buckets: [], maxCount: 0, rangeLabel: 'No timestamped logs' };
    }

    const start = timestamps[0];
    const end = timestamps[timestamps.length - 1];
    const span = Math.max(1, end - start);

    const buckets = Array.from({ length: GRAPH_BUCKET_COUNT }, (_, index) => {
      const bucketTime = start + (span * index) / Math.max(1, GRAPH_BUCKET_COUNT - 1);
      return {
        label: formatBucketLabel(bucketTime),
        count: 0,
      };
    });

    for (const timestamp of timestamps) {
      const bucket = Math.min(GRAPH_BUCKET_COUNT - 1, Math.floor(((timestamp - start) / span) * GRAPH_BUCKET_COUNT));
      buckets[bucket].count += 1;
    }

    const maxCount = buckets.reduce((max, bucket) => Math.max(max, bucket.count), 0);
    return {
      buckets,
      maxCount,
      rangeLabel: `${formatBucketLabel(start)} - ${formatBucketLabel(end)}`,
    };
  }, [logs]);

  const hasOlderPage = logs.length === pageSize;
  const hasNewerPage = pageIndex > 0;

  const pageSql = useMemo(() => {
    const where: string[] = [];
    if (logType !== 'all') {
      where.push(`level = ${sqlLiteral(logType.toUpperCase())}`);
    }
    if (cursor) {
      where.push(`timestamp < ${sqlLiteral(cursor)}`);
    }
    if (searchQuery.trim().length > 0) {
      const escaped = `%${searchQuery.trim()}%`;
      if (caseSensitive) {
        where.push(`message LIKE ${sqlLiteral(escaped)}`);
      } else {
        where.push(`LOWER(message) LIKE LOWER(${sqlLiteral(escaped)})`);
      }
    }

    return [
      'SELECT timestamp, level, target, line, message',
      'FROM system.server_logs',
      where.length > 0 ? `WHERE ${where.join(' AND ')}` : null,
      'ORDER BY timestamp DESC',
      `LIMIT ${pageSize};`,
    ]
      .filter(Boolean)
      .join('\n');
  }, [logType, cursor, searchQuery, caseSensitive, pageSize]);

  const openSqlStudio = () => {
    navigate('/sql', {
      state: {
        prefillSql: pageSql,
        prefillTitle: 'Server Logs',
      },
    });
  };

  const goOlder = () => {
    if (!hasOlderPage || logs.length === 0) {
      return;
    }
    const oldest = logs[logs.length - 1]?.timestamp;
    if (!oldest) {
      return;
    }
    setPageCursors((prev) => [...prev.slice(0, pageIndex + 1), oldest]);
    setPageIndex((prev) => prev + 1);
  };

  const goNewer = () => {
    if (!hasNewerPage) {
      return;
    }
    setPageIndex((prev) => Math.max(0, prev - 1));
  };

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden rounded-md border bg-background text-sm text-foreground shadow-sm">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b bg-muted/10 px-4 py-3">
        <div className="min-w-0">
          <h3 className="text-sm font-medium">Server Logs</h3>
          <p className="text-xs text-muted-foreground">Query-backed view of system.server_logs</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button variant="outline" size="sm" onClick={openSqlStudio} className="h-9">
            <Play className="h-3.5 w-3.5 mr-2" />
            Query
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setIsPaused(!isPaused)}
            className={`h-9 ${isPaused ? 'text-primary border-primary/50' : 'text-muted-foreground'}`}
          >
            <Pause className="h-3.5 w-3.5 mr-2" />
            {isPaused ? 'RESUME' : 'PAUSE'}
          </Button>
          <Button variant="outline" size="sm" className="h-9" onClick={() => refetch()} disabled={isLoading}>
            <RefreshCw className={`h-3.5 w-3.5 mr-2 ${isLoading ? 'animate-spin' : ''}`} />
            REFRESH
          </Button>
        </div>
      </div>

      {/* Graph Section */}
      <div className="flex h-28 shrink-0 flex-col border-b bg-muted/30 p-4">
        <div className="mb-2 flex items-center justify-between">
          <span className="text-xs text-muted-foreground">Log Frequency Graph</span>
          <span className="text-xs text-muted-foreground">{histogram.rangeLabel} · {logs.length} rows</span>
        </div>
        {histogram.maxCount === 0 ? (
          <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
            No timestamped logs available.
          </div>
        ) : (
          <div className="mt-1 flex h-14 items-end gap-px">
            {histogram.buckets.map((bucket, index) => {
              const height = bucket.count > 0 ? Math.max(4, Math.round((bucket.count / histogram.maxCount) * 52)) : 2;
              return (
                <div key={`${bucket.label}-${index}`} className="flex h-full min-w-0 flex-1 items-end">
                  <div
                    className={bucket.count > 0 ? 'w-full rounded-t-sm bg-primary' : 'w-full rounded-t-sm bg-border'}
                    style={{ height: `${height}px` }}
                    title={`${bucket.label}: ${bucket.count} logs`}
                  />
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Toolbar */}
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b bg-background px-4 py-3">
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">Filters</span>
        </div>
        <div className="flex items-center gap-3">

          <label className="flex items-center gap-2 text-xs text-muted-foreground cursor-pointer">
            <input
              type="checkbox"
              checked={caseSensitive}
              onChange={(e) => setCaseSensitive(e.target.checked)}
              className="rounded border-input bg-transparent"
            />
            Case sensitive
          </label>

          <div className="relative w-64">
            <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              placeholder="Filter logs"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-9 bg-background text-sm h-9"
            />
          </div>

          <Select value={logType} onValueChange={setLogType}>
            <SelectTrigger className="w-36 bg-background h-9 text-sm">
              <SelectValue placeholder="All Levels" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All Levels</SelectItem>
              <SelectItem value="info">Info</SelectItem>
              <SelectItem value="warn">Warning</SelectItem>
              <SelectItem value="error">Error</SelectItem>
              <SelectItem value="debug">Debug</SelectItem>
            </SelectContent>
          </Select>

          <Select
            value={String(pageSize)}
            onValueChange={(value) => setPageSize(Number(value))}
          >
            <SelectTrigger className="w-[88px] bg-background h-9 text-sm">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PAGE_SIZE_OPTIONS.map((size) => (
                <SelectItem key={size} value={String(size)}>{size}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      {/* Table Header */}
      <div
        className="grid min-w-[860px] gap-4 border-b bg-muted/30 px-4 py-2 text-xs font-semibold text-muted-foreground"
        style={{ gridTemplateColumns: LOG_GRID_TEMPLATE_COLUMNS }}
      >
        <div>Timestamp</div>
        <div>Level</div>
        <div>Target</div>
        <div>Message</div>
      </div>

      {/* Table Body */}
      <div className="min-h-0 flex-1 overflow-auto bg-background font-mono">
        {error ? (
          <div className="p-4">
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" />
              <AlertDescription className="mt-2 space-y-3">
                <p>{error}</p>
                <Button variant="outline" onClick={() => refetch()}>
                  Retry
                </Button>
              </AlertDescription>
            </Alert>
          </div>
        ) : isLoading && filteredLogs.length === 0 ? (
          <div className="flex items-center justify-center h-full">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : filteredLogs.length === 0 ? (
          <div className="flex items-center justify-center h-full text-muted-foreground font-sans">
            No logs found
          </div>
        ) : (
          <div className="flex flex-col">
            {filteredLogs.map((log, index) => {
              const levelConfig = getLevelConfig(log.level);
              const LevelIcon = levelConfig.icon;
              return (
                <div 
                  key={`${log.timestamp}-${index}`}
                  className="grid min-w-[860px] gap-4 border-b border-border/50 px-4 py-2 text-xs transition-colors hover:bg-muted/50"
                  style={{ gridTemplateColumns: LOG_GRID_TEMPLATE_COLUMNS }}
                >
                  <div className="text-muted-foreground truncate">
                    {formatTimestamp(log.timestamp, 'Timestamp(Microsecond, None)')}
                  </div>
                  <div className={`flex items-center gap-1.5 ${levelConfig.color}`}>
                    <LevelIcon className="h-3 w-3" />
                    <span className="capitalize">{log.level.toLowerCase()}</span>
                  </div>
                  <div className="truncate">
                    {log.target || '__kalamdb__'}
                  </div>
                  <div className="truncate">
                    {log.message}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Pagination Footer */}
      <div className="border-t px-4 py-2 bg-muted/10 font-sans flex items-center justify-between">
        <span className="text-xs text-muted-foreground">Page {pageIndex + 1}</span>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={goNewer} disabled={!hasNewerPage}>
            <ChevronLeft className="h-3.5 w-3.5 mr-1" />
            Prev (Newer)
          </Button>
          <Button variant="outline" size="sm" onClick={goOlder} disabled={!hasOlderPage}>
            Next (Older)
            <ChevronRight className="h-3.5 w-3.5 ml-1" />
          </Button>
        </div>
      </div>
    </div>
  );
}
