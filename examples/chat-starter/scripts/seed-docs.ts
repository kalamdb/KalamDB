import "dotenv/config";
import { logger } from "../src/lib/logger.js";
import { embed, embeddingLiteral } from "../src/lib/llm/embedding.js";
import { SEED_SOURCE_TAG } from "../src/lib/constants.js";

// Seeds chat.docs (SHARED) with a hand-curated KalamDB knowledge base.
// Run via `npm run seed-docs`. Idempotent — wipes existing rows first.
//
// Uses OpenAI embeddings when OPENAI_API_KEY is set, otherwise falls back
// to the deterministic fake embedder (suitable for offline demos; nearest-
// neighbour matches will be poorer but the flow still works end-to-end).

const log = logger.child({ component: "seed-docs" });

const URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:8080";
const USER = process.env.KALAMDB_USER ?? "root";
const PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";

interface DocSeed {
  id: string;
  title: string;
  source: string;
  body: string;
}

const DOCS: DocSeed[] = [
  {
    id: "doc-overview",
    title: "KalamDB Overview",
    source: "kalamdb.org",
    body:
      "KalamDB is a database with a SQL surface and built-in realtime primitives. " +
      "It speaks ordinary SELECT/INSERT/UPDATE/DELETE, plus a few extensions: " +
      "topics (Kafka-style pub/sub queues), live queries (subscriptions that push " +
      "matching row changes as they happen), and vector storage (EMBEDDING columns " +
      "with cosine / L2 distance functions). The goal is to be 'one system' for " +
      "transactional data, the event bus, and the vector store an AI app needs.",
  },
  {
    id: "doc-live-queries",
    title: "Live queries",
    source: "specs/014-live-queries-websocket",
    body:
      "A live query is a SELECT that the client subscribes to. The server holds " +
      "the query plan and pushes diffs (inserts, updates, deletes) over the same " +
      "connection as soon as they happen. From the client's perspective it looks " +
      "like a regular SELECT that magically updates — no polling, no SSE, no " +
      "bespoke WebSocket protocol. In the chat-starter, the React UI uses one " +
      "live query per visible collection (messages, typing_tokens, approvals, " +
      "tasks, conversations) and renders rows directly.",
  },
  {
    id: "doc-topics",
    title: "Topics & runConsumer",
    source: "specs/024-topic-pubsub",
    body:
      "Topics are an append-only stream of events. You can publish to a topic " +
      "explicitly, or auto-source from a table: ALTER TOPIC chat.task_events ADD " +
      "SOURCE chat.tasks ON INSERT. Consumers join a group_id and read with at-" +
      "least-once delivery + automatic load balancing across replicas. The TS " +
      "SDK's runConsumer wraps the polling / acking / reconnection loop. The " +
      "chat-starter agent uses runConsumer on a topic sourced from chat.tasks " +
      "so multiple agent replicas can share the work queue.",
  },
  {
    id: "doc-streaming",
    title: "Token streaming via typing_tokens",
    source: "examples/chat-starter",
    body:
      "Token streaming in the chat-starter is implemented WITHOUT SSE or any " +
      "custom WebSocket protocol. The agent batches LLM token deltas and INSERTs " +
      "each batch as a row in chat.typing_tokens. The frontend has a live query " +
      "on typing_tokens; new rows appear automatically and are concatenated " +
      "client-side to render the in-flight assistant body. When the message " +
      "finalizes, the agent clears typing_tokens for that message.",
  },
  {
    id: "doc-cancellation",
    title: "Cancellation (Stop button)",
    source: "examples/chat-starter",
    body:
      "Cancellation is just an UPDATE. The Stop button in the UI runs UPDATE " +
      "chat.tasks SET is_cancelled = true WHERE id = $1. The agent has a live " +
      "query on its own task row; when is_cancelled flips, the live update fires " +
      "and the agent calls controller.abort() on the in-flight LLM stream. The " +
      "agent then finalizes the assistant message with status='cancelled', and " +
      "the UI renders a '(stopped)' marker.",
  },
  {
    id: "doc-approvals",
    title: "Human-in-the-loop approvals",
    source: "examples/chat-starter",
    body:
      "Approvals are a row pattern. When the LLM calls request_approval(question), " +
      "the agent INSERTs a row into chat.approvals with status='pending', then " +
      "blocks on a live query for that row. The UI sees the new row and renders " +
      "an Approve/Reject card. The user click UPDATEs the row's status; the live " +
      "query fires; the agent reads the decision and either continues with the " +
      "destructive tool call or aborts.",
  },
  {
    id: "doc-agent",
    title: "What is the agent?",
    source: "examples/chat-starter",
    body:
      "The agent is a Node.js worker process. It is NOT part of KalamDB; KalamDB " +
      "is a database. The agent reads new task events from a topic, calls the LLM " +
      "(OpenAI / Anthropic / mock), streams the response back into KalamDB tables, " +
      "and handles tool calls like query_database, search_documents, " +
      "delete_conversation, and request_approval. You run as many replicas as you " +
      "need; they coordinate via the consumer group.",
  },
  {
    id: "doc-tools",
    title: "Agent tools (function calling)",
    source: "examples/chat-starter",
    body:
      "The chat-starter exposes four tools to the LLM. request_approval pauses " +
      "for a human yes/no. query_database runs read-only SELECTs against the chat " +
      "namespace. delete_conversation cascades a destructive delete after explicit " +
      "approval. search_documents performs vector similarity search over the " +
      "knowledge base (this table). The agent's dispatchTool routes each tool " +
      "call to its handler. A sql-guard layer rejects non-SELECT, comments, " +
      "multi-statement input, or anything outside the chat namespace.",
  },
  {
    id: "doc-vectors",
    title: "Vector storage in KalamDB",
    source: "kalamdb.org / specs",
    body:
      "KalamDB has first-class vector support. Declare a column as EMBEDDING(N) " +
      "and you get fixed-dimensional float vectors. Use COSINE_DISTANCE(col, " +
      "'[v1, v2, ...]') in ORDER BY or WHERE to do nearest-neighbour search. " +
      "ALTER TABLE name CREATE INDEX colname USING COSINE builds a vector index " +
      "in the background so queries scale beyond brute force. The chat-starter " +
      "uses an EMBEDDING(384) column on chat.docs to power RAG.",
  },
  {
    id: "doc-security",
    title: "Browser auth model",
    source: "examples/chat-starter",
    body:
      "The browser holds NO KalamDB credentials. It calls POST /api/auth/token " +
      "(served by a tiny Node backend in server/index.ts) which logs into KalamDB " +
      "with bundled root credentials and returns a JWT. The frontend uses that " +
      "JWT to authenticate its live queries and SQL. The backend is also where " +
      "real per-user auth plugs in: validate the user's session, mint a per-user " +
      "scoped token, return it. Today the starter mints a shared root token to " +
      "keep the demo focused on the realtime primitives.",
  },
  {
    id: "doc-schema",
    title: "Chat schema",
    source: "examples/chat-starter/chat-app.sql",
    body:
      "The chat namespace has six tables. conversations, messages, typing_tokens, " +
      "approvals, and tasks are USER tables — they carry _seq, _deleted, and " +
      "_commit_seq system columns and participate in live queries. docs is a " +
      "SHARED table because the knowledge base is global, not per-user. tasks " +
      "is sourced into a topic chat.task_events ON INSERT so runConsumer can " +
      "drive the work queue.",
  },
  {
    id: "doc-rag",
    title: "How RAG works here",
    source: "examples/chat-starter",
    body:
      "Retrieval-augmented generation: when the user asks a fuzzy question (what " +
      "is X, how does Y work), the LLM calls search_documents(query). The agent " +
      "embeds the query using OpenAI text-embedding-3-small at 384 dimensions, " +
      "then runs SELECT ... ORDER BY COSINE_DISTANCE(embedding, ...) LIMIT 5 " +
      "against chat.docs. The top hits come back to the LLM as JSON with title + " +
      "body. The LLM phrases the answer in natural language and cites the doc " +
      "titles. Structured questions (counts, lists over the user's chat data) " +
      "still go through query_database — RAG is for fuzzy, conceptual queries.",
  },
];

