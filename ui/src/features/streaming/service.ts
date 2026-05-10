import { api } from "@/lib/api";
import { getDb } from "@/lib/db";
import { system_topic_offsets, system_topics } from "@/lib/schema";
import {
  type StreamingOffsetsFilter,
} from "@/features/streaming/sql";
import type {
  ConsumedMessageBatch,
  ConsumeApiResponse,
  ConsumeMessagesInput,
  DecodedPayload,
  PayloadDecodeMode,
  StreamingConsumerOffsetSummary,
  StreamingConsumerGroup,
  StreamingMessage,
  StreamingOffset,
  StreamingTopicPartitionCursor,
  StreamingTopic,
  StreamingTopicOffsetRow,
  StreamingTopicRoute,
  StreamingTopicRow,
} from "@/features/streaming/types";
import { and, asc, desc, eq, type SQL } from "drizzle-orm";

function asString(value: unknown): string {
  if (value === null || value === undefined) {
    return "";
  }
  return String(value);
}

function asNullableString(value: unknown): string | null {
  if (value === null || value === undefined) {
    return null;
  }
  const text = String(value);
  return text.length > 0 ? text : null;
}

function asNumber(value: unknown): number {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : 0;
  }
  if (typeof value === "string") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : 0;
  }
  return 0;
}

