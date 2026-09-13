import { useMemo, useState, type ReactNode } from "react";
import { Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { RevisionIdDisplay } from "@/components/functions/RevisionIdDisplay";
import { displayLogActor, displayLogLevel, displayLogOrigin, formatDurationMs } from "@/features/functions/format";
import { formatLogTime, formatLogTimestamp, logLevelClassName, rtkErrorMessage } from "@/features/functions/display";
import { useGetProcedureLogsQuery } from "@/store/apiSlice";
import type { SystemProcedureLogRow } from "@/features/functions/types";

const LEVELS = ["all", "debug", "info", "warn", "error"] as const;
const ORIGINS = ["all", "sql", "http", "topic"] as const;

export function FunctionLogs({ procedureId }: { procedureId: string }) {
  const [search, setSearch] = useState("");
  const [level, setLevel] = useState<string>("all");
  const [origin, setOrigin] = useState<string>("all");
  const [executionId, setExecutionId] = useState("");
  const [revisionId, setRevisionId] = useState("");
  const [selected, setSelected] = useState<SystemProcedureLogRow | null>(null);

  const filters = useMemo(
    () => ({
      procedureId,
      search,
      level,
      origin,
      executionId,
      revisionId,
      limit: 200,
    }),
    [procedureId, search, level, origin, executionId, revisionId],
  );

  const { data: logs = [], isFetching, error } = useGetProcedureLogsQuery(filters);
  const errorMessage = rtkErrorMessage(error, "Failed to load procedure logs");

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h2 className="text-sm font-semibold">Logs</h2>
        <p className="text-xs text-muted-foreground">Filtered to this procedure from system.procedure_logs.</p>
      </div>

      <div className="flex flex-col gap-2 lg:flex-row lg:items-center">
        <div className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search logs..."
            className="pl-9"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </div>
        <Select value={level} onValueChange={setLevel}>
          <SelectTrigger className="w-full lg:w-36" size="sm">
            <SelectValue placeholder="All levels" />
          </SelectTrigger>
          <SelectContent>
            {LEVELS.map((item) => (
              <SelectItem key={item} value={item}>
                {item === "all" ? "All levels" : item.toUpperCase()}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={origin} onValueChange={setOrigin}>
          <SelectTrigger className="w-full lg:w-36" size="sm">
            <SelectValue placeholder="All origins" />
          </SelectTrigger>
          <SelectContent>
            {ORIGINS.map((item) => (
              <SelectItem key={item} value={item}>
                {item === "all" ? "All origins" : item}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="grid gap-2 sm:grid-cols-2">
        <Input
          placeholder="Filter execution ID"
          value={executionId}
          onChange={(event) => setExecutionId(event.target.value)}
        />
        <Input
          placeholder="Filter revision ID"
          value={revisionId}
          onChange={(event) => setRevisionId(event.target.value)}
        />
      </div>

      {errorMessage ? <p className="text-sm text-destructive">{errorMessage}</p> : null}

      <div className="rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Time</TableHead>
              <TableHead>Level</TableHead>
              <TableHead>Actor</TableHead>
              <TableHead>Origin</TableHead>
              <TableHead>Execution</TableHead>
              <TableHead>Message</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {isFetching && logs.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="py-10 text-center text-sm text-muted-foreground">
                  Loading logs…
                </TableCell>
              </TableRow>
            ) : logs.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="py-10 text-center text-sm text-muted-foreground">
                  No logs for this procedure.
                </TableCell>
              </TableRow>
            ) : (
              logs.map((log) => (
                <TableRow
                  key={`${log.timestamp}-${log.execution_id}-${log.channel}-${log.message}`}
                  className="cursor-pointer"
                  onClick={() => setSelected(log)}
                >
                  <TableCell className="whitespace-nowrap text-muted-foreground">
                    {formatLogTime(log.timestamp)}
                  </TableCell>
                  <TableCell className={logLevelClassName(log.level)}>{displayLogLevel(log.level)}</TableCell>
                  <TableCell className="max-w-[8rem] truncate">{displayLogActor(log.actor)}</TableCell>
                  <TableCell className="text-muted-foreground">{displayLogOrigin(log.origin)}</TableCell>
                  <TableCell className="max-w-[10rem] truncate font-mono text-xs">{log.execution_id || "—"}</TableCell>
                  <TableCell className="max-w-xl truncate">{log.message || "—"}</TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      <Sheet open={selected !== null} onOpenChange={(open) => !open && setSelected(null)}>
        <SheetContent className="w-full sm:max-w-md">
          <SheetHeader>
            <SheetTitle>Log details</SheetTitle>
            <SheetDescription>Catalog metadata for this procedure log record.</SheetDescription>
          </SheetHeader>
          {selected ? <LogDetails log={selected} /> : null}
        </SheetContent>
      </Sheet>
    </div>
  );
}

function LogDetails({ log }: { log: SystemProcedureLogRow }) {
  const rows: Array<[string, ReactNode]> = [
    ["Timestamp", formatLogTimestamp(log.timestamp)],
    ["Level", displayLogLevel(log.level)],
    ["Message", log.message || "—"],
    ["Execution ID", <span className="break-all font-mono text-xs">{log.execution_id || "—"}</span>],
    ["Request ID", <span className="break-all font-mono text-xs">{log.request_id || "—"}</span>],
    ["Origin", displayLogOrigin(log.origin)],
    ["Outcome", log.outcome || "—"],
    ["Actor", displayLogActor(log.actor)],
    ["Node", log.node_id || "—"],
    ["Channel", log.channel || "—"],
    ["Revision", <RevisionIdDisplay revisionId={log.revision_id} />],
    ["Module", log.module_id || "—"],
    ["Error code", log.error_code || "—"],
    ["Duration", formatDurationMs(log.duration_ms)],
  ];

  return (
    <dl className="grid gap-3 px-6 pb-6">
      {rows.map(([label, value]) => (
        <div key={label} className="grid gap-1">
          <dt className="text-xs text-muted-foreground">{label}</dt>
          <dd className="text-sm">{value}</dd>
        </div>
      ))}
    </dl>
  );
}
