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
    const bucketCount = 24;
    const hourMs = 60 * 60 * 1000;
    const now = Date.now();
    const start = now - (bucketCount - 1) * hourMs;

    const buckets = Array.from({ length: bucketCount }, (_, index) => {
      const bucketTime = start + index * hourMs;
      return {
        label: new Date(bucketTime).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false }),
        count: 0,
      };
    });

    for (const log of logs) {
      const timestamp = parseTimestamp(log.timestamp);
      if (timestamp === null || timestamp < start || timestamp > now) {
        continue;
      }
      const bucket = Math.min(bucketCount - 1, Math.floor((timestamp - start) / hourMs));
      buckets[bucket].count += 1;
    }

    const maxCount = buckets.reduce((max, bucket) => Math.max(max, bucket.count), 0);
    return { buckets, maxCount };
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
    setPageCursors((prev) => [...prev, oldest]);
    setPageIndex((prev) => prev + 1);
  };

  const goNewer = () => {
    if (!hasNewerPage) {
      return;
    }
    setPageIndex((prev) => Math.max(0, prev - 1));
  };

  return (
    <div className="flex flex-col h-full min-h-0 bg-background text-foreground font-mono text-sm border rounded-md overflow-hidden shadow-sm">
      {/* Graph Section */}
      <div className="h-28 border-b p-4 flex flex-col bg-muted/30">
        <div className="flex items-center justify-between mb-2">
          <span className="text-xs text-muted-foreground font-sans">Log Frequency Graph (last 24h)</span>
          <span className="text-xs text-muted-foreground font-sans">{logs.length} rows on this page</span>
        </div>
        {histogram.maxCount === 0 ? (
          <div className="flex h-full items-center justify-center text-xs text-muted-foreground font-sans">
            No timestamped logs available for the last 24 hours.
          </div>
        ) : (
          <div className="flex items-end justify-between h-14 gap-1 mt-1">
            {histogram.buckets.map((bucket, index) => {
              const height = histogram.maxCount > 0 ? Math.max(6, Math.round((bucket.count / histogram.maxCount) * 48)) : 6;
              const emphasized = index === histogram.buckets.length - 1;
              return (
                <div key={`${bucket.label}-${index}`} className="flex flex-col items-center flex-1 gap-1">
                  <div
                    className={emphasized ? 'w-full max-w-[22px] rounded-t-sm bg-primary' : 'w-full max-w-[22px] rounded-t-sm bg-muted-foreground/40'}
                    style={{ height: `${height}px` }}
                    title={`${bucket.label}: ${bucket.count} logs`}
                  />
                  {index % 3 === 0 ? (
                    <span className="text-[9px] text-muted-foreground">{bucket.label}</span>
                  ) : (
                    <span className="text-[9px] text-transparent">00:00</span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Toolbar */}
      <div className="flex items-center justify-between p-3 border-b bg-muted/10 font-sans">
        <h3 className="font-medium text-sm">Server Logs</h3>
        <div className="flex items-center gap-3">
          <Button variant="outline" size="sm" onClick={openSqlStudio} className="h-9">
            <Play className="h-3.5 w-3.5 mr-2" />
            Query
          </Button>

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

      {/* Table Header */}
      <div className="grid grid-cols-[220px_90px_220px_1fr] gap-4 px-4 py-2 border-b text-xs font-semibold text-muted-foreground bg-muted/30 font-sans">
        <div>Timestamp</div>
        <div>Level</div>
        <div>Target</div>
        <div>Message</div>
      </div>

      {/* Table Body */}
      <div className="flex-1 overflow-auto bg-background">
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
                  className="grid grid-cols-[220px_90px_220px_1fr] gap-4 px-4 py-2 border-b border-border/50 hover:bg-muted/50 text-xs transition-colors"
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
