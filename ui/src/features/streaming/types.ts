import type { SystemTopicOffsetRow, SystemTopicRow } from "@/lib/models";

export type StreamingTopicRow = SystemTopicRow;
export type StreamingTopicOffsetRow = SystemTopicOffsetRow;
export type StreamingTopicId = StreamingTopicRow["topic_id"];
export type StreamingGroupId = StreamingTopicOffsetRow["group_id"];
export type StreamingPartitionId = StreamingTopicOffsetRow["partition_id"];

export interface StreamingTopicRoute {
  tableId: string;
  op: string;
  payloadMode: string;
  filterExpr: string | null;
  partitionKeyExpr: string | null;
}

export interface StreamingTopic {
  topicId: StreamingTopicId;
  name: StreamingTopicRow["name"];
  partitions: StreamingTopicRow["partitions"];
  retentionSeconds: number | null;
  retentionMaxBytes: number | null;
  routeCount: number;
  routes: StreamingTopicRoute[];
  createdAt: StreamingTopicRow["created_at"] | null;
  updatedAt: StreamingTopicRow["updated_at"] | null;
}

export interface StreamingConsumerGroup {
  groupId: StreamingGroupId;
  topicCount: number;
  partitionCount: number;
  lastUpdatedAt: StreamingTopicOffsetRow["updated_at"] | null;
}

export interface StreamingOffset {
  topicId: StreamingTopicOffsetRow["topic_id"];
  groupId: StreamingTopicOffsetRow["group_id"];
  partitionId: StreamingTopicOffsetRow["partition_id"];
  lastAckedOffset: number;
  nextOffset: number;
  updatedAt: StreamingTopicOffsetRow["updated_at"] | null;
}

export interface StreamingConsumerOffsetSummary {
  topicId: StreamingTopicOffsetRow["topic_id"];
  topicName: StreamingTopicRow["name"];
  groupId: StreamingTopicOffsetRow["group_id"];
  claimedPartitions: number;
  configuredPartitions: StreamingTopicRow["partitions"];
  lastAckedOffset: number;
  committedOffset: number;
  updatedAt: StreamingTopicOffsetRow["updated_at"] | null;
}

export interface StreamingTopicPartitionCursor {
  topicId: StreamingTopicOffsetRow["topic_id"];
  partitionId: StreamingTopicOffsetRow["partition_id"];
  nextOffset: number;
  lastOffset: number | null;
}

export type ConsumeStartMode = "Latest" | "Earliest" | "Offset";
export type ConsumeReadMode = "Inspect" | "Group";

export interface ConsumeMessagesInput {
  topicId: StreamingTopicId;
  groupId?: StreamingGroupId;
  partitionId: StreamingPartitionId;
  startMode: ConsumeStartMode;
  offset?: number;
  limit: number;
  timeoutSeconds?: number;
}

export interface ConsumeApiMessage {
  topic_id: StreamingTopicId;
  partition_id: StreamingPartitionId;
  offset: number;
  payload: string;
  key: string | null;
  timestamp_ms: number;
  username?: string | null;
  op: string;
}

export interface ConsumeApiResponse {
  messages: ConsumeApiMessage[];
  next_offset: number;
  has_more: boolean;
}

export interface StreamingMessage {
  topicId: StreamingTopicId;
  partitionId: StreamingPartitionId;
  offset: number;
  payloadBase64: string;
  key: string | null;
  timestampMs: number;
  username: string | null;
  op: string;
}

export interface ConsumedMessageBatch {
  messages: StreamingMessage[];
  nextOffset: number;
  hasMore: boolean;
}

export type PayloadDecodeMode = "auto-json" | "text" | "base64";

export interface DecodedPayload {
  mode: PayloadDecodeMode;
  text: string | null;
  prettyJson: string | null;
  base64: string;
  error: string | null;
}

