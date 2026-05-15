import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Conversation } from "../../src/components/Conversation";
import { conversations, messages, tasks } from "../../src/schema";

// Regression: the agent now owns the assistant-message stub. The frontend
// must only write the user message + the task row — never the assistant
// stub itself.

function makeMocks() {
  type InsertCall = { table: unknown; row: Record<string, unknown> };
  const inserts: InsertCall[] = [];
  type UpdateCall = { table: unknown; id: string; patch: Record<string, unknown> };
  const updates: UpdateCall[] = [];

  const insert = vi.fn((table: unknown) => ({
    values: vi.fn(async (row: Record<string, unknown>) => {
      inserts.push({ table, row });
    }),
  }));
  const update = vi.fn((table: unknown, id: string) => ({
    set: vi.fn(async (patch: Record<string, unknown>) => {
      updates.push({ table, id, patch });
    }),
  }));

  return { insert, update, inserts, updates };
}

describe("Conversation.sendUserMessage", () => {
  it("writes a user message + task, NEVER an assistant stub (agent owns it)", async () => {
    const user = userEvent.setup();
    const { insert, update, inserts, updates } = makeMocks();

    render(
      <Conversation
        conversationId="conv-1"
        conversation={{
          id: "conv-1",
          title: "New conversation",
          createdAt: new Date(),
          updatedAt: new Date(),
        }}
        messages={[]}
        typingTokens={[]}
        approvals={[]}
        activeTask={null}
        isAgentBusy={false}
        insert={insert as never}
        update={update as never}
      />,
    );

    const ta = screen.getByPlaceholderText(/Message KalamDB Chat/i);
    await user.type(ta, "what is the capital of france?");
    await user.click(screen.getByRole("button", { name: "Send" }));

    // Exactly two inserts, in this order.
    expect(inserts).toHaveLength(2);
    expect(inserts[0]!.table).toBe(messages);
    expect(inserts[0]!.row.role).toBe("user");
    expect(inserts[0]!.row.body).toBe("what is the capital of france?");
    expect(inserts[0]!.row.status).toBe("final");

    expect(inserts[1]!.table).toBe(tasks);
    expect(inserts[1]!.row.isCancelled).toBe(false);
    expect(inserts[1]!.row.finishedAt).toBeNull();

    // No assistant-role message insert from the frontend — the agent owns it.
    expect(inserts.filter((c) => c.row.role === "assistant")).toHaveLength(0);

    // The conversation's updatedAt + (because this is the first msg) title is set.
    expect(updates).toHaveLength(1);
    expect(updates[0]!.table).toBe(conversations);
    expect(updates[0]!.patch.title).toBe("what is the capital of france?".slice(0, 60));
  });

  it("does NOT overwrite the title on subsequent user messages", async () => {
    const user = userEvent.setup();
    const { insert, update, updates } = makeMocks();

    const existing = {
      id: "msg-prior",
      conversationId: "conv-1",
      role: "user" as const,
      body: "earlier",
      status: "final" as const,
      createdAt: new Date(),
      updatedAt: new Date(),
    };

    render(
      <Conversation
        conversationId="conv-1"
        conversation={{
          id: "conv-1",
          title: "Custom title",
          createdAt: new Date(),
          updatedAt: new Date(),
        }}
        messages={[existing]}
        typingTokens={[]}
        approvals={[]}
        activeTask={null}
        isAgentBusy={false}
        insert={insert as never}
        update={update as never}
      />,
    );

    await user.type(screen.getByPlaceholderText(/Message KalamDB Chat/i), "follow-up");
    await user.click(screen.getByRole("button", { name: "Send" }));

    expect(updates).toHaveLength(1);
    expect(updates[0]!.patch.title).toBeUndefined();
    expect(updates[0]!.patch.updatedAt).toBeInstanceOf(Date);
  });

  it("Stop button issues an UPDATE on the task setting isCancelled=true", async () => {
    const user = userEvent.setup();
    const { insert, update, updates } = makeMocks();
    const active = {
      id: "task-1",
      conversationId: "conv-1",
      messageId: "asst-pending",
      isCancelled: false,
      startedAt: new Date(),
      finishedAt: null,
    };
    render(
      <Conversation
        conversationId="conv-1"
        conversation={null}
        messages={[]}
        typingTokens={[]}
        approvals={[]}
        activeTask={active}
        isAgentBusy={true}
        insert={insert as never}
        update={update as never}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(updates).toHaveLength(1);
    expect(updates[0]!.table).toBe(tasks);
    expect(updates[0]!.id).toBe("task-1");
    expect(updates[0]!.patch.isCancelled).toBe(true);
  });
});
