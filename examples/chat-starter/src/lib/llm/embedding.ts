// Embedding helper used by RAG flows (seed-docs script + search_documents
// tool). Two modes:
//
//   - Real:  OpenAI text-embedding-3-small at dimensions=384 to match the
//            EMBEDDING(384) column. Cheap (~$0.02 per million tokens) and
//            plenty for English-language chunks of a few hundred tokens.
//
//   - Fake:  deterministic 384-dim vector derived from a hash of the text.
//            Used in unit tests and during seeding when EMBEDDING_PROVIDER
//            is unset. NOT semantically meaningful — nearest-neighbour
//            results in fake mode reflect string similarity at best.
//
// The mode is picked at call time, not boot, so a single process can mix
// fake (tests) with real (live agent) safely.

export const EMBEDDING_DIMENSIONS = 384;

export interface EmbedOptions {
  /** Override which model to call. Defaults to text-embedding-3-small. */
  model?: string;
  /** Override the embedding API base URL. */
  baseUrl?: string;
}

/**
 * Returns a 384-dim float vector for the given text. Uses OpenAI when
 * OPENAI_API_KEY is set; otherwise falls back to the deterministic fake.
 * Set EMBEDDING_PROVIDER=fake to force fake mode regardless of keys.
 */
export async function embed(text: string, opts: EmbedOptions = {}): Promise<number[]> {
  if (typeof text !== "string" || text.trim().length === 0) {
    throw new Error("embed: text must be a non-empty string");
  }
  const provider = pickEmbeddingProvider();
  if (provider === "openai") {
    return await embedWithOpenAI(text, opts);
  }
  return fakeEmbed(text);
}

function pickEmbeddingProvider(): "openai" | "fake" {
  const explicit = (process.env.EMBEDDING_PROVIDER ?? "").toLowerCase();
  if (explicit === "openai" || explicit === "fake") return explicit;
  return process.env.OPENAI_API_KEY ? "openai" : "fake";
}

async function embedWithOpenAI(text: string, opts: EmbedOptions): Promise<number[]> {
  const apiKey = process.env.OPENAI_API_KEY;
  if (!apiKey) throw new Error("OPENAI_API_KEY is not set");
  const baseUrl = opts.baseUrl ?? "https://api.openai.com/v1";
  const model = opts.model ?? "text-embedding-3-small";
  const res = await fetch(`${baseUrl}/embeddings`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${apiKey}`,
    },
    body: JSON.stringify({
      model,
      input: text,
      dimensions: EMBEDDING_DIMENSIONS,
    }),
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`OpenAI embedding request failed (${res.status}): ${body}`);
  }
  const json = (await res.json()) as { data?: Array<{ embedding?: number[] }> };
  const vec = json.data?.[0]?.embedding;
  if (!Array.isArray(vec) || vec.length !== EMBEDDING_DIMENSIONS) {
    throw new Error(`OpenAI returned unexpected embedding shape (length=${vec?.length ?? "n/a"})`);
  }
  return vec;
}

/**
 * Deterministic fake embedding. Uses two cheap independent rolling-hash
 * passes over the text and writes them into a 384-dim vector, then
 * normalizes to unit length so cosine distance still ranks meaningfully
 * between strings. Different strings yield different vectors; identical
 * strings yield identical vectors. Not semantically aware.
 */
export function fakeEmbed(text: string): number[] {
  const out = new Array<number>(EMBEDDING_DIMENSIONS).fill(0);
  let h1 = 2166136261; // FNV-1a 32-bit init
  let h2 = 5381;        // DJB2 init
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    h1 ^= c;
    h1 = Math.imul(h1, 16777619) >>> 0;
    h2 = (Math.imul(h2, 33) + c) >>> 0;
    out[i % EMBEDDING_DIMENSIONS] += ((h1 % 1000) - 500) / 500;
    out[(i + 7) % EMBEDDING_DIMENSIONS] += ((h2 % 1000) - 500) / 500;
  }
  // L2-normalize so cosine distance behaves predictably.
  let norm = 0;
  for (const v of out) norm += v * v;
  norm = Math.sqrt(norm) || 1;
  for (let i = 0; i < out.length; i++) out[i] = out[i]! / norm;
  return out;
}

/** Render a vector as a KalamDB embedding literal: '[v1, v2, ...]'. */
export function embeddingLiteral(vec: number[]): string {
  return `[${vec.map((v) => v.toFixed(6)).join(",")}]`;
}
