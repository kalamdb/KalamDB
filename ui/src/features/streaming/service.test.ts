import { describe, expect, it, vi } from "vitest";

vi.mock("@/lib/schema", () => ({
  system_topic_offsets: {},
  system_topics: {},
}));

vi.mock("@kalamdb/orm", () => ({
  kTable: () => ({}),
}));

import {
  buildConsumeRequestBody,
  decodeTopicPayload,
  summarizeStreamingConsumerOffsets,
  fetchStreamingTopicPartitionCursors,
  mapOffsetRows,
  mapTopicRows,
  parseTopicRoutes,
} from "@/features/streaming/service";
import { api } from "@/lib/api";

vi.mock("@/lib/api", () => ({
  api: {
    post: vi.fn(),
  },
}));

describe("streaming service helpers", () => {
  it("parses topic routes from JSON payload", () => {
    const routes = parseTopicRoutes([
      {
        table_id: { namespace_id: "default", table_name: "messages" },
        op: "Insert",
        payload_mode: "Key",
        filter_expr: null,
        partition_key_expr: null,
      },
    ]);

    expect(routes).toHaveLength(1);
    expect(routes[0].tableId).toBe("default.messages");
    expect(routes[0].op).toBe("Insert");
  });

  it("maps topic rows with route counts", () => {
    const topics = mapTopicRows([
      {
        topic_id: "default.events",
        name: "default.events",
        alias: null,
        partitions: 3,
        retention_seconds: null,
        retention_max_bytes: null,
        routes: '[{"table_id":{"namespace_id":"default","table_name":"events"},"op":"Insert","payload_mode":"Key"}]',
        created_at: "2026-02-17T10:00:00Z",
        updated_at: "2026-02-17T10:10:00Z",
      },
    ]);

    expect(topics).toHaveLength(1);
    expect(topics[0].partitions).toBe(3);
    expect(topics[0].routeCount).toBe(1);
    expect(topics[0].routes[0].tableId).toBe("default.events");
  });

  it("maps offsets and calculates next offsets", () => {
    const offsets = mapOffsetRows([
      {
        topic_id: "default.events",
        group_id: "worker-a",
        partition_id: 0,
        last_acked_offset: "41",
        updated_at: "2026-02-17T12:00:00Z",
      },
    ]);

    expect(offsets).toHaveLength(1);
    expect(offsets[0].lastAckedOffset).toBe(41);
    expect(offsets[0].nextOffset).toBe(42);
  });

  it("summarizes offsets by group and topic", () => {
    const topics = mapTopicRows([
      {
        topic_id: "default.events",
        name: "Events",
        alias: null,
        partitions: 3,
        retention_seconds: null,
        retention_max_bytes: null,
        routes: "[]",
        created_at: "2026-02-17T10:00:00Z",
        updated_at: "2026-02-17T10:10:00Z",
      },
    ]);
    const offsets = mapOffsetRows([
      {
        topic_id: "default.events",
        group_id: "worker-a",
        partition_id: 0,
        last_acked_offset: "41",
        updated_at: "2026-02-17T12:00:00Z",
      },
      {
        topic_id: "default.events",
        group_id: "worker-a",
        partition_id: 1,
        last_acked_offset: "99",
        updated_at: "2026-02-17T12:05:00Z",
      },
    ]);

    const summaries = summarizeStreamingConsumerOffsets(topics, offsets);

    expect(summaries).toHaveLength(1);
    expect(summaries[0]).toMatchObject({
      topicId: "default.events",
      topicName: "Events",
      groupId: "worker-a",
      claimedPartitions: 2,
      configuredPartitions: 3,
      lastAckedOffset: 140,
      committedOffset: 142,
      updatedAt: "2026-02-17T12:05:00Z",
    });
  });

  it("fetches batched topic partition cursors", async () => {
    vi.mocked(api.post).mockResolvedValueOnce({
      offsets: [
        {
          topic_id: "default.events",
          partition_id: 0,
          next_offset: 120,
          last_offset: 119,
        },
      ],
    });

    const cursors = await fetchStreamingTopicPartitionCursors([
      { topicId: "default.events", partitionId: 0 },
      { topicId: "default.events", partitionId: 0 },
    ]);

    expect(api.post).toHaveBeenCalledWith("/topics/latest-offsets", {
      partitions: [{ topic_id: "default.events", partition_id: 0 }],
    });
    expect(cursors).toEqual([
      {
        topicId: "default.events",
        partitionId: 0,
        nextOffset: 120,
        lastOffset: 119,
      },
    ]);
  });

  it("builds consume request body with explicit offset", () => {
    const body = buildConsumeRequestBody({
      topicId: "default.events",
      groupId: "debug-ui",
      partitionId: 0,
      startMode: "Offset",
      offset: 99,
      limit: 25,
      timeoutSeconds: 5,
    });

    expect(body).toEqual({
      topic_id: "default.events",
      group_id: "debug-ui",
      partition_id: 0,
      limit: 25,
      timeout_seconds: 5,
      start: { Offset: 99 },
    });
  });

  it("decodes JSON payload in auto-json mode", () => {
    const payloadBase64 = "eyJpZCI6MSwidGV4dCI6ImhlbGxvIn0=";
    const decoded = decodeTopicPayload(payloadBase64, "auto-json");

    expect(decoded.error).toBeNull();
    expect(decoded.prettyJson).toContain('"id": 1');
    expect(decoded.text).toContain('"text":"hello"');
  });

  it("returns decode error for invalid base64", () => {
    const decoded = decodeTopicPayload("@@not-base64@@", "text");

    expect(decoded.error).not.toBeNull();
    expect(decoded.text).toBeNull();
  });
});

