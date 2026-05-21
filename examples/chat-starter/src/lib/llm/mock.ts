import type { LlmAdapter, LlmStreamArgs, LlmStreamEvent } from "./index.js";

/**
 * Canned-response adapter for first-run demos without an API key.
 *
 * The mock pattern-matches the latest user message to decide which tool flow
 * (if any) to exercise. This keeps unit / e2e tests deterministic without a
 * live model. Keywords:
 *
 *   "__slow_stream__"      — long, deliberately slow stream so the cancel
 *                            path can be tested.
 *   "delete this conversation" / "delete my account" / etc.
 *                          — request_approval → (on approved) → delete_conversation
 *   "how many" / "count" / "list" / "show me" / "what is in"
 *                          — query_database with a sensible canned SELECT.
 *
 * Anything else → a short canned reply.
 */
export class MockAdapter implements LlmAdapter {
  public readonly name = "mock";

  async *stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
    const lastUser = [...args.messages].reverse().find((m) => m.role === "user");
    const text = lastUser && lastUser.role === "user" ? lastUser.content.toLowerCase() : "";

    const lastTool = [...args.messages].reverse().find((m) => m.role === "tool");
    const lastAssistant = [...args.messages].reverse().find((m) => m.role === "assistant");
    // Lowercased view for keyword matching; raw view preserves casing for
    // JSON field extraction (e.g., the title we cite in RAG replies).
    const decision = lastTool && lastTool.role === "tool" ? lastTool.content.toLowerCase() : null;
    const decisionRaw = lastTool && lastTool.role === "tool" ? lastTool.content : null;

    // The agent injects `conversation_id = <uuid>` in a system message so any
    // tool that needs it (delete_conversation, query_database with a WHERE
    // clause) can pull it out without the LLM having to guess.
    const conversationId = (() => {
      for (const m of args.messages) {
        if (m.role !== "system") continue;
        const match = m.content.match(/conversation_id\s*=\s*([0-9a-f-]{36})/i);
        if (match) return match[1]!;
      }
      return "00000000-0000-0000-0000-000000000000";
    })();

    // ---- slow stream (cancel test) -----------------------------------------
    if (!decision && /__slow_stream__/.test(text)) {
      yield {
        type: "text",
        delta: "Sure, here's a long, slow response so you can interrupt me. ",
      };
      for (let i = 0; i < 200; i++) {
        if (args.signal.aborted) return;
        await sleep(100);
        yield { type: "text", delta: `chunk_${i.toString().padStart(3, "0")} ` };
      }
      yield { type: "done", reason: "stop" };
      return;
    }

    // ---- bulk delete flow: turn 1 = approval -------------------------------
    // Detect bulk intent BEFORE the single-delete branch so "delete all my
    // conversations" doesn't get routed through delete_conversation.
    const isBulkDelete =
      /\b(delete all|wipe everything|wipe all|clear (my )?history|clear all)\b/i.test(text);
    if (!decision && isBulkDelete) {
      yield { type: "text", delta: "I can do that — getting your approval first. " };
      await sleep(20);
      yield {
        type: "tool_call",
        call: {
          id: `mock_approval_${Date.now()}`,
          name: "request_approval",
          arguments: {
            question: "Permanently delete ALL of your conversations?",
          },
        },
      };
      yield { type: "done", reason: "tool_calls" };
      return;
    }

    // ---- delete flow: turn 1 = approval ------------------------------------
    if (!decision && /\b(delete|remove|wipe|drop|uninstall)\b/.test(text)) {
      yield { type: "text", delta: "I can do that — getting your approval first. " };
      await sleep(20);
      yield {
        type: "tool_call",
        call: {
          id: `mock_approval_${Date.now()}`,
          name: "request_approval",
          arguments: {
            question: "Permanently delete this conversation and its messages?",
          },
        },
      };
      yield { type: "done", reason: "tool_calls" };
      return;
    }

    // ---- delete flow: turn 2 = after approval ------------------------------
    // Look at the *previous* turn — if the assistant just called request_approval
    // and it was approved, the model should now call the matching destructive
    // tool. Pick single vs bulk based on the approval question text.
    if (
      decision === "approved" &&
      lastAssistant &&
      lastAssistant.role === "assistant" &&
      lastAssistant.toolCalls?.some((c) => c.name === "request_approval")
    ) {
      const approvalCall = lastAssistant.toolCalls.find((c) => c.name === "request_approval");
      const question = String(approvalCall?.arguments?.question ?? "");
      const isBulk = /\ball of your conversations\b/i.test(question);
      yield {
        type: "tool_call",
        call: {
          id: `mock_delete_${Date.now()}`,
          name: isBulk ? "delete_all_conversations" : "delete_conversation",
          arguments: isBulk ? {} : { conversation_id: conversationId },
        },
      };
      yield { type: "done", reason: "tool_calls" };
      return;
    }

