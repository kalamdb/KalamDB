import type {
  ConsumeStartMode,
  StreamingGroupId,
  StreamingPartitionId,
  StreamingTopicId,
} from "@/features/streaming/types";

export interface StreamingOffsetsFilter {
  topicId?: StreamingTopicId;
  groupId?: StreamingGroupId;
  limit?: number;
}

function escapeSqlLiteral(value: string): string {
  return value.replace(/'/g, "''");
}

export function buildTopicSqlSnippet(topicId: StreamingTopicId): string {
  const escapedTopicId = escapeSqlLiteral(topicId);
  return [
    `SELECT * FROM system.topics WHERE topic_id = '${escapedTopicId}';`,
    `SELECT topic_id, group_id, partition_id, last_acked_offset, updated_at`,
    `FROM system.topic_offsets WHERE topic_id = '${escapedTopicId}'`,
    `ORDER BY group_id, partition_id;`,
  ].join("\n");
}

export function buildGroupSqlSnippet(groupId: StreamingGroupId): string {
  const escapedGroupId = escapeSqlLiteral(groupId);
  return [
    `SELECT topic_id, group_id, partition_id, last_acked_offset, updated_at`,
    `FROM system.topic_offsets WHERE group_id = '${escapedGroupId}'`,
    `ORDER BY topic_id, partition_id;`,
  ].join("\n");
}

export function buildConsumeSqlSnippet(
  topicId: StreamingTopicId,
  groupId: StreamingGroupId | null,
  startMode: ConsumeStartMode,
  offset?: number,
  limit: number = 100,
): string {
  const safeLimit = Math.min(Math.max(limit, 1), 1000);
  let startClause = "FROM LATEST";
  if (startMode === "Earliest") {
    startClause = "FROM EARLIEST";
  }
  if (startMode === "Offset") {
    startClause = `FROM ${Math.max(0, offset ?? 0)}`;
  }
  if (!groupId) {
    return `CONSUME FROM ${topicId} ${startClause} LIMIT ${safeLimit};`;
  }

  const escapedGroupId = escapeSqlLiteral(groupId);
  return `CONSUME FROM ${topicId} GROUP '${escapedGroupId}' ${startClause} LIMIT ${safeLimit};`;
}

export function buildResetConsumerGroupSqlSnippet(
  topicId: StreamingTopicId,
  groupId: StreamingGroupId,
  partitionId: StreamingPartitionId = 0,
  nextOffset: number = 0,
): string {
  const escapedGroupId = escapeSqlLiteral(groupId);
  const safePartitionId = Math.max(0, Number(partitionId) || 0);
  const safeNextOffset = Math.max(0, nextOffset || 0);
  return `RESET CONSUMER GROUP '${escapedGroupId}' ON ${topicId} PARTITION ${safePartitionId} TO ${safeNextOffset};`;
}