async function login(): Promise<string> {
  const res = await fetch(`${URL}/v1/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username: USER, password: PASSWORD }),
  });
  if (!res.ok) {
    throw new Error(`Login failed (${res.status}): ${await res.text().catch(() => "")}`);
  }
  const body = (await res.json()) as { access_token: string };
  return body.access_token;
}

// All seed SQL is constructed here from values we control (seeded DOCS
// array + helper functions); we use the single-statement {sql} shape and
// escape via sqlString() consistently. No parameterized $1 binding —
// keeping the seed script uniform makes the cascade easier to read.
async function execSql(token: string, sql: string): Promise<unknown> {
  const res = await fetch(`${URL}/v1/api/sql`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ sql }),
  });
  if (!res.ok) {
    throw new Error(
      `SQL failed (${res.status}): ${sql.slice(0, 80)} — ${await res.text().catch(() => "")}`,
    );
  }
  return res.json();
}

function sqlString(value: string): string {
  return "'" + value.replace(/'/g, "''") + "'";
}

async function main(): Promise<void> {
  const token = await login();
  log.info({ url: URL, count: DOCS.length }, "seed-docs starting");

  // Wipe by the SEED_SOURCE_TAG prefix (not by id) so renamed or removed
  // doc ids from previous runs don't leave orphans behind. We stamp every
  // seed row with source = `<tag>:<human-source>` below.
  await execSql(
    token,
    `DELETE FROM chat.docs WHERE source LIKE ${sqlString(SEED_SOURCE_TAG + ":%")}`,
  );
  log.info({ tag: SEED_SOURCE_TAG }, "chat.docs wiped");

  for (const doc of DOCS) {
    const vec = await embed(`${doc.title}\n\n${doc.body}`);
    const vecLit = embeddingLiteral(vec);
    const now = new Date().toISOString();
    // Embed the per-doc source as `<SEED_SOURCE_TAG>:<doc-source>` so the
    // LLM still sees the human-friendly origin in citations, while the
    // wipe predicate above can match every seed regardless of doc id.
    const taggedSource = `${SEED_SOURCE_TAG}:${doc.source}`;
    const sql =
      `INSERT INTO chat.docs (id, title, body, source, embedding, created_at) VALUES (` +
      `${sqlString(doc.id)}, ${sqlString(doc.title)}, ${sqlString(doc.body)}, ` +
      `${sqlString(taggedSource)}, '${vecLit}', ${sqlString(now)})`;
    await execSql(token, sql);
    log.info({ id: doc.id, title: doc.title }, "indexed");
  }

  log.info({ count: DOCS.length }, "seed-docs done");
}

main().catch((err) => {
  log.fatal({ err }, "seed-docs failed");
  process.exit(1);
});
