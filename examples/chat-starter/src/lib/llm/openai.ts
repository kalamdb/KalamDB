import type {
  LlmAdapter,
  LlmMessage,
  LlmStreamArgs,
  LlmStreamEvent,
  LlmTool,
} from "./index.js";

interface OpenAiOptions {
  apiKey: string;
  model: string;
  baseUrl?: string;
}

interface OpenAiMessage {
  role: "system" | "user" | "assistant" | "tool";
  content: string | null;
  tool_calls?: Array<{
    id: string;
    type: "function";
    function: { name: string; arguments: string };
  }>;
  tool_call_id?: string;
}

export class OpenAiAdapter implements LlmAdapter {
  public readonly name: string;
  private readonly apiKey: string;
  private readonly model: string;
  private readonly baseUrl: string;

  constructor(opts: OpenAiOptions) {
    this.apiKey = opts.apiKey;
    this.model = opts.model;
    this.baseUrl = opts.baseUrl ?? "https://api.openai.com/v1";
    this.name = `openai:${opts.model}`;
  }

  async *stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
    const body: Record<string, unknown> = {
      model: this.model,
      messages: args.messages.map(toOpenAiMessage),
      stream: true,
    };
    if (args.tools && args.tools.length > 0) {
      body.tools = args.tools.map(toOpenAiTool);
      body.tool_choice = "auto";
    }

    const res = await fetch(`${this.baseUrl}/chat/completions`, {
      method: "POST",
      signal: args.signal,
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${this.apiKey}`,
      },
      body: JSON.stringify(body),
    });
    if (!res.ok || !res.body) {
      throw new Error(
        `OpenAI request failed (${res.status}): ${await res.text().catch(() => "")}`,
      );
    }

    // Accumulators for tool calls (OpenAI streams the arguments JSON in deltas).
    const pendingTools = new Map<
      number,
      { id: string; name: string; argsText: string }
    >();

    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let finishReason: "stop" | "tool_calls" | "length" | "error" = "stop";

    while (true) {
      if (args.signal.aborted) {
        reader.cancel().catch(() => undefined);
        return;
      }
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed.startsWith("data: ")) continue;
        const payload = trimmed.slice(6);
        if (payload === "[DONE]") {
          break;
        }
        try {
          const json = JSON.parse(payload) as {
            choices?: Array<{
              delta?: {
                content?: string;
                tool_calls?: Array<{
                  index: number;
                  id?: string;
                  function?: { name?: string; arguments?: string };
                }>;
              };
              finish_reason?: "stop" | "tool_calls" | "length" | null;
            }>;
          };
          const choice = json.choices?.[0];
          const delta = choice?.delta;
          if (delta?.content) {
            yield { type: "text", delta: delta.content };
          }
          if (delta?.tool_calls) {
            for (const tc of delta.tool_calls) {
              const existing = pendingTools.get(tc.index) ?? {
                id: tc.id ?? "",
                name: "",
                argsText: "",
              };
              if (tc.id) existing.id = tc.id;
              if (tc.function?.name) existing.name = tc.function.name;
              if (tc.function?.arguments) existing.argsText += tc.function.arguments;
              pendingTools.set(tc.index, existing);
            }
          }
          if (choice?.finish_reason) {
            finishReason = choice.finish_reason;
          }
        } catch {
          // Ignore malformed frames.
        }
      }
    }

    for (const pending of pendingTools.values()) {
      if (!pending.id || !pending.name) continue;
      let parsed: Record<string, unknown> = {};
      try {
        parsed = JSON.parse(pending.argsText || "{}") as Record<string, unknown>;
      } catch {
        parsed = { _raw: pending.argsText };
      }
      yield {
        type: "tool_call",
        call: { id: pending.id, name: pending.name, arguments: parsed },
      };
    }

    yield { type: "done", reason: finishReason };
  }
}

function toOpenAiMessage(msg: LlmMessage): OpenAiMessage {
  switch (msg.role) {
    case "system":
    case "user":
      return { role: msg.role, content: msg.content };
    case "assistant":
      return {
        role: "assistant",
        content: msg.content || null,
        tool_calls: msg.toolCalls?.map((tc) => ({
          id: tc.id,
          type: "function",
          function: { name: tc.name, arguments: JSON.stringify(tc.arguments) },
        })),
      };
    case "tool":
      return {
        role: "tool",
        content: msg.content,
        tool_call_id: msg.toolCallId,
      };
  }
}

function toOpenAiTool(tool: LlmTool) {
  return {
    type: "function" as const,
    function: {
      name: tool.name,
      description: tool.description,
      parameters: tool.parameters,
    },
  };
}
