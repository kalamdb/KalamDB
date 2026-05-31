import { useEffect, useMemo, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { AlertCircle, ArrowDown, ArrowUp, CheckCircle2, Info } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { CodeBlock } from "@/components/ui/code-block";
import { ScrollArea } from "@/components/ui/scroll-area";
import { PanelHeader, chromeLabelClassName } from "@/components/layout/typography";
import { cn } from "@/lib/utils";
import type { QueryLogEntry } from "../../shared/types";

interface StudioExecutionLogProps {
  logs: QueryLogEntry[];
  status: "success" | "error";
}

function formatLogTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

type LogEntryKind = "send" | "receive" | "error" | "event";

interface DecoratedLogEntry {
  entry: QueryLogEntry;
  index: number;
  kind: LogEntryKind;
  label: string;
  title: string;
  preview: string | null;
  payload: unknown;
  sizeLabel: string | null;
  Icon: LucideIcon;
}

function unwrapPayload(response: unknown): unknown {
  if (response && typeof response === "object" && !Array.isArray(response) && "raw" in response) {
    const raw = (response as { raw?: unknown }).raw;
    if (raw !== undefined) {
      return raw;
    }
  }
  return response;
}

function serializePayload(value: unknown): string | null {
  if (value === undefined) {
    return null;
  }
  if (value === null) {
    return "null";
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") {
    return String(value);
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function formatPayloadSize(value: unknown): string | null {
  const serialized = serializePayload(value);
  if (!serialized || serialized.length === 0) {
    return null;
  }

  const byteLength = typeof TextEncoder === "function"
    ? new TextEncoder().encode(serialized).length
    : serialized.length;

  if (byteLength < 1024) {
    return `${byteLength}b`;
  }

  const kilobytes = byteLength / 1024;
  return `${kilobytes < 10 ? kilobytes.toFixed(1) : Math.round(kilobytes)}kb`;
}

function previewPayload(value: unknown): string | null {
  const serialized = serializePayload(value);
  if (!serialized) {
    return null;
  }

  const condensed = serialized.replace(/\s+/g, " ").trim();
  if (!condensed) {
    return null;
  }

  const maxPreviewLength = 240;
  return condensed.length > maxPreviewLength
    ? `...${condensed.slice(0, maxPreviewLength - 3)}`
    : condensed;
}

function buildFallbackPayload(entry: QueryLogEntry): Record<string, unknown> {
  return {
    message: entry.message,
    level: entry.level,
    as_user: entry.asUser ?? null,
    row_count: entry.rowCount ?? null,
    statement_index: typeof entry.statementIndex === "number" ? entry.statementIndex + 1 : null,
    created_at: entry.createdAt,
  };
}

function decorateLogEntry(entry: QueryLogEntry, index: number): DecoratedLogEntry {
  const payload = unwrapPayload(entry.response);
  const detailPayload = payload === undefined ? buildFallbackPayload(entry) : payload;
  const sendPrefix = "WS SEND · ";
  const receivePrefix = "WS RECEIVE · ";

  if (entry.message.startsWith(sendPrefix)) {
    return {
      entry,
      index,
      kind: "send",
      label: "OUT",
      title: entry.message.slice(sendPrefix.length),
      preview: previewPayload(detailPayload),
      payload: detailPayload,
      sizeLabel: formatPayloadSize(detailPayload),
      Icon: ArrowUp,
    };
  }

  if (entry.message.startsWith(receivePrefix)) {
    return {
      entry,
      index,
      kind: "receive",
      label: "IN",
      title: entry.message.slice(receivePrefix.length),
      preview: previewPayload(detailPayload),
      payload: detailPayload,
      sizeLabel: formatPayloadSize(detailPayload),
      Icon: ArrowDown,
    };
  }

  if (entry.level === "error") {
    return {
      entry,
      index,
      kind: "error",
      label: "ERR",
      title: entry.message,
      preview: previewPayload(detailPayload),
      payload: detailPayload,
      sizeLabel: formatPayloadSize(detailPayload),
      Icon: AlertCircle,
    };
  }

  return {
    entry,
    index,
    kind: "event",
    label: "LOG",
    title: entry.message,
    preview: previewPayload(detailPayload),
    payload: detailPayload,
    sizeLabel: formatPayloadSize(detailPayload),
    Icon: payload === undefined ? CheckCircle2 : Info,
  };
}

function getBadgeClassName(kind: LogEntryKind): string {
  switch (kind) {
    case "send":
      return "border-red-500/40 bg-red-500/10 text-red-700";
    case "receive":
      return "border-emerald-500/40 bg-emerald-500/10 text-emerald-700";
    case "error":
      return "border-red-500/40 bg-red-500/10 text-red-700";
    case "event":
      return "border-border bg-muted/40 text-muted-foreground";
  }
}

function getIconClassName(kind: LogEntryKind, status: StudioExecutionLogProps["status"]): string {
  switch (kind) {
    case "send":
      return "text-red-500";
    case "receive":
      return "text-emerald-500";
    case "error":
      return "text-red-500";
    case "event":
      return status === "success" ? "text-emerald-500" : "text-muted-foreground";
  }
}

function describeFlow(kind: LogEntryKind): string {
  switch (kind) {
    case "send":
      return "Client -> server";
    case "receive":
      return "Server -> client";
    case "error":
      return "Connection or execution failure";
    case "event":
      return "Local execution or SDK event";
  }
}

export function StudioExecutionLog({ logs, status }: StudioExecutionLogProps) {
  const [selectedLogId, setSelectedLogId] = useState<string | null>(null);
  const previousLastLogIdRef = useRef<string | null>(null);

  const handleSelectLog = (id: string) => {
    setSelectedLogId(id);
  };

  const decoratedLogs = useMemo(
    () => logs.map((entry, index) => decorateLogEntry(entry, index)),
    [logs],
  );

  useEffect(() => {
    const nextLastLogId = decoratedLogs.length > 0
      ? decoratedLogs[decoratedLogs.length - 1].entry.id
      : null;

    setSelectedLogId((current) => {
      if (!nextLastLogId) {
        return null;
      }

      if (!current) {
        return nextLastLogId;
      }

      const stillExists = decoratedLogs.some((item) => item.entry.id === current);
      if (!stillExists) {
        return nextLastLogId;
      }

      if (current === previousLastLogIdRef.current) {
        return nextLastLogId;
      }

      return current;
    });

    previousLastLogIdRef.current = nextLastLogId;
  }, [decoratedLogs]);

  if (logs.length === 0) {
    return (
      <div className="flex h-full items-center justify-center px-4 text-sm text-muted-foreground">
        No execution logs yet.
      </div>
    );
  }

  const selectedEntry = decoratedLogs.find((item) => item.entry.id === selectedLogId)
    ?? decoratedLogs[decoratedLogs.length - 1];
  const websocketFrameCount = decoratedLogs.filter((item) => item.kind === "send" || item.kind === "receive").length;

  return (
    <div data-testid="studio-execution-log-split" className="flex min-h-0 min-w-0 flex-1 flex-row">
      <section
        aria-label="Trace timeline"
        className="flex min-h-0 min-w-0 flex-col border-r border-border bg-muted/10"
        style={{ width: "40%", minWidth: 280, maxWidth: 500 }}
      >
          <div className="flex shrink-0 items-center justify-between border-b border-border px-3 py-2">
            <PanelHeader
              title="Timeline Stream"
              className="flex-1"
              description={
                websocketFrameCount > 0
                  ? `${websocketFrameCount} frame${websocketFrameCount === 1 ? "" : "s"} captured • ${logs.length} total event${logs.length === 1 ? "" : "s"}`
                  : `${logs.length} event${logs.length === 1 ? "" : "s"}`
              }
              actions={
                <Badge
                  variant="outline"
                  className={cn(
                    "text-[10px] font-semibold uppercase",
                    websocketFrameCount > 0
                      ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-700"
                      : "border-border bg-muted/40 text-muted-foreground",
                  )}
                >
                  {websocketFrameCount > 0 ? "WS Trace" : "Execution"}
                </Badge>
              }
            />
          </div>
          <ScrollArea className="min-h-0 flex-1 [&>[data-radix-scroll-area-viewport]>div]:!block">
            <div className="space-y-1.5 p-2">
              {decoratedLogs.map((item) => {
                const isSelected = item.entry.id === selectedEntry.entry.id;

                return (
                  <button
                    key={item.entry.id}
                    type="button"
                    onClick={() => handleSelectLog(item.entry.id)}
                    className={cn(
                      "w-full rounded-md border px-2.5 py-1.5 text-left transition-colors",
                      "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                      isSelected
                        ? "border-sky-500/40 bg-sky-500/10 shadow-sm"
                        : "border-border bg-background hover:bg-muted/60",
                      item.kind === "error" && !isSelected && "border-red-500/30 bg-red-500/5",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <item.Icon className={cn("h-3.5 w-3.5 shrink-0", getIconClassName(item.kind, status))} />
                      <Badge variant="outline" className={cn("px-1.5 py-0 text-[10px] font-semibold uppercase", getBadgeClassName(item.kind))}>
                        {item.label}
                      </Badge>
                      <p
                        className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground"
                        title={item.title}
                      >
                          {item.title}
                      </p>
                      {item.sizeLabel && (
                        <span className="shrink-0 font-mono text-[10px] text-muted-foreground">
                          {item.sizeLabel}
                        </span>
                      )}
                    </div>

                    {item.preview && (
                      <p className="mt-1 truncate font-mono text-[10px] leading-4 text-muted-foreground">
                        {item.preview}
                      </p>
                    )}

                    <div className="mt-1 flex flex-wrap items-center gap-2 font-mono text-[10px] text-muted-foreground">
                      <span>{formatLogTime(item.entry.createdAt)}</span>
                      <span>#{item.index + 1}</span>
                      {typeof item.entry.rowCount === "number" && <span>{item.entry.rowCount} rows</span>}
                      {typeof item.entry.statementIndex === "number" && <span>statement {item.entry.statementIndex + 1}</span>}
                    </div>
                  </button>
                );
              })}
            </div>
          </ScrollArea>
      </section>

      <section aria-label="Trace details" className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
          <div className="min-w-0 border-b border-border px-4 py-3">
            <div className="flex min-w-0 items-center gap-2">
              <Badge variant="outline" className={cn("shrink-0 px-2 py-0.5 text-[10px] font-semibold uppercase", getBadgeClassName(selectedEntry.kind))}>
                {selectedEntry.label}
              </Badge>
              <h3 className="min-w-0 flex-1 truncate font-mono text-sm text-foreground">
                {selectedEntry.title}
              </h3>
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
              <span>{describeFlow(selectedEntry.kind)}</span>
              <span>{formatLogTime(selectedEntry.entry.createdAt)}</span>
              <span>#{selectedEntry.index + 1}</span>
              {selectedEntry.sizeLabel && <span>{selectedEntry.sizeLabel}</span>}
              {selectedEntry.entry.asUser && <span>as {selectedEntry.entry.asUser}</span>}
              {typeof selectedEntry.entry.rowCount === "number" && <span>{selectedEntry.entry.rowCount} rows</span>}
              {typeof selectedEntry.entry.statementIndex === "number" && <span>statement {selectedEntry.entry.statementIndex + 1}</span>}
            </div>
          </div>

          <div className={`flex shrink-0 items-center gap-2 border-b border-border px-4 py-2 ${chromeLabelClassName}`}>
            <span className="rounded border border-border bg-muted/40 px-2 py-0.5 text-[10px] font-semibold text-foreground">
              Preview
            </span>
            <span>Raw payload and structured detail for the selected entry</span>
          </div>

          <div className="min-h-0 min-w-0 flex-1 p-3">
            <CodeBlock value={selectedEntry.payload} jsonPreferred maxHeightClassName="h-full max-h-full" />
          </div>
      </section>
    </div>
  );
}
