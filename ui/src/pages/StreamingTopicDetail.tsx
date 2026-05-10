import { useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { ArrowLeft, Copy, RefreshCw } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { PageLayout } from "@/components/layout/PageLayout";
import { StreamingTabs } from "@/features/streaming/components/StreamingTabs";
import {
  useConsumeStreamingMessagesMutation,
  useGetStreamingOffsetsQuery,
  useGetStreamingTopicsQuery,
} from "@/store/apiSlice";
import {
  buildConsumeSqlSnippet,
  buildResetConsumerGroupSqlSnippet,
  buildTopicSqlSnippet,
} from "@/features/streaming/sql";
import { decodeTopicPayload } from "@/features/streaming/service";
import type { ConsumeReadMode, PayloadDecodeMode, StreamingMessage } from "@/features/streaming/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { CodeBlock } from "@/components/ui/code-block";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { formatTimestamp } from "@/lib/formatters";

function formatNullableTimestamp(value: string | null): string {
  if (!value) {
    return "-";
  }
  return formatTimestamp(value, undefined, "iso8601-datetime", "utc");
}

function formatMessageTimestamp(value: number): string {
  return formatTimestamp(value, "Timestamp(Millisecond, None)", "iso8601-datetime", "utc");
}

function toMessageKey(message: StreamingMessage): string {
  return `${message.partitionId}:${message.offset}`;
}

export default function StreamingTopicDetail() {
  const navigate = useNavigate();
  const { user } = useAuth();
  const params = useParams<{ topicId: string }>();
  const topicId = decodeURIComponent(params.topicId ?? "");

  const {
    data: topics = [],
    isFetching: topicsLoading,
    refetch: refetchTopics,
  } = useGetStreamingTopicsQuery();
  const {
    data: offsets = [],
    isFetching: offsetsLoading,
    refetch: refetchOffsets,
  } = useGetStreamingOffsetsQuery(topicId ? { topicId, limit: 2000 } : undefined, {
    skip: !topicId,
  });
  const [consumeMessages, consumeState] = useConsumeStreamingMessagesMutation();

  const selectedTopic = useMemo(() => topics.find((topic) => topic.topicId === topicId) ?? null, [topics, topicId]);

  const [readMode, setReadMode] = useState<ConsumeReadMode>("Inspect");
  const [groupId, setGroupId] = useState(`ui-debug-${user?.username ?? "admin"}`);
  const [partitionId, setPartitionId] = useState("0");
  const [startMode, setStartMode] = useState<"Latest" | "Earliest" | "Offset">("Latest");
  const [offsetValue, setOffsetValue] = useState("0");
  const [limitValue, setLimitValue] = useState("100");
  const [timeoutValue, setTimeoutValue] = useState("5");
  const [decodeMode, setDecodeMode] = useState<PayloadDecodeMode>("auto-json");
  const [messages, setMessages] = useState<StreamingMessage[]>([]);
  const [selectedMessageKey, setSelectedMessageKey] = useState<string | null>(null);
  const [lastFetchedAt, setLastFetchedAt] = useState<string | null>(null);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [hasMore, setHasMore] = useState<boolean>(false);

  const selectedMessage = useMemo(
    () => messages.find((message) => toMessageKey(message) === selectedMessageKey) ?? null,
    [messages, selectedMessageKey],
  );

  const decodedPayload = useMemo(
    () => (selectedMessage ? decodeTopicPayload(selectedMessage.payloadBase64, decodeMode) : null),
    [decodeMode, selectedMessage],
  );

  const consumeErrorMessage =
    consumeState.error && "error" in consumeState.error && typeof consumeState.error.error === "string"
      ? consumeState.error.error
      : consumeState.error
        ? "Failed to consume topic messages"
        : null;

  const fetchMessages = async () => {
    if (!topicId) {
      return;
    }

    const limit = Math.min(Math.max(Number(limitValue) || 100, 1), 500);
    const timeoutSeconds = Math.min(Math.max(Number(timeoutValue) || 5, 1), 30);
    const parsedPartitionId = Math.max(Number(partitionId) || 0, 0);
    const parsedOffset = Math.max(Number(offsetValue) || 0, 0);

    const batch = await consumeMessages({
      topicId,
      groupId: readMode === "Group" ? groupId.trim() || "ui-debug-admin" : undefined,
      partitionId: parsedPartitionId,
      startMode,
      offset: startMode === "Offset" ? parsedOffset : undefined,
      limit,
      timeoutSeconds,
    }).unwrap();

    setMessages(batch.messages);
    setNextOffset(batch.nextOffset);
    setHasMore(batch.hasMore);
    setLastFetchedAt(new Date().toISOString());
    setSelectedMessageKey(batch.messages[0] ? toMessageKey(batch.messages[0]) : null);
  };

  return (
    <PageLayout
      title={topicId ? `Streaming / ${topicId}` : "Streaming"}
      description="Inspect messages and offset snapshots for a single topic"
      actions={(
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => navigate("/streaming/topics")}>
            <ArrowLeft className="mr-1.5 h-4 w-4" />
            Back to Topics
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void refetchTopics();
              void refetchOffsets();
            }}
            disabled={topicsLoading || offsetsLoading}
          >
            <RefreshCw className={`mr-1.5 h-4 w-4 ${(topicsLoading || offsetsLoading) ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        </div>
      )}
    >
      <StreamingTabs />

      {!selectedTopic ? (
        <Card>
          <CardHeader>
            <CardTitle>Topic Not Found</CardTitle>
            <CardDescription>No topic metadata found for {topicId || "the selected topic"}.</CardDescription>
          </CardHeader>
        </Card>
      ) : (
        <>
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-base">Topic Summary</CardTitle>
              <CardDescription className="font-mono text-xs">{selectedTopic.topicId}</CardDescription>
            </CardHeader>
            <CardContent className="grid gap-3 md:grid-cols-4">
              <div>
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Partitions</p>
                <p className="font-medium">{selectedTopic.partitions}</p>
              </div>
              <div>
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Routes</p>
                <p className="font-medium">{selectedTopic.routeCount}</p>
              </div>
              <div>
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Created</p>
                <p className="font-mono text-xs">{formatNullableTimestamp(selectedTopic.createdAt)}</p>
              </div>
              <div>
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Updated</p>
                <p className="font-mono text-xs">{formatNullableTimestamp(selectedTopic.updatedAt)}</p>
              </div>
              <div className="md:col-span-4 flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    navigate("/sql", {
                      state: {
                        prefillSql: buildTopicSqlSnippet(selectedTopic.topicId),
                        prefillTitle: `Topic ${selectedTopic.topicId}`,
                      },
                    })
                  }
                >
                  Open Topic SQL
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    navigate("/sql", {
                      state: {
                        prefillSql: buildConsumeSqlSnippet(
                          selectedTopic.topicId,
                          null,
                          startMode,
                          Number(offsetValue),
                          Number(limitValue) || 100,
                        ),
                        prefillTitle: `Inspect ${selectedTopic.topicId}`,
                      },
                    })
                  }
                >
                  Open Inspect SQL
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    navigate("/sql", {
                      state: {
                        prefillSql: buildConsumeSqlSnippet(
                          selectedTopic.topicId,
                          groupId.trim() || "ui-debug-admin",
                          startMode,
                          Number(offsetValue),
                          Number(limitValue) || 100,
                        ),
                        prefillTitle: `Group consume ${selectedTopic.topicId}`,
                      },
                    })
                  }
                >
                  Open Group SQL
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    navigate("/sql", {
                      state: {
                        prefillSql: buildResetConsumerGroupSqlSnippet(
                          selectedTopic.topicId,
                          groupId.trim() || "ui-debug-admin",
                          Number(partitionId) || 0,
                          Number(offsetValue) || 0,
                        ),
                        prefillTitle: `Reset ${groupId.trim() || "ui-debug-admin"}`,
                      },
                    })
                  }
                >
                  Open Reset SQL
                </Button>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-base">Message Inspector Controls</CardTitle>
              <CardDescription>Consume a bounded window from this topic for inspection</CardDescription>
            </CardHeader>
            <CardContent className="grid gap-3 md:grid-cols-6">
              <div className="space-y-1">
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Mode</p>
                <Select value={readMode} onValueChange={(value) => setReadMode(value as ConsumeReadMode)}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="Inspect">Inspect</SelectItem>
                    <SelectItem value="Group">Group</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1 md:col-span-2">
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Group ID</p>
                <Input
                  value={groupId}
                  onChange={(event) => setGroupId(event.target.value)}
                  disabled={readMode !== "Group"}
                />
              </div>
              <div className="space-y-1">
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Partition</p>
                <Select value={partitionId} onValueChange={setPartitionId}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {Array.from({ length: selectedTopic.partitions }).map((_, index) => (
                      <SelectItem key={index} value={String(index)}>{index}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1">
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Start</p>
                <Select value={startMode} onValueChange={(value) => setStartMode(value as "Latest" | "Earliest" | "Offset")}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="Latest">Latest</SelectItem>
                    <SelectItem value="Earliest">Earliest</SelectItem>
                    <SelectItem value="Offset">Offset</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1">
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Offset</p>
                <Input
                  value={offsetValue}
                  onChange={(event) => setOffsetValue(event.target.value)}
                  disabled={startMode !== "Offset"}
                />
              </div>
              <div className="space-y-1">
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Limit</p>
                <Input value={limitValue} onChange={(event) => setLimitValue(event.target.value)} />
              </div>
              <div className="space-y-1">
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Timeout (s)</p>
                <Input value={timeoutValue} onChange={(event) => setTimeoutValue(event.target.value)} />
              </div>
              <div className="space-y-1">
                <p className="text-xs uppercase tracking-wide text-muted-foreground">Decode</p>
                <Select value={decodeMode} onValueChange={(value) => setDecodeMode(value as PayloadDecodeMode)}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto-json">auto-json</SelectItem>
                    <SelectItem value="text">text</SelectItem>
                    <SelectItem value="base64">base64</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex items-end md:col-span-2">
                <Button onClick={() => void fetchMessages()} disabled={consumeState.isLoading}>
                  {consumeState.isLoading ? "Fetching..." : readMode === "Group" ? "Consume" : "Inspect"}
                </Button>
              </div>
              <div className="text-xs text-muted-foreground md:col-span-6">
                {readMode === "Group" ? "Group mode claims offsets for the selected group. " : "Inspect mode does not use a group cursor. "}
                {lastFetchedAt ? `Last fetched: ${formatNullableTimestamp(lastFetchedAt)}` : "No messages fetched yet."}
                {nextOffset !== null ? `  |  Next offset: ${nextOffset}` : ""}
                {nextOffset !== null ? `  |  Has more: ${hasMore ? "yes" : "no"}` : ""}
              </div>
            </CardContent>
          </Card>

          {consumeErrorMessage && (
            <Card className="border-destructive/30 bg-destructive/5">
              <CardContent className="pt-4 text-sm text-destructive">{consumeErrorMessage}</CardContent>
            </Card>
          )}

          <div className="grid gap-4 xl:grid-cols-[1.3fr_1fr]">
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-base">Messages</CardTitle>
                <CardDescription>{messages.length} row{messages.length === 1 ? "" : "s"}</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="rounded-md border">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead>Offset</TableHead>
                        <TableHead>Partition</TableHead>
                        <TableHead>Op</TableHead>
                        <TableHead>Timestamp</TableHead>
                        <TableHead>Key</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {messages.length === 0 ? (
                        <TableRow>
                          <TableCell colSpan={5} className="py-8 text-center text-muted-foreground">
                            No messages returned in the selected window.
                          </TableCell>
                        </TableRow>
                      ) : (
                        messages.map((message) => {
                          const rowKey = toMessageKey(message);
                          const isSelected = selectedMessageKey === rowKey;
                          return (
                            <TableRow
                              key={rowKey}
                              data-state={isSelected ? "selected" : undefined}
                              className="cursor-pointer"
                              onClick={() => setSelectedMessageKey(rowKey)}
                            >
                              <TableCell>{message.offset}</TableCell>
                              <TableCell>{message.partitionId}</TableCell>
                              <TableCell>{message.op}</TableCell>
                              <TableCell className="font-mono text-xs">{formatMessageTimestamp(message.timestampMs)}</TableCell>
                              <TableCell className="max-w-[220px] truncate font-mono text-xs">{message.key ?? "-"}</TableCell>
                            </TableRow>
                          );
                        })
                      )}
                    </TableBody>
                  </Table>
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-base">Message Payload</CardTitle>
                <CardDescription>
                  {selectedMessage ? `Offset ${selectedMessage.offset} | ${selectedMessage.op}` : "Select a message row"}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {selectedMessage && decodedPayload ? (
                  <>
                    <div className="grid gap-2 text-xs">
                      <div className="flex items-center justify-between">
                        <span className="text-muted-foreground">User</span>
                        <span className="font-mono">{selectedMessage.username ?? "-"}</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-muted-foreground">Partition</span>
                        <span className="font-mono">{selectedMessage.partitionId}</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-muted-foreground">Timestamp</span>
                        <span className="font-mono">{formatMessageTimestamp(selectedMessage.timestampMs)}</span>
                      </div>
                    </div>

                    {decodedPayload.error && (
                      <p className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                        Decode error: {decodedPayload.error}
                      </p>
                    )}

                    <CodeBlock
                      value={decodedPayload.prettyJson ?? decodedPayload.text ?? decodedPayload.base64}
                      jsonPreferred={Boolean(decodedPayload.prettyJson)}
                      maxHeightClassName="max-h-[420px]"
                    />

                    <div className="flex items-center gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => {
                          navigator.clipboard
                            .writeText(decodedPayload.prettyJson ?? decodedPayload.text ?? decodedPayload.base64)
                            .catch(() => undefined);
                        }}
                      >
                        <Copy className="mr-1.5 h-3.5 w-3.5" />
                        Copy Payload
                      </Button>
                    </div>
                  </>
                ) : (
                  <p className="text-sm text-muted-foreground">Select a message to inspect decoded payload.</p>
                )}
              </CardContent>
            </Card>
          </div>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-base">Committed Offsets Snapshot</CardTitle>
              <CardDescription>{offsets.length} row{offsets.length === 1 ? "" : "s"} for {selectedTopic.topicId}</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="rounded-md border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Group</TableHead>
                      <TableHead>Partition</TableHead>
                      <TableHead>Last Acked</TableHead>
                      <TableHead>Next</TableHead>
                      <TableHead>Updated</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {offsets.length === 0 ? (
                      <TableRow>
                        <TableCell colSpan={5} className="py-8 text-center text-muted-foreground">
                          No committed offsets for this topic yet.
                        </TableCell>
                      </TableRow>
                    ) : (
                      offsets.map((offset) => (
                        <TableRow key={`${offset.groupId}:${offset.partitionId}`}>
                          <TableCell className="font-mono text-xs">{offset.groupId}</TableCell>
                          <TableCell>{offset.partitionId}</TableCell>
                          <TableCell>{offset.lastAckedOffset}</TableCell>
                          <TableCell>{offset.nextOffset}</TableCell>
                          <TableCell className="font-mono text-xs">{formatNullableTimestamp(offset.updatedAt)}</TableCell>
                        </TableRow>
                      ))
                    )}
                  </TableBody>
                </Table>
              </div>
            </CardContent>
          </Card>
        </>
      )}
    </PageLayout>
  );
}

