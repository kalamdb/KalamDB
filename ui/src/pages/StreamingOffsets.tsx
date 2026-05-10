import { useEffect, useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { ChevronLeft, ChevronRight, RefreshCw, Search } from "lucide-react";
import { PageLayout } from "@/components/layout/PageLayout";
import { StreamingTabs } from "@/features/streaming/components/StreamingTabs";
import {
  useGetStreamingConsumerGroupsQuery,
  useGetStreamingOffsetsQuery,
  useGetStreamingTopicsQuery,
} from "@/store/apiSlice";
import { buildGroupSqlSnippet, buildTopicSqlSnippet } from "@/features/streaming/sql";
import {
  fetchStreamingTopicPartitionCursors,
  summarizeStreamingConsumerOffsets,
} from "@/features/streaming/service";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
import { formatNumber, formatTimestamp } from "@/lib/formatters";

const ALL_FILTERS = "__all__";
const DEFAULT_PAGE_SIZE = 25;
const PAGE_SIZE_OPTIONS = [10, 25, 50, 100];

function formatNullableTimestamp(value: string | null): string {
  if (!value) {
    return "-";
  }
  return formatTimestamp(value, undefined, "iso8601-datetime", "utc");
}

function partitionKey(topicId: string, partitionId: number): string {
  return `${topicId}:${partitionId}`;
}

export default function StreamingOffsets() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const initialTopic = searchParams.get("topic");
  const initialGroup = searchParams.get("group");
  const [selectedTopic, setSelectedTopic] = useState(initialTopic || ALL_FILTERS);
  const [selectedGroup, setSelectedGroup] = useState(initialGroup || ALL_FILTERS);
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
  const [headOffsetsByKey, setHeadOffsetsByKey] = useState<Record<string, number>>({});
  const [headOffsetsLoading, setHeadOffsetsLoading] = useState(false);
  const [headOffsetsError, setHeadOffsetsError] = useState<string | null>(null);

  const {
    data: topics = [],
    isFetching: topicsFetching,
    refetch: refetchTopics,
  } = useGetStreamingTopicsQuery();
  const {
    data: groups = [],
    isFetching: groupsFetching,
    refetch: refetchGroups,
  } = useGetStreamingConsumerGroupsQuery();

  const filters = useMemo(() => ({
    topicId: selectedTopic === ALL_FILTERS ? undefined : selectedTopic,
    groupId: selectedGroup === ALL_FILTERS ? undefined : selectedGroup,
    limit: 5000,
  }), [selectedGroup, selectedTopic]);

  const {
    data: offsets = [],
    isFetching,
    error,
    refetch,
  } = useGetStreamingOffsetsQuery(filters);

  const errorMessage =
    error && "error" in error && typeof error.error === "string"
      ? error.error
      : error
        ? "Failed to load offsets"
        : null;

  const selectedTopicId = selectedTopic === ALL_FILTERS ? null : selectedTopic;
  const selectedGroupId = selectedGroup === ALL_FILTERS ? null : selectedGroup;

  const summaries = useMemo(
    () => summarizeStreamingConsumerOffsets(topics, offsets),
    [offsets, topics],
  );

  const filteredSummaries = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) {
      return summaries;
    }

    return summaries.filter((summary) =>
      summary.groupId.toLowerCase().includes(query)
      || summary.topicId.toLowerCase().includes(query)
      || summary.topicName.toLowerCase().includes(query),
    );
  }, [search, summaries]);

  const totalPages = Math.max(1, Math.ceil(filteredSummaries.length / pageSize));

  useEffect(() => {
    setPage(0);
  }, [pageSize, search, selectedGroup, selectedTopic]);

  useEffect(() => {
    setPage((currentPage) => Math.min(currentPage, totalPages - 1));
  }, [totalPages]);

  const visibleSummaries = useMemo(() => {
    const start = page * pageSize;
    return filteredSummaries.slice(start, start + pageSize);
  }, [filteredSummaries, page, pageSize]);

  const visiblePartitions = useMemo(() => {
    const partitions = new Map<string, { topicId: string; partitionId: number }>();

    visibleSummaries.forEach((summary) => {
      for (let partitionId = 0; partitionId < summary.configuredPartitions; partitionId += 1) {
        partitions.set(partitionKey(summary.topicId, partitionId), {
          topicId: summary.topicId,
          partitionId,
        });
      }
    });

    return Array.from(partitions.values());
  }, [visibleSummaries]);

  const missingPartitions = useMemo(
    () => visiblePartitions.filter((partition) => !(partitionKey(partition.topicId, partition.partitionId) in headOffsetsByKey)),
    [headOffsetsByKey, visiblePartitions],
  );

  useEffect(() => {
    let cancelled = false;

    if (missingPartitions.length === 0) {
      if (visiblePartitions.length === 0) {
        setHeadOffsetsError(null);
      }
      return () => {
        cancelled = true;
      };
    }

    setHeadOffsetsLoading(true);
    setHeadOffsetsError(null);

    void fetchStreamingTopicPartitionCursors(missingPartitions)
      .then((cursors) => {
        if (cancelled) {
          return;
        }

        setHeadOffsetsByKey((current) => {
          const next = { ...current };
          cursors.forEach((cursor) => {
            next[partitionKey(cursor.topicId, cursor.partitionId)] = cursor.nextOffset;
          });
          return next;
        });
      })
      .catch((fetchError) => {
        if (cancelled) {
          return;
        }
        setHeadOffsetsError(
          fetchError instanceof Error ? fetchError.message : "Failed to load max offsets",
        );
      })
      .finally(() => {
        if (!cancelled) {
          setHeadOffsetsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [missingPartitions, visiblePartitions.length]);

  function getSummaryMaxOffset(topicId: string, configuredPartitions: number): number | null {
    let total = 0;

    for (let partitionId = 0; partitionId < configuredPartitions; partitionId += 1) {
      const value = headOffsetsByKey[partitionKey(topicId, partitionId)];
      if (value === undefined) {
        return null;
      }
      total += value;
    }

    return total;
  }

  const loading = isFetching || topicsFetching || groupsFetching || headOffsetsLoading;

  return (
    <PageLayout
      title="Streaming"
      description="Combined consumer cursor summary by group and topic"
      actions={(
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            setHeadOffsetsByKey({});
            void refetchTopics();
            void refetchGroups();
            void refetch();
          }}
          disabled={loading}
        >
          <RefreshCw className={`mr-1.5 h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          Refresh
        </Button>
      )}
    >
      <StreamingTabs />

      <div className="relative max-w-sm">
        <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
        <Input
          placeholder="Filter groups or topics..."
          className="pl-9"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
      </div>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">Filters</CardTitle>
          <CardDescription>Limit the combined consumer cursor list by topic and/or group</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-3 md:grid-cols-2">
          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-muted-foreground">Topic</p>
            <Select value={selectedTopic} onValueChange={setSelectedTopic}>
              <SelectTrigger>
                <SelectValue placeholder="All topics" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL_FILTERS}>All topics</SelectItem>
                {topics.map((topic) => (
                  <SelectItem key={topic.topicId} value={topic.topicId}>{topic.topicId}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-muted-foreground">Group</p>
            <Select value={selectedGroup} onValueChange={setSelectedGroup}>
              <SelectTrigger>
                <SelectValue placeholder="All groups" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL_FILTERS}>All groups</SelectItem>
                {groups.map((group) => (
                  <SelectItem key={group.groupId} value={group.groupId}>{group.groupId}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </CardContent>
      </Card>

      {errorMessage && (
        <Card className="border-destructive/30 bg-destructive/5">
          <CardContent className="pt-4 text-sm text-destructive">{errorMessage}</CardContent>
        </Card>
      )}

      {headOffsetsError && (
        <Card className="border-destructive/30 bg-destructive/5">
          <CardContent className="pt-4 text-sm text-destructive">
            Max offsets are unavailable right now. Committed offsets are still shown.
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">Consumers</CardTitle>
          <CardDescription>
            {filteredSummaries.length} group-topic row{filteredSummaries.length === 1 ? "" : "s"} returned. Partitions show claimed/configured coverage.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap items-center gap-2">
            {selectedTopicId && (
              <Button
                size="sm"
                variant="outline"
                onClick={() =>
                  navigate("/sql", {
                    state: {
                      prefillSql: buildTopicSqlSnippet(selectedTopicId),
                      prefillTitle: `Topic ${selectedTopicId}`,
                    },
                  })
                }
              >
                Open Topic SQL
              </Button>
            )}
            {selectedGroupId && (
              <Button
                size="sm"
                variant="outline"
                onClick={() =>
                  navigate("/sql", {
                    state: {
                      prefillSql: buildGroupSqlSnippet(selectedGroupId),
                      prefillTitle: `Group ${selectedGroupId}`,
                    },
                  })
                }
              >
                Open Group SQL
              </Button>
            )}
            </div>

            <div className="flex items-center gap-4">
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Select value={String(pageSize)} onValueChange={(value) => setPageSize(Number(value))}>
                  <SelectTrigger className="h-8 w-[76px]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {PAGE_SIZE_OPTIONS.map((size) => (
                      <SelectItem key={size} value={String(size)}>{size}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <span>per page</span>
              </div>

              <div className="flex items-center gap-1">
                <Button
                  variant="outline"
                  size="icon"
                  className="h-8 w-8"
                  disabled={page === 0}
                  onClick={() => setPage((currentPage) => Math.max(0, currentPage - 1))}
                >
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="px-2 text-sm text-muted-foreground">{page + 1} / {totalPages}</span>
                <Button
                  variant="outline"
                  size="icon"
                  className="h-8 w-8"
                  disabled={page >= totalPages - 1}
                  onClick={() => setPage((currentPage) => Math.min(totalPages - 1, currentPage + 1))}
                >
                  <ChevronRight className="h-4 w-4" />
                </Button>
              </div>
            </div>
          </div>

          <div className="rounded-md border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Consumer Group</TableHead>
                  <TableHead>Topic</TableHead>
                  <TableHead>Partitions</TableHead>
                  <TableHead className="text-right">Ack Offset</TableHead>
                  <TableHead className="text-right">Max Offset</TableHead>
                  <TableHead>Updated</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {visibleSummaries.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                      No consumer offsets found for the selected filters.
                    </TableCell>
                  </TableRow>
                ) : (
                  visibleSummaries.map((summary) => {
                    const maxOffset = getSummaryMaxOffset(summary.topicId, summary.configuredPartitions);
                    const partitionCoverage = summary.claimedPartitions === summary.configuredPartitions
                      ? formatNumber(summary.configuredPartitions)
                      : `${formatNumber(summary.claimedPartitions)}/${formatNumber(summary.configuredPartitions)}`;

                    return (
                    <TableRow key={`${summary.groupId}:${summary.topicId}`}>
                      <TableCell className="font-mono text-xs">{summary.groupId}</TableCell>
                      <TableCell>
                        <div className="space-y-0.5">
                          <div className="font-mono text-xs">{summary.topicId}</div>
                          {summary.topicName !== summary.topicId && (
                            <div className="text-xs text-muted-foreground">{summary.topicName}</div>
                          )}
                        </div>
                      </TableCell>
                      <TableCell>{partitionCoverage}</TableCell>
                      <TableCell className="text-right">{formatNumber(summary.committedOffset)}</TableCell>
                      <TableCell className="text-right">
                        {maxOffset !== null ? formatNumber(maxOffset) : (headOffsetsLoading ? "Loading..." : "-")}
                      </TableCell>
                      <TableCell className="font-mono text-xs">{formatNullableTimestamp(summary.updatedAt)}</TableCell>
                    </TableRow>
                    );
                  })
                )}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>
    </PageLayout>
  );
}

