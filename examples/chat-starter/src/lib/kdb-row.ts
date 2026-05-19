// KalamDB result-row helpers shared by the agent and the agent's tools.
// The WASM client returns cells as wrapped objects exposing asString() or
// toJson(); unwrap() reaches in once and produces a plain JS value the rest
// of the code can pattern-match against.

export function unwrap(value: unknown): unknown {
  if (value && typeof value === "object" && "asString" in value) {
    return (value as { asString: () => string }).asString();
  }
  if (value && typeof value === "object" && "toJson" in value) {
    return (value as { toJson: () => unknown }).toJson();
  }
  return value;
}
