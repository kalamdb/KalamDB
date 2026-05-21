import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { ArrowLeft, Copy, FileCode2, RefreshCw } from "lucide-react";
import { useAuth } from "@/lib/auth";
import { PageLayout } from "@/components/layout/PageLayout";
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
import type { ConsumeReadMode, ConsumeStartMode, PayloadDecodeMode, StreamingMessage } from "@/features/streaming/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { CodeBlock } from "@/components/ui/code-block";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { formatDate, formatTimestamp } from "@/lib/formatters";

function formatNullableTimestamp(value: string | null): string {
  if (!value) {
    return "-";
  }
  return formatTimestamp(value, undefined, "iso8601-datetime", "utc");
}

function formatMessageTimestamp(value: number): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return String(value);
  }
  return formatDate(date, "iso8601-datetime", "utc");
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
  const partitionOptions = useMemo(
    () => Array.from({ length: selectedTopic?.partitions ?? 0 }, (_, index) => String(index)),
    [selectedTopic?.partitions],
  );

  const [readMode, setReadMode] = useState<ConsumeReadMode>("Inspect");
  const [groupId, setGroupId] = useState(`ui-debug-${user?.username ?? "admin"}`);
  const [partitionId, setPartitionId] = useState("0");
  const [startMode, setStartMode] = useState<ConsumeStartMode>("Offset");
  const [offsetValue, setOffsetValue] = useState("0");
  const [limitValue, setLimitValue] = useState("100");
  const [timeoutValue, setTimeoutValue] = useState("5");
  const [decodeMode, setDecodeMode] = useState<PayloadDecodeMode>("auto-json");
  const [messages, setMessages] = useState<StreamingMessage[]>([]);
  const [selectedMessageKey, setSelectedMessageKey] = useState<string | null>(null);
  const [lastFetchedAt, setLastFetchedAt] = useState<string | null>(null);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [hasMore, setHasMore] = useState<boolean>(false);

  useEffect(() => {
    if (partitionOptions.length === 0) {
      return;
    }

    setPartitionId((currentPartitionId) => (
      partitionOptions.includes(currentPartitionId) ? currentPartitionId : partitionOptions[0]
    ));
  }, [partitionOptions]);

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
      title={topicId ? "Streaming Topic" : "Streaming"}
      description="Inspect messages and offset snapshots for a single topic"
      actions={(
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void refetchTopics();
              void refetchOffsets();
            }}
            disabled={topicsLoading || offsetsLoading}
          >
            <RefreshCw data-icon="inline-start" className={(topicsLoading || offsetsLoading) ? "animate-spin" : undefined} />
            Refresh
          </Button>
        </div>
      )}
    >
      <nav aria-label="Breadcrumb" className="flex flex-wrap items-center gap-2 text-sm">
        <span className="text-muted-foreground">Streaming</span>
        <span className="text-muted-foreground">/</span>
        <Button variant="ghost" size="sm" className="h-7 px-2" onClick={() => navigate("/streaming/topics")}>
          <ArrowLeft data-icon="inline-start" />
          Topics
        </Button>
        {topicId ? (
          <>
            <span className="text-muted-foreground">/</span>
            <span aria-current="page" className="min-w-0 truncate font-mono text-xs text-foreground">
              {topicId}
            </span>
          </>
        ) : null}
      </nav>

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
              <div className="flex flex-wrap gap-1.5 border-t pt-2 md:col-span-4">
                <Button
                  size="xs"
                  variant="ghost"
                  onClick={() =>
                    navigate("/sql", {
                      state: {
                        prefillSql: buildTopicSqlSnippet(selectedTopic.topicId),
                        prefillTitle: `Topic ${selectedTopic.topicId}`,
                      },
                    })
                  }
                >
                  <FileCode2 data-icon="inline-start" />
                  Open Topic SQL
                </Button>
                <Button
                  size="xs"
                  variant="ghost"
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
                  <FileCode2 data-icon="inline-start" />
                  Open Inspect SQL
                </Button>
                <Button
                  size="xs"
                  variant="ghost"
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
                  <FileCode2 data-icon="inline-start" />
                  Open Group SQL
                </Button>
                <Button
                  size="xs"
                  variant="ghost"
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
                  <FileCode2 data-icon="inline-start" />
                  Open Reset SQL
                </Button>
              </div>
            </CardContent>
          </Card>

          <Tabs defaultValue="messages" className="gap-4">
            <TabsList variant="line" className="w-full justify-start overflow-x-auto">
              <TabsTrigger value="messages">Inspect Messages</TabsTrigger>
              <TabsTrigger value="offsets">Committed Offsets</TabsTrigger>
            </TabsList>

            <TabsContent value="messages" className="mt-0 flex flex-col gap-4">
              <Card>
                <CardHeader className="pb-2">
                  <CardTitle className="text-base">Inspect Messages</CardTitle>
                  <CardDescription>{messages.length} row{messages.length === 1 ? "" : "s"} in the current inspector result</CardDescription>
                </CardHeader>
                <CardContent className="flex flex-col gap-4">
                  <section className="rounded-md border bg-muted/20 p-3" aria-labelledby="message-inspector-controls-title">
                    <div className="mb-3 flex flex-wrap items-start justify-between gap-2">
                      <div className="flex flex-col gap-1">
                        <h2 id="message-inspector-controls-title" className="text-sm font-medium">
                          Message Inspector Controls
                        </h2>
                        <p className="text-xs text-muted-foreground">These controls drive the messages table and payload view below.</p>
                      </div>
                    </div>

                    <div className="grid gap-3 md:grid-cols-6">
                      <div className="flex flex-col gap-1">
                        <p className="text-xs uppercase tracking-wide text-muted-foreground">Mode</p>
                        <Select value={readMode} onValueChange={(value) => setReadMode(value as ConsumeReadMode)}>
                          <SelectTrigger aria-label="Mode">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectGroup>
                              <SelectItem value="Inspect">Inspect</SelectItem>
                              <SelectItem value="Group">Group</SelectItem>
                            </SelectGroup>
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="flex flex-col gap-1 md:col-span-2">
                        <p className="text-xs uppercase tracking-wide text-muted-foreground">Group ID</p>
                        <Input
                          aria-label="Group ID"
                          value={groupId}
                          onChange={(event) => setGroupId(event.target.value)}
                          disabled={readMode !== "Group"}
                        />
                      </div>
                      <div className="flex flex-col gap-1">
                        <p className="text-xs uppercase tracking-wide text-muted-foreground">Partition</p>
                        <Select value={partitionId} onValueChange={setPartitionId}>
                          <SelectTrigger aria-label="Partition">
                            <SelectValue placeholder={partitionOptions[0] ?? "0"} />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectGroup>
                              {partitionOptions.map((option) => (
                                <SelectItem key={option} value={option}>{option}</SelectItem>
                              ))}
                            </SelectGroup>
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="flex flex-col gap-1">
                        <p className="text-xs uppercase tracking-wide text-muted-foreground">Start</p>
                        <Select value={startMode} onValueChange={(value) => setStartMode(value as ConsumeStartMode)}>
                          <SelectTrigger aria-label="Start">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectGroup>
                              <SelectItem value="Offset">Offset</SelectItem>
                              <SelectItem value="Earliest">Earliest</SelectItem>
                              <SelectItem value="Latest">Latest</SelectItem>
                            </SelectGroup>
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="flex flex-col gap-1">
                        <p className="text-xs uppercase tracking-wide text-muted-foreground">Offset</p>
                        <Input
                          aria-label="Offset"
                          value={offsetValue}
                          onChange={(event) => setOffsetValue(event.target.value)}
                          disabled={startMode !== "Offset"}
                        />
                      </div>
                      <div className="flex flex-col gap-1">
                        <p className="text-xs uppercase tracking-wide text-muted-foreground">Limit</p>
                        <Input aria-label="Limit" value={limitValue} onChange={(event) => setLimitValue(event.target.value)} />
                      </div>
                      <div className="flex flex-col gap-1">
                        <p className="text-xs uppercase tracking-wide text-muted-foreground">Timeout (s)</p>
                        <Input aria-label="Timeout" value={timeoutValue} onChange={(event) => setTimeoutValue(event.target.value)} />
                      </div>
                      <div className="flex flex-col gap-1">
                        <p className="text-xs uppercase tracking-wide text-muted-foreground">Decode</p>
                        <Select value={decodeMode} onValueChange={(value) => setDecodeMode(value as PayloadDecodeMode)}>
                          <SelectTrigger aria-label="Decode">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectGroup>
                              <SelectItem value="auto-json">auto-json</SelectItem>
                              <SelectItem value="text">text</SelectItem>
                              <SelectItem value="base64">base64</SelectItem>
                            </SelectGroup>
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
                    </div>
                  </section>

                  {consumeErrorMessage && (
                    <Alert variant="destructive">
                      <AlertDescription>{consumeErrorMessage}</AlertDescription>
                    </Alert>
                  )}

                  <div className="grid gap-4 xl:grid-cols-[1.3fr_1fr]">
                    <section className="flex min-w-0 flex-col gap-2" aria-labelledby="topic-messages-title">
                      <div className="flex flex-wrap items-end justify-between gap-2">
                        <div>
                          <h2 id="topic-messages-title" className="text-sm font-medium">Messages</h2>
                          <p className="text-xs text-muted-foreground">{messages.length} row{messages.length === 1 ? "" : "s"}</p>
                        </div>
                      </div>
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
                    </section>

                    <aside className="flex min-w-0 flex-col gap-3 rounded-md border p-3" aria-labelledby="message-payload-title">
                      <div className="flex flex-col gap-1">
                        <h2 id="message-payload-title" className="text-sm font-medium">Message Payload</h2>
                        <p className="text-xs text-muted-foreground">
                          {selectedMessage ? `Offset ${selectedMessage.offset} | ${selectedMessage.op}` : "Select a message row"}
                        </p>
                      </div>
                      {selectedMessage && decodedPayload ? (
                        <>
                          <div className="grid gap-2 text-xs">
                            <div className="flex items-center justify-between gap-3">
                              <span className="text-muted-foreground">User</span>
                              <span className="truncate font-mono">{selectedMessage.username ?? "-"}</span>
                            </div>
                            <div className="flex items-center justify-between gap-3">
                              <span className="text-muted-foreground">Partition</span>
                              <span className="font-mono">{selectedMessage.partitionId}</span>
                            </div>
                            <div className="flex items-center justify-between gap-3">
                              <span className="text-muted-foreground">Timestamp</span>
                              <span className="font-mono">{formatMessageTimestamp(selectedMessage.timestampMs)}</span>
                            </div>
                          </div>

                          {decodedPayload.error && (
                            <Alert variant="destructive">
                              <AlertDescription>Decode error: {decodedPayload.error}</AlertDescription>
                            </Alert>
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
                              <Copy data-icon="inline-start" />
                              Copy Payload
                            </Button>
                          </div>
                        </>
                      ) : (
                        <p className="text-sm text-muted-foreground">Select a message to inspect decoded payload.</p>
                      )}
                    </aside>
                  </div>
                </CardContent>
              </Card>
            </TabsContent>

            <TabsContent value="offsets" className="mt-0">
              <Card>
                <CardHeader className="pb-2">
                  <CardTitle className="text-base">Committed Offsets</CardTitle>
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
            </TabsContent>
          </Tabs>
        </>
      )}
    </PageLayout>
  );
}