function asNullableNumber(value: unknown): number | null {
  if (value === null || value === undefined) {
    return null;
  }
  const parsed = asNumber(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function tableIdToString(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  if (value && typeof value === "object") {
    const item = value as {
      namespace_id?: unknown;
      namespace?: unknown;
      table_name?: unknown;
      table?: unknown;
    };
    const namespace = asString(item.namespace_id ?? item.namespace);
    const tableName = asString(item.table_name ?? item.table);
    if (namespace && tableName) {
      return `${namespace}.${tableName}`;
    }
  }
  return "";
}

function parseJsonString(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

export function parseTopicRoutes(value: unknown): StreamingTopicRoute[] {
  const fromObject = (route: Record<string, unknown>): StreamingTopicRoute => ({
    tableId: tableIdToString(route.table_id),
    op: asString(route.op),
    payloadMode: asString(route.payload_mode),
    filterExpr: asNullableString(route.filter_expr),
    partitionKeyExpr: asNullableString(route.partition_key_expr),
  });

  if (Array.isArray(value)) {
    return value.filter((item): item is Record<string, unknown> => typeof item === "object" && item !== null).map(fromObject);
  }
  if (typeof value === "string") {
    const parsed = parseJsonString(value);
    if (Array.isArray(parsed)) {
      return parsed
        .filter((item): item is Record<string, unknown> => typeof item === "object" && item !== null)
        .map(fromObject);
    }
  }
  return [];
}

export function mapTopicRows(rows: StreamingTopicRow[]): StreamingTopic[] {
  return rows.map((row) => {
    const routes = parseTopicRoutes(row.routes);
    return {
      topicId: row.topic_id,
      name: row.name || row.topic_id,
      partitions: Math.max(1, row.partitions),
      retentionSeconds: asNullableNumber(row.retention_seconds),
      retentionMaxBytes: asNullableNumber(row.retention_max_bytes),
      routeCount: routes.length,
      routes,
      createdAt: row.created_at ?? null,
      updatedAt: row.updated_at ?? null,
    };
  });
}

export function mapOffsetRows(rows: StreamingTopicOffsetRow[]): StreamingOffset[] {
  return rows.map((row) => {
    const lastAckedOffset = asNumber(row.last_acked_offset);
    return {
      topicId: row.topic_id,
      groupId: row.group_id,
      partitionId: row.partition_id,
      lastAckedOffset,
      nextOffset: lastAckedOffset + 1,
      updatedAt: row.updated_at ?? null,
    };
  });
}

export function summarizeStreamingConsumerOffsets(
  topics: StreamingTopic[],
  offsets: StreamingOffset[],
): StreamingConsumerOffsetSummary[] {
  const topicById = new Map(topics.map((topic) => [topic.topicId, topic]));
  const summaries = new Map<string, StreamingConsumerOffsetSummary>();

  offsets.forEach((offset) => {
    const topic = topicById.get(offset.topicId);
    const key = `${offset.groupId}:${offset.topicId}`;
    const existing = summaries.get(key) ?? {
      topicId: offset.topicId,
      topicName: topic?.name ?? offset.topicId,
      groupId: offset.groupId,
      claimedPartitions: 0,
      configuredPartitions: topic?.partitions ?? 0,
      lastAckedOffset: 0,
      committedOffset: 0,
      updatedAt: null,
    };

    existing.claimedPartitions += 1;
    existing.configuredPartitions = Math.max(
      existing.configuredPartitions,
      topic?.partitions ?? existing.claimedPartitions,
    );
    existing.lastAckedOffset += offset.lastAckedOffset;
    existing.committedOffset += offset.nextOffset;

    if (offset.updatedAt && (!existing.updatedAt || offset.updatedAt > existing.updatedAt)) {
      existing.updatedAt = offset.updatedAt;
    }

    summaries.set(key, existing);
  });

  return Array.from(summaries.values()).sort((left, right) => {
    const leftUpdated = left.updatedAt ?? "";
    const rightUpdated = right.updatedAt ?? "";
    if (leftUpdated !== rightUpdated) {
      return rightUpdated.localeCompare(leftUpdated);
    }

    const groupCompare = left.groupId.localeCompare(right.groupId);
    if (groupCompare !== 0) {
      return groupCompare;
    }

    return left.topicId.localeCompare(right.topicId);
  });
}

interface LatestOffsetsApiResponse {
  offsets: Array<{
    topic_id: string;
    partition_id: number;
    next_offset: number;
    last_offset: number | null;
  }>;
}

export async function fetchStreamingTopicPartitionCursors(
  partitions: Array<Pick<StreamingTopicPartitionCursor, "topicId" | "partitionId">>,
): Promise<StreamingTopicPartitionCursor[]> {
  const uniquePartitions = new Map<string, Pick<StreamingTopicPartitionCursor, "topicId" | "partitionId">>();

  partitions.forEach((partition) => {
    uniquePartitions.set(`${partition.topicId}:${partition.partitionId}`, partition);
  });

  if (uniquePartitions.size === 0) {
    return [];
  }

  const response = await api.post<LatestOffsetsApiResponse>("/topics/latest-offsets", {
    partitions: Array.from(uniquePartitions.values()).map((partition) => ({
      topic_id: partition.topicId,
      partition_id: partition.partitionId,
    })),
  });

  return response.offsets.map((offset) => ({
    topicId: offset.topic_id,
    partitionId: Number(offset.partition_id ?? 0),
    nextOffset: Number(offset.next_offset ?? 0),
    lastOffset: offset.last_offset === null || offset.last_offset === undefined
      ? null
      : Number(offset.last_offset),
  }));
}

export async function fetchStreamingTopics(): Promise<StreamingTopic[]> {
  const db = getDb();
  const rows = await db
    .select()
    .from(system_topics)
    .orderBy(asc(system_topics.topic_id));

  return mapTopicRows(rows);
}

export async function fetchStreamingConsumerGroups(): Promise<StreamingConsumerGroup[]> {
  const db = getDb();
  const rows = await db
    .select()
    .from(system_topic_offsets)
    .orderBy(
      desc(system_topic_offsets.updated_at),
      asc(system_topic_offsets.group_id),
      asc(system_topic_offsets.topic_id),
      asc(system_topic_offsets.partition_id),
    );

  const groups = new Map<string, StreamingConsumerGroup & { topics: Set<StreamingTopicOffsetRow["topic_id"]> }>();

  rows.forEach((row) => {
    const groupId = row.group_id;
    if (!groupId) {
      return;
    }

    const existing = groups.get(groupId) ?? {
      groupId,
      topicCount: 0,
      partitionCount: 0,
      lastUpdatedAt: null,
      topics: new Set<string>(),
    };

    existing.partitionCount += 1;
    existing.topics.add(row.topic_id);

    const updatedAt = row.updated_at ?? null;
    if (updatedAt && (!existing.lastUpdatedAt || updatedAt > existing.lastUpdatedAt)) {
      existing.lastUpdatedAt = updatedAt;
    }

    groups.set(groupId, existing);
  });

  return Array.from(groups.values())
    .map(({ topics, ...group }) => ({
      ...group,
      topicCount: topics.size,
    }))
    .sort((left, right) => {
      const leftUpdated = left.lastUpdatedAt ?? "";
      const rightUpdated = right.lastUpdatedAt ?? "";

      if (leftUpdated === rightUpdated) {
        return left.groupId.localeCompare(right.groupId);
      }

      return rightUpdated.localeCompare(leftUpdated);
    })
    .slice(0, 500);
}

export async function fetchStreamingOffsets(filters?: StreamingOffsetsFilter): Promise<StreamingOffset[]> {
  const db = getDb();
  const conditions: SQL[] = [];

  if (filters?.topicId) {
    conditions.push(eq(system_topic_offsets.topic_id, filters.topicId));
  }
  if (filters?.groupId) {
    conditions.push(eq(system_topic_offsets.group_id, filters.groupId));
  }

  const rows = await db
    .select()
    .from(system_topic_offsets)
    .where(conditions.length > 0 ? and(...conditions) : undefined)
    .orderBy(
      desc(system_topic_offsets.updated_at),
      asc(system_topic_offsets.topic_id),
      asc(system_topic_offsets.group_id),
      asc(system_topic_offsets.partition_id),
    )
    .limit(Math.min(Math.max(filters?.limit ?? 1000, 1), 5000));

  return mapOffsetRows(rows);
}

export function buildConsumeRequestBody(input: ConsumeMessagesInput): Record<string, unknown> {
  const body: Record<string, unknown> = {
    topic_id: input.topicId,
    partition_id: input.partitionId,
    limit: input.limit,
    timeout_seconds: input.timeoutSeconds,
  };

  if (input.groupId?.trim()) {
    body.group_id = input.groupId.trim();
  }

  if (input.startMode === "Offset") {
    body.start = { Offset: Math.max(0, input.offset ?? 0) };
    return body;
  }

  body.start = input.startMode;
  return body;
}

export async function consumeTopicMessages(input: ConsumeMessagesInput): Promise<ConsumedMessageBatch> {
  const response = await api.post<ConsumeApiResponse>("/topics/consume", buildConsumeRequestBody(input));
  return {
    messages: response.messages.map((message): StreamingMessage => ({
      topicId: message.topic_id,
      partitionId: Number(message.partition_id ?? 0),
      offset: Number(message.offset ?? 0),
      payloadBase64: message.payload,
      key: message.key ?? null,
      timestampMs: Number(message.timestamp_ms ?? 0),
      username: message.username ?? null,
      op: message.op,
    })),
    nextOffset: Number(response.next_offset ?? 0),
    hasMore: Boolean(response.has_more),
  };
}

function decodeBase64(payloadBase64: string): Uint8Array {
  if (typeof atob === "function") {
    const binary = atob(payloadBase64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
  }

  const nodeBuffer = (globalThis as { Buffer?: { from: (input: string, encoding: string) => Uint8Array } }).Buffer;
  if (nodeBuffer) {
    return nodeBuffer.from(payloadBase64, "base64");
  }
  throw new Error("No base64 decoder available");
}

function decodeAsUtf8(payloadBase64: string): string {
  const bytes = decodeBase64(payloadBase64);
  return new TextDecoder().decode(bytes);
}

export function decodeTopicPayload(payloadBase64: string, mode: PayloadDecodeMode): DecodedPayload {
  if (mode === "base64") {
    return {
      mode,
      text: null,
      prettyJson: null,
      base64: payloadBase64,
      error: null,
    };
  }

  try {
    const text = decodeAsUtf8(payloadBase64);
    if (mode === "text") {
      return {
        mode,
        text,
        prettyJson: null,
        base64: payloadBase64,
        error: null,
      };
    }

    const parsed = parseJsonString(text);
    if (parsed !== null) {
      return {
        mode,
        text,
        prettyJson: JSON.stringify(parsed, null, 2),
        base64: payloadBase64,
        error: null,
      };
    }

    return {
      mode,
      text,
      prettyJson: null,
      base64: payloadBase64,
      error: null,
    };
  } catch (error) {
    const message = error instanceof Error ? error.message : "Failed to decode payload";
    return {
      mode,
      text: null,
      prettyJson: null,
      base64: payloadBase64,
      error: message,
    };
  }
}