    // ---- delete flow: turn 3 = wrap up -------------------------------------
    if (
      decision &&
      (decision.startsWith("deleted conversation") || decision.startsWith("deleted "))
    ) {
      yield { type: "text", delta: "Deleted. Anything else?" };
      yield { type: "done", reason: "stop" };
      return;
    }

    // ---- search_documents flow: turn 1 = run the search --------------------
    // Fuzzy / conceptual questions about KalamDB itself go through RAG, not
    // query_database. We detect them BEFORE the structured-query branch
    // because "how does X work" and "what is X" overlap with "how many" only
    // weakly — but we want RAG to win on those phrasings.
    if (
      !decision &&
      /\b(what is|what's|how does|how do|explain|tell me about|describe)\b/.test(text) &&
      !/\b(how many|count of|number of)\b/.test(text)
    ) {
      yield {
        type: "tool_call",
        call: {
          id: `mock_search_${Date.now()}`,
          name: "search_documents",
          arguments: { query: lastUser?.role === "user" ? lastUser.content : text, limit: 3 },
        },
      };
      yield { type: "done", reason: "tool_calls" };
      return;
    }

    // ---- search_documents flow: turn 2 = phrase the answer with citation --
    if (
      decisionRaw &&
      decisionRaw.startsWith("{") &&
      lastAssistant?.toolCalls?.some((c) => c.name === "search_documents")
    ) {
      const titleMatch = decisionRaw.match(/"title":\s*"([^"]+)"/);
      const bodyMatch = decisionRaw.match(/"body":\s*"([^"]+)"/);
      const title = titleMatch ? titleMatch[1] : "the docs";
      const snippet = bodyMatch ? bodyMatch[1]!.slice(0, 120) : "see the documentation";
      const reply = `${snippet} (source: "${title}")`;
      for (const chunk of chunkString(reply, 6)) {
        if (args.signal.aborted) return;
        await sleep(40);
        yield { type: "text", delta: chunk };
      }
      yield { type: "done", reason: "stop" };
      return;
    }

    // ---- query_database flow: turn 1 = run the query -----------------------
    if (!decision && /\b(how many|count|list|show me|what is in|what's in|how much)\b/.test(text)) {
      yield { type: "text", delta: "Checking. " };
      await sleep(20);
      // Pick a plausible canned SELECT based on which table the user mentioned.
      let sql = "SELECT count(*) AS n FROM chat.conversations";
      if (/messages?/.test(text)) sql = "SELECT count(*) AS n FROM chat.messages";
      else if (/approvals?/.test(text)) sql = "SELECT count(*) AS n FROM chat.approvals";
      else if (/tasks?/.test(text)) sql = "SELECT count(*) AS n FROM chat.tasks";
      yield {
        type: "tool_call",
        call: {
          id: `mock_query_${Date.now()}`,
          name: "query_database",
          arguments: { sql },
        },
      };
      yield { type: "done", reason: "tool_calls" };
      return;
    }

    // ---- query_database flow: turn 2 = phrase the answer -------------------
    if (
      decision &&
      decision.startsWith("{") &&
      lastAssistant?.toolCalls?.some((c) => c.name === "query_database")
    ) {
      // Naive extraction of a count from the JSON result.
      const match = decision.match(/"n":\s*(\d+)/) || decision.match(/"row_count":\s*(\d+)/);
      const n = match ? match[1] : "some";
      const reply = `Looks like ${n}.`;
      for (const chunk of chunkString(reply, 6)) {
        if (args.signal.aborted) return;
        await sleep(40);
        yield { type: "text", delta: chunk };
      }
      yield { type: "done", reason: "stop" };
      return;
    }

    // ---- fallback: short canned reply --------------------------------------
    const reply = pickReply(text, decision);
    for (const chunk of chunkString(reply, 6)) {
      if (args.signal.aborted) return;
      await sleep(40);
      yield { type: "text", delta: chunk };
    }
    yield { type: "done", reason: "stop" };
  }
}

function pickReply(userText: string, decision: string | null): string {
  if (decision === "rejected" || (decision && decision.startsWith("rejected"))) {
    return "Okay, cancelled. No action taken.";
  }
  if (decision && decision.startsWith("Error:")) {
    return "Hmm, that didn't work. Want me to try a different approach?";
  }
  if (!userText.trim()) {
    return "Hi! I'm the mock assistant. Try asking me to delete something to see the approval flow.";
  }
  if (/\b(hi|hello|hey|yo)\b/.test(userText)) {
    return "Hey! I'm the mock assistant. Ask me anything, or try a phrase like 'how many conversations do I have' to see the query flow.";
  }
  return (
    "(Mock reply) I see your message. Wire up OPENAI_API_KEY or ANTHROPIC_API_KEY in your .env to get real responses. " +
    "Try 'how many messages do I have' or 'delete my old account' to see the tool flows."
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
