import type { LlmAdapter, LlmStreamArgs, LlmStreamEvent } from "./index.js";

/**
 * Canned-response adapter for first-run demos without an API key.
 *
 * Trigger words:
 *  - "delete | remove | drop | send | charge | email | uninstall | wipe":
 *    emits a request_approval tool call so the approval flow can be seen.
 *  - "__slow_stream__":  emits a long, deliberately slow stream so the
 *    Stop-mid-stream path can be tested deterministically.
 *
 * Otherwise it streams a short canned reply.
 */
export class MockAdapter implements LlmAdapter {
  public readonly name = "mock";

  async *stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
    const lastUser = [...args.messages].reverse().find((m) => m.role === "user");
    const text = lastUser && lastUser.role === "user" ? lastUser.content.toLowerCase() : "";

    const lastTool = [...args.messages].reverse().find((m) => m.role === "tool");
    const isResumeAfterApproval = !!lastTool;

    if (!isResumeAfterApproval && /__slow_stream__/.test(text)) {
      // Long, deliberately slow stream — gives the Stop test a wide window.
      // 200 chunks × 100ms = 20s, far longer than any real interactive reply.
      yield { type: "text", delta: "Sure, here's a long, slow response so you can interrupt me. " };
      for (let i = 0; i < 200; i++) {
        if (args.signal.aborted) return;
        await sleep(100);
        yield { type: "text", delta: `chunk_${i.toString().padStart(3, "0")} ` };
      }
      yield { type: "done", reason: "stop" };
      return;
    }

    if (
      !isResumeAfterApproval &&
      /\b(delete|remove|drop|send|charge|email|uninstall|wipe)\b/.test(text)
    ) {
      // Interleave a short text preface before the tool_call so the agent's
      // assembled-text-flush path is exercised the same way it would be with
      // a real model.
      yield { type: "text", delta: "Let me get explicit approval before I do that. " };
      await sleep(40);
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
      reply = "Approved — done. (Mock adapter; nothing actually happened.)";
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
    "(Mock reply) I see your message. Wire up OPENAI_API_KEY or ANTHROPIC_API_KEY in your .env to get real responses. " +
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
