// @vitest-environment jsdom

import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import StreamingTopicDetail from "@/pages/StreamingTopicDetail";
import type { ConsumedMessageBatch, StreamingOffset, StreamingTopic } from "@/features/streaming/types";

const mockUseAuth = vi.fn();
const mockUseGetStreamingTopicsQuery = vi.fn();
const mockUseGetStreamingOffsetsQuery = vi.fn();
const mockUseConsumeStreamingMessagesMutation = vi.fn();
const mockRefetchTopics = vi.fn();
const mockRefetchOffsets = vi.fn();
const mockConsumeMessages = vi.fn();

vi.mock("@/lib/auth", () => ({
  useAuth: () => mockUseAuth(),
}));

vi.mock("@/store/apiSlice", () => ({
  useGetStreamingTopicsQuery: () => mockUseGetStreamingTopicsQuery(),
  useGetStreamingOffsetsQuery: (...args: unknown[]) => mockUseGetStreamingOffsetsQuery(...args),
  useConsumeStreamingMessagesMutation: () => mockUseConsumeStreamingMessagesMutation(),
}));

vi.mock("@kalamdb/client", () => ({
  KalamCellValue: class KalamCellValue {
    private value: unknown;

    constructor(value: unknown) {
      this.value = value;
    }

    toJson() {
      return this.value;
    }
  },
}));

const topic: StreamingTopic = {
  topicId: "blog.summarizer",
  name: "blog.summarizer",
  partitions: 2,
  retentionSeconds: null,
  retentionMaxBytes: null,
  routeCount: 1,
  routes: [],
  createdAt: "2026-05-20T10:00:00Z",
  updatedAt: "2026-05-20T10:05:00Z",
};

const offsets: StreamingOffset[] = [
  {
    topicId: "blog.summarizer",
    groupId: "summarizer-workers",
    partitionId: 0,
    lastAckedOffset: 41,
    nextOffset: 42,
    updatedAt: "2026-05-20T10:10:00Z",
  },
];

const consumedBatch: ConsumedMessageBatch = {
  messages: [
    {
      topicId: "blog.summarizer",
      partitionId: 0,
      offset: 0,
      payloadBase64: "eyJ0aXRsZSI6ImhlbGxvIn0=",
      key: "315556072339275776",
      timestampMs: 1_735_689_600_000,
      username: "admin",
      op: "Insert",
    },
  ],
  nextOffset: 1,
  hasMore: false,
};

function renderTopicDetail() {
  return render(
    <MemoryRouter initialEntries={["/streaming/topics/blog.summarizer"]}>
      <Routes>
        <Route path="/streaming/topics/:topicId" element={<StreamingTopicDetail />} />
        <Route path="/streaming/topics" element={<div>Topic list</div>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("StreamingTopicDetail", () => {
  beforeEach(() => {
    cleanup();
    vi.stubGlobal(
      "ResizeObserver",
      class ResizeObserver {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
    vi.stubGlobal("PointerEvent", MouseEvent);
    Object.defineProperty(window.HTMLElement.prototype, "scrollIntoView", {
      value: vi.fn(),
      configurable: true,
    });
    Object.defineProperty(window.HTMLElement.prototype, "hasPointerCapture", {
      value: vi.fn(() => false),
      configurable: true,
    });
    Object.defineProperty(window.HTMLElement.prototype, "releasePointerCapture", {
      value: vi.fn(),
      configurable: true,
    });

    mockUseAuth.mockReturnValue({ user: { username: "admin" } });
    mockUseGetStreamingTopicsQuery.mockReturnValue({
      data: [topic],
      isFetching: false,
      refetch: mockRefetchTopics,
    });
    mockUseGetStreamingOffsetsQuery.mockReturnValue({
      data: offsets,
      isFetching: false,
      refetch: mockRefetchOffsets,
    });
    mockConsumeMessages.mockReturnValue({ unwrap: () => Promise.resolve(consumedBatch) });
    mockUseConsumeStreamingMessagesMutation.mockReturnValue([
      mockConsumeMessages,
      { isLoading: false, error: null },
    ]);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it("uses topic breadcrumbs and local inspector tabs without the global streaming tabs", () => {
    renderTopicDetail();

    expect(screen.getByRole("heading", { name: "Streaming Topic" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Topics/i })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Topics" })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Consumers" })).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Inspect Messages" })).toHaveAttribute("data-state", "active");
    expect(screen.getByRole("tab", { name: "Committed Offsets" })).toBeInTheDocument();
  });

  it("defaults the inspector to the first partition and offset start mode", () => {
    renderTopicDetail();

    expect(screen.getByRole("combobox", { name: "Partition" })).toHaveTextContent("0");
    expect(screen.getByRole("combobox", { name: "Start" })).toHaveTextContent("Offset");
    expect(screen.getByRole("textbox", { name: "Offset" })).toBeEnabled();
  });

  it("keeps a changed partition selected and uses it for inspection", async () => {
    renderTopicDetail();

    const partitionSelect = screen.getByRole("combobox", { name: "Partition" });
    partitionSelect.focus();
    fireEvent.keyDown(partitionSelect, { key: "ArrowDown", code: "ArrowDown" });
    fireEvent.click(await screen.findByRole("option", { name: "1" }));

    await waitFor(() => {
      expect(partitionSelect).toHaveTextContent("1");
    });

    fireEvent.click(screen.getByRole("button", { name: "Inspect" }));

    await waitFor(() => {
      expect(mockConsumeMessages).toHaveBeenCalledWith(expect.objectContaining({ partitionId: 1 }));
    });
  });

  it("consumes from the selected defaults and renders millisecond timestamps", async () => {
    renderTopicDetail();

    fireEvent.click(screen.getByRole("button", { name: "Inspect" }));

    await waitFor(() => {
      expect(mockConsumeMessages).toHaveBeenCalledWith({
        topicId: "blog.summarizer",
        groupId: undefined,
        partitionId: 0,
        startMode: "Offset",
        offset: 0,
        limit: 100,
        timeoutSeconds: 5,
      });
    });

    await waitFor(() => {
      expect(screen.getAllByText("2025-01-01T00:00:00Z")).toHaveLength(2);
    });
    expect(document.body.textContent).toContain('"title": "hello"');
    expect(screen.queryByText(/1970-/)).not.toBeInTheDocument();
  });

  it("shows committed offsets in the second local tab", async () => {
    renderTopicDetail();

    const committedOffsetsTab = screen.getByRole("tab", { name: "Committed Offsets" });
    committedOffsetsTab.focus();
    fireEvent.keyDown(committedOffsetsTab, { key: "Enter", code: "Enter" });

    await waitFor(() => {
      expect(committedOffsetsTab).toHaveAttribute("data-state", "active");
      expect(screen.getByText("summarizer-workers")).toBeInTheDocument();
    });
    expect(screen.getByText("41")).toBeInTheDocument();
    expect(screen.getByText("42")).toBeInTheDocument();
  });
});