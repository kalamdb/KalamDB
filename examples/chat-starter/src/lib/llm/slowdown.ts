// Recorder-only LLM-stream slowdown.
//
// Imported unconditionally by src/agent/index.ts but a no-op unless the
// agent is launched with RECORDER_SLOWDOWN_MS=<positive int>. The recorder
// script sets that env when capturing the Stop-mid-stream demo so the
// streaming window is long enough for a Playwright click to land before a
// fast model finishes. Production deployments leave the env unset and pay
// zero cost — withSlowdown returns the inner adapter unchanged.

import type { LlmAdapter, LlmStreamArgs, LlmStreamEvent } from "./index.js";

export function withSlowdown(inner: LlmAdapter, ms: number): LlmAdapter {
  if (!Number.isFinite(ms) || ms <= 0) return inner;
  return {
    name: `${inner.name}:slow(${ms}ms)`,
    async *stream(args: LlmStreamArgs): AsyncGenerator<LlmStreamEvent, void, undefined> {
      for await (const event of inner.stream(args)) {
        if (args.signal.aborted) return;
        // Sleep only on token-bearing events so a model that produces one
        // huge text block doesn't slip past the cancel window.
        if (event.type === "text") {
          await sleep(ms, args.signal);
        }
        yield event;
      }
    },
  };
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve();
    const t = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    const onAbort = (): void => {
      clearTimeout(t);
      resolve();
    };
    signal.addEventListener("abort", onAbort, { once: true });
  });
}
