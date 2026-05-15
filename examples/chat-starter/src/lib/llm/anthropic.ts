import type { LlmAdapter, LlmMessage, LlmStreamArgs, LlmStreamEvent, LlmTool } from "./index.js";

interface AnthropicOptions {
  apiKey: string;
  model: string;
  maxTokens?: number;
  baseUrl?: string;
}

interface AnthropicMessage {
  role: "user" | "assistant";
  content:
    | string
    | Array<
        | { type: "text"; text: string }
        | { type: "tool_use"; id: string; name: string; input: Record<string, unknown> }
        | { type: "tool_result"; tool_use_id: string; content: string }
      >;
}

const MAX_TOOL_ARGS_BYTES = 64 * 1024;

export class AnthropicAdapter implements LlmAdapter {
  public readonly name: string;
  private readonly apiKey: string;
  private readonly model: string;
  private readonly maxTokens: number;
  private readonly baseUrl: string;

  constructor(opts: AnthropicOptions) {
    this.apiKey = opts.apiKey;
    this.model = opts.model;
    this.maxTokens = opts.maxTokens ?? 2048;
    this.baseUrl = opts.baseUrl ?? "https://api.anthropic.com/v1";
    this.name = `anthropic:${opts.model}`;
  }

  async *stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
    const { system, messages } = splitSystemAndMessages(args.messages);
    const body: Record<string, unknown> = {
      model: this.model,
      max_tokens: this.maxTokens,
      messages,
      stream: true,
    };
    if (system) body.system = system;
    if (args.tools && args.tools.length > 0) {
      body.tools = args.tools.map(toAnthropicTool);
    }

    const res = await fetch(`${this.baseUrl}/messages`, {
      method: "POST",
      signal: args.signal,
      headers: {
        "content-type": "application/json",
        "x-api-key": this.apiKey,
        "anthropic-version": "2023-06-01",
      },
      body: JSON.stringify(body),
    });
    if (!res.ok || !res.body) {
      throw new Error(
        `Anthropic request failed (${res.status}): ${await res.text().catch(() => "")}`,
      );
    }

    // Per-content-block accumulator. Anthropic streams tool args as
    // input_json_delta events on a specific block index.
    const toolBlocks = new Map<number, { id: string; name: string; argsText: string }>();
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let finishReason: "stop" | "tool_calls" | "length" | "error" = "error";
    let finishObserved = false;

    while (true) {
      if (args.signal.aborted) {
        reader.cancel().catch(() => undefined);
        return;
      }
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      // SSE frames are separated by double newlines and include both
      // `event: foo` and `data: {...}` lines.
      const frames = buffer.split("\n\n");
      buffer = frames.pop() ?? "";
      for (const frame of frames) {
        const dataLine = frame
          .split("\n")
          .map((l) => l.trim())
          .find((l) => l.startsWith("data: "));
        if (!dataLine) continue;
        const payload = dataLine.slice(6);
        try {
          const json = JSON.parse(payload) as
            | {
                type: "content_block_start";
                index: number;
                content_block: { type: string; id?: string; name?: string };
              }
            | {
                type: "content_block_delta";
                index: number;
                delta: { type: string; text?: string; partial_json?: string };
              }
            | { type: "content_block_stop"; index: number }
            | { type: "message_delta"; delta: { stop_reason?: string } }
            | { type: "message_stop" }
            | { type: "ping" }
            | { type: "error"; error: { message?: string } };

          if (json.type === "content_block_start") {
            if (json.content_block.type === "tool_use") {
              toolBlocks.set(json.index, {
                id: json.content_block.id ?? "",
                name: json.content_block.name ?? "",
                argsText: "",
              });
            }
          } else if (json.type === "content_block_delta") {
            if (json.delta.type === "text_delta" && json.delta.text) {
              yield { type: "text", delta: json.delta.text };
            } else if (json.delta.type === "input_json_delta" && json.delta.partial_json) {
              const block = toolBlocks.get(json.index);
              if (block) {
                block.argsText += json.delta.partial_json;
                if (block.argsText.length > MAX_TOOL_ARGS_BYTES) {
                  reader.cancel().catch(() => undefined);
                  throw new Error(
                    `Anthropic tool-call arguments exceeded ${MAX_TOOL_ARGS_BYTES} bytes; aborting`,
                  );
                }
              }
            }
          } else if (json.type === "message_delta") {
            const reason = json.delta.stop_reason;
            if (reason) {
              finishObserved = true;
              finishReason = mapStopReason(reason);
            }
          } else if (json.type === "error") {
            throw new Error(`Anthropic stream error: ${json.error.message ?? "unknown"}`);
          }
        } catch (err) {
          if (err instanceof Error && err.message.startsWith("Anthropic ")) throw err;
          // Otherwise, ignore malformed frames.
        }
      }
    }

    for (const block of toolBlocks.values()) {
      if (!block.id || !block.name) continue;
      let parsed: Record<string, unknown>;
      try {
        parsed = JSON.parse(block.argsText || "{}") as Record<string, unknown>;
      } catch {
        parsed = { _raw: block.argsText };
      }
      yield {
        type: "tool_call",
        call: { id: block.id, name: block.name, arguments: parsed },
      };
    }

    yield { type: "done", reason: finishObserved ? finishReason : "error" };
  }
}

export function splitSystemAndMessages(messages: LlmMessage[]): {
  system: string | undefined;
  messages: AnthropicMessage[];
} {
  const systemParts: string[] = [];
  const out: AnthropicMessage[] = [];
  for (const m of messages) {
    if (m.role === "system") {
      systemParts.push(m.content);
      continue;
    }
    if (m.role === "user") {
      out.push({ role: "user", content: m.content });
      continue;
    }
    if (m.role === "assistant") {
      const blocks: AnthropicMessage["content"] = [];
      if (m.content) blocks.push({ type: "text", text: m.content });
      for (const tc of m.toolCalls ?? []) {
        blocks.push({ type: "tool_use", id: tc.id, name: tc.name, input: tc.arguments });
      }
      out.push({ role: "assistant", content: blocks.length === 0 ? "" : blocks });
      continue;
    }
    // role === 'tool' — Anthropic models tool results as a user message
    // containing a tool_result block.
    out.push({
      role: "user",
      content: [{ type: "tool_result", tool_use_id: m.toolCallId, content: m.content }],
    });
  }
  return { system: systemParts.join("\n\n") || undefined, messages: out };
}

function toAnthropicTool(tool: LlmTool) {
  return {
    name: tool.name,
    description: tool.description,
    input_schema: tool.parameters,
  };
}

export function mapStopReason(reason: string): "stop" | "tool_calls" | "length" | "error" {
  switch (reason) {
    case "end_turn":
    case "stop_sequence":
      return "stop";
    case "tool_use":
      return "tool_calls";
    case "max_tokens":
      return "length";
    default:
      return "error";
  }
}
