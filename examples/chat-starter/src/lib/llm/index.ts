export type LlmMessage =
  | { role: "system"; content: string }
  | { role: "user"; content: string }
  | { role: "assistant"; content: string; toolCalls?: LlmToolCall[] }
  | { role: "tool"; toolCallId: string; content: string };

export interface LlmToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

export interface LlmTool {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

export type LlmStreamEvent =
  | { type: "text"; delta: string }
  | { type: "tool_call"; call: LlmToolCall }
  | { type: "done"; reason: "stop" | "tool_calls" | "length" | "error" };

export interface LlmStreamArgs {
  messages: LlmMessage[];
  tools?: LlmTool[];
  signal: AbortSignal;
}

export interface LlmAdapter {
  readonly name: string;
  stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined>;
}

let cached: LlmAdapter | null = null;

export async function getLlmAdapter(): Promise<LlmAdapter> {
  if (cached) return cached;
  if (process.env.OPENAI_API_KEY) {
    const { OpenAiAdapter } = await import("./openai.js");
    cached = new OpenAiAdapter({
      apiKey: process.env.OPENAI_API_KEY,
      model: process.env.OPENAI_MODEL ?? "gpt-4o-mini",
    });
    return cached;
  }
  const { MockAdapter } = await import("./mock.js");
  cached = new MockAdapter();
  console.warn(
    "[llm] No OPENAI_API_KEY set. Using mock adapter (canned replies). Set the env var to talk to OpenAI for real.",
  );
  return cached;
}
