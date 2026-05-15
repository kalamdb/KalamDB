import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Messages } from "../../src/components/Messages";

function makeMessage(
  overrides: Partial<{
    id: string;
    conversationId: string;
    role: "user" | "assistant" | "system";
    body: string;
    status: "pending" | "streaming" | "final" | "cancelled" | "error";
    createdAt: Date;
    updatedAt: Date;
  }> = {},
) {
  return {
    id: overrides.id ?? "msg-1",
    conversationId: overrides.conversationId ?? "conv-1",
    role: overrides.role ?? "user",
    body: overrides.body ?? "hello",
    status: overrides.status ?? "final",
    createdAt: overrides.createdAt ?? new Date("2026-05-15T10:00:00Z"),
    updatedAt: overrides.updatedAt ?? new Date("2026-05-15T10:00:00Z"),
  };
}

describe("Messages", () => {
  it("renders the empty-state when no messages are present", () => {
    render(<Messages messages={[]} typingTokens={[]} approvals={[]} onApproval={vi.fn()} />);
    expect(screen.getByText(/Send a message to start the conversation/i)).toBeInTheDocument();
  });

  it("renders user + assistant bubbles with their bodies", () => {
    const messages = [
      makeMessage({ id: "u1", role: "user", body: "what is the capital of france?" }),
      makeMessage({ id: "a1", role: "assistant", body: "Paris.", status: "final" }),
    ];
    render(<Messages messages={messages} typingTokens={[]} approvals={[]} onApproval={vi.fn()} />);
    expect(screen.getByText("what is the capital of france?")).toBeInTheDocument();
    expect(screen.getByText("Paris.")).toBeInTheDocument();
  });

  it("prefers concatenated typing tokens over m.body while status='streaming'", () => {
    const m = makeMessage({ id: "a1", role: "assistant", body: "old body", status: "streaming" });
    const tokens = [
      {
        id: "t1",
        conversationId: "conv-1",
        messageId: "a1",
        body: "Hello ",
        seq: 1,
        createdAt: new Date(),
      },
      {
        id: "t2",
        conversationId: "conv-1",
        messageId: "a1",
        body: "world",
        seq: 2,
        createdAt: new Date(),
      },
    ];
    render(<Messages messages={[m]} typingTokens={tokens} approvals={[]} onApproval={vi.fn()} />);
    expect(screen.getByText("Hello world")).toBeInTheDocument();
    expect(screen.queryByText("old body")).toBeNull();
  });

  it("shows '(stopped)' marker for cancelled assistant messages", () => {
    const m = makeMessage({ id: "a1", role: "assistant", body: "partial", status: "cancelled" });
    render(<Messages messages={[m]} typingTokens={[]} approvals={[]} onApproval={vi.fn()} />);
    expect(screen.getByText("(stopped)")).toBeInTheDocument();
  });

  it("renders approval card buttons and fires onApproval", async () => {
    const user = userEvent.setup();
    const m = makeMessage({ id: "a1", role: "assistant", status: "final", body: "" });
    const approvals = [
      {
        id: "ap-1",
        conversationId: "conv-1",
        messageId: "a1",
        question: "Approve deletion?",
        status: "pending",
        createdAt: new Date(),
        resolvedAt: null,
      },
    ];
    const onApproval = vi.fn().mockResolvedValue(undefined);
    render(
      <Messages messages={[m]} typingTokens={[]} approvals={approvals} onApproval={onApproval} />,
    );
    expect(screen.getByText("Approve deletion?")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Approve/ }));
    expect(onApproval).toHaveBeenCalledWith("ap-1", "approved");
  });

  it("renders resolved approvals with status badge instead of buttons", () => {
    const m = makeMessage({ id: "a1", role: "assistant", status: "final" });
    const approvals = [
      {
        id: "ap-1",
        conversationId: "conv-1",
        messageId: "a1",
        question: "Approve?",
        status: "approved",
        createdAt: new Date(),
        resolvedAt: new Date(),
      },
    ];
    render(
      <Messages messages={[m]} typingTokens={[]} approvals={approvals} onApproval={vi.fn()} />,
    );
    expect(screen.queryByRole("button", { name: /Approve/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Reject/ })).toBeNull();
    expect(screen.getByText(/Approved/)).toBeInTheDocument();
  });
});
