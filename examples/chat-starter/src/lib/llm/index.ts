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

/**
 * Thrown by an LlmAdapter when the upstream provider returned a non-OK HTTP
 * status. Carries the status code so the retry layer can decide what's
 * transient (429, 5xx) vs terminal (4xx) without parsing strings.
 */
export class LlmHttpError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "LlmHttpError";
    this.status = status;
  }
}

let cached: LlmAdapter | null = null;

/** Test helper — drops the cached adapter so a subsequent getLlmAdapter()
 *  re-runs the provider-pick logic against the current env. Production
 *  code should never call this. */
export function resetCachedAdapterForTests(): void {
  cached = null;
}

type Provider = "openai" | "anthropic" | "mock";

function pickProvider(): Provider {
  const explicit = (process.env.LLM_PROVIDER ?? "").toLowerCase();
  if (explicit === "openai" || explicit === "anthropic" || explicit === "mock") {
    return explicit;
  }
  if (process.env.OPENAI_API_KEY) return "openai";
  if (process.env.ANTHROPIC_API_KEY) return "anthropic";
  return "mock";
}

export async function getLlmAdapter(): Promise<LlmAdapter> {
  if (cached) return cached;
  const provider = pickProvider();
  if (provider === "openai") {
    if (!process.env.OPENAI_API_KEY) {
      throw new Error("LLM_PROVIDER=openai but OPENAI_API_KEY is not set");
    }
    const { OpenAiAdapter } = await import("./openai.js");
    cached = new OpenAiAdapter({
      apiKey: process.env.OPENAI_API_KEY,
      model: process.env.OPENAI_MODEL ?? "gpt-4o-mini",
    });
    return cached;
  }
  if (provider === "anthropic") {
    if (!process.env.ANTHROPIC_API_KEY) {
      throw new Error("LLM_PROVIDER=anthropic but ANTHROPIC_API_KEY is not set");
    }
    const { AnthropicAdapter } = await import("./anthropic.js");
    cached = new AnthropicAdapter({
      apiKey: process.env.ANTHROPIC_API_KEY,
      model: process.env.ANTHROPIC_MODEL ?? "claude-haiku-4-5-20251001",
    });
    return cached;
  }
  const { MockAdapter } = await import("./mock.js");
  cached = new MockAdapter();
  console.warn(
    "[llm] No OPENAI_API_KEY / ANTHROPIC_API_KEY set. Using mock adapter (canned replies). Set one to use a real model.",
  );
  return cached;
}
