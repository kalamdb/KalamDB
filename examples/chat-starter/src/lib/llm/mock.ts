import type {
  LlmAdapter,
  LlmStreamArgs,
  LlmStreamEvent,
} from "./index.js";

/**
 * Canned-response adapter for first-run demos without an API key.
 *
 * If the latest user message contains words like "delete", "send", "charge",
 * "remove", "drop", "uninstall"  the agent emits a `request_approval` tool
 * call. This lets you see the approval flow without paying OpenAI.
 *
 * Otherwise it streams a short fixed reply token-by-token.
 */
export class MockAdapter implements LlmAdapter {
  public readonly name = "mock";

  async *stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
    const lastUser = [...args.messages].reverse().find((m) => m.role === "user");
    const text = lastUser && lastUser.role === "user" ? lastUser.content.toLowerCase() : "";

    const lastTool = [...args.messages].reverse().find((m) => m.role === "tool");
    const isResumeAfterApproval = !!lastTool;

    if (!isResumeAfterApproval && /\b(delete|remove|drop|send|charge|email|uninstall|wipe)\b/.test(text)) {
      yield {
        type: "tool_call",
        call: {
          id: `mock_${Date.now()}`,
          name: "request_approval",
          arguments: {
            question:
              "I'm about to perform a potentially destructive action based on your request. Approve?",
          },
        },
      };
      yield { type: "done", reason: "tool_calls" };
      return;
    }

    const decision =
      lastTool && lastTool.role === "tool" ? lastTool.content.toLowerCase().trim() : null;

    let reply: string;
    if (decision === "approved") {
      reply = "Approved  done. (Mock adapter; nothing actually happened.)";
    } else if (decision === "rejected") {
      reply = "Okay, cancelled. No action taken.";
    } else if (decision && decision.startsWith("rejected")) {
      reply = "Cancelled before approval was resolved. No action taken.";
    } else {
      reply = pickReply(text);
    }

    for (const chunk of chunkString(reply, 6)) {
      if (args.signal.aborted) return;
      await sleep(40);
      yield { type: "text", delta: chunk };
    }
    yield { type: "done", reason: "stop" };
  }
}

function pickReply(userText: string): string {
  if (!userText.trim()) {
    return "Hi! I'm the mock assistant. Try asking me to delete something to see the approval flow.";
  }
  if (/\b(hi|hello|hey|yo)\b/.test(userText)) {
    return "Hey! I'm the mock assistant. Ask me anything, or try a phrase like 'delete my account' to see the approval flow in action.";
  }
  return (
    "(Mock reply) I see your message. Wire up `OPENAI_API_KEY` in your .env to get real responses. " +
    "Try saying something like 'delete my old files' to see the approval flow."
  );
}

function chunkString(s: string, size: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < s.length; i += size) {
    out.push(s.slice(i, i + size));
  }
  return out;
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
