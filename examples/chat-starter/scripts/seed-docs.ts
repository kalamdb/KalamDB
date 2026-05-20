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

const URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:2900";
const USER = process.env.KALAMDB_USER ?? "root";
const PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";

interface DocSeed {
  id: string;
  title: string;
  source: string;
  body: string;
}

// The corpus is split into two groups by `source`:
//
//   - kalamdb.org/...  → real prose lifted from the public docs at the URL
//     in `source`. Replace these by re-fetching when the upstream docs
//     change. Each one is paraphrased lightly to fit a single embedding
//     window; the source URL is the canonical reference.
//
//   - examples/chat-starter/... → hand-written descriptions of THIS
//     starter app (streaming via typing_tokens, agent tools, the
//     auth-token broker, the chat schema). These describe the starter
//     itself, not KalamDB, so they don't have a kalamdb.org counterpart.
//
// The LLM cites the `title` field in answers; pretty-print it accordingly.
const DOCS: DocSeed[] = [
  // ---- kalamdb.org (real docs) -------------------------------------------
  {
    id: "doc-overview",
    title: "KalamDB Overview",
    source: "https://kalamdb.org/docs/server/getting-started",
    body:
      "KalamDB is a database system designed to streamline backend development with " +
      "integrated authentication, real-time capabilities, and multi-SDK support. It " +
      "distinguishes itself through several key features: live query and vector " +
      "search architecture components, multiple language SDKs (TypeScript, " +
      "Dart/Flutter, and Rust), and built-in topic pub/sub messaging for background " +
      "workers and automation. KalamDB also integrates with PostgreSQL through an " +
      "extension, enabling teams to leverage existing PostgreSQL workflows while " +
      "accessing modern database capabilities. The platform targets developers " +
      "building applications, AI-agent backends, and background automation systems.",
  },
  {
    id: "doc-live-queries",
    title: "Live Queries",
    source: "https://kalamdb.org/docs/server/architecture/live-query",
    body:
      "KalamDB implements native real-time functionality through WebSocket-based " +
      "Live Queries, eliminating the need for external message brokers. When clients " +
      "subscribe to queries on USER, SHARED, or STREAM tables, the system registers " +
      "a Live Query Subscription in the cluster rather than executing queries once. " +
      "This enables instant push notifications for any data modifications. The " +
      "offline-first design uses Sequence IDs for monotonically increasing operation " +
      "ordering — clients can request historical synchronization with `since_seq` " +
      "or `fetch_last`, then transition seamlessly to live streaming. In clustered " +
      "deployments, follower nodes evaluate subscriptions locally, distributing " +
      "client connections to increase concurrent capacity.",
  },
  {
    id: "doc-topics",
    title: "Topic Pub/Sub",
    source: "https://kalamdb.org/docs/topic-pubsub",
    body:
      "KalamDB topics function as a durable table-change feed where table " +
      "modifications automatically generate topic messages. Definitions live in " +
      "system.topics and consumer progress in system.topic_offsets; messages are " +
      "stored in RocksDB for reliable delivery. Consumer groups are named cursors " +
      "that let workers resume from where they left off. A visibility-timeout " +
      "mechanism (default 60s, `topics.visibility_timeout_secs`) temporarily " +
      "claims in-flight messages and redelivers them if no ack arrives — at-least- " +
      "once delivery semantics. Topics can also be auto-sourced from a table " +
      "(ALTER TOPIC ... ADD SOURCE ... ON INSERT) so a regular INSERT publishes " +
      "automatically — that's what the chat-starter's runConsumer agent reads from.",
  },
  {
    id: "doc-vectors",
    title: "Vector Search",
    source: "https://kalamdb.org/docs/server/architecture/vector-search",
    body:
      "KalamDB provides native vector search through EMBEDDING(n) columns combined " +
      "with cosine similarity indexing. The workflow: create a table with an " +
      "embedding column, build an index using the COSINE method, and rank query " +
      "results by distance — smaller COSINE_DISTANCE(...) values are more similar, " +
      "so the nearest matches appear first. A common pattern stores document " +
      "metadata in a main table and embeddings in a corresponding vectors table " +
      "indexed by the same identifier; for tenant-isolated scenarios this pairs " +
      "with TYPE = 'USER' so each user only searches their own vectors. Vector " +
      "search functionality persists across the hot and cold storage tiers, " +
      "making it suitable for long-term semantic search over growing collections.",
  },
  {
    id: "doc-storage-tiers",
    title: "Storage Tiers and Table Types",
    source: "https://kalamdb.org/docs/server/architecture/storage-tiers",
    body:
      "KalamDB uses a dual-tier storage system: a hot tier on RocksDB column " +
      "families handles incoming writes with sub-millisecond latency, then data " +
      "flushes to Apache Parquet in the cold tier for compression and analytical " +
      "efficiency. Tables are classified by type. USER tables hold per-user data " +
      "that can be isolated in separate storage directories, enabling " +
      "straightforward exports and GDPR-compliant deletions. SHARED tables hold " +
      "collaborative data across users without per-user partitioning. STREAM tables " +
      "operate outside the standard flush pipeline for event-streaming workloads. " +
      "SYSTEM tables are internal metadata. The chat-starter uses USER for the " +
      "conversation-scoped tables and SHARED for the RAG knowledge base.",
  },
  {
    id: "doc-clustering",
    title: "Clustering & High Availability",
    source: "https://kalamdb.org/docs/server/architecture/clustering",
    body:
      "KalamDB implements a Multi-Raft architecture: leadership is distributed " +
      "across multiple Raft consensus groups rather than centralized on one node. " +
      "Each group elects its own leader, so writes scale in parallel across " +
      "leaders rather than bottlenecking on a single one. Clients can connect to " +
      "any node — reads and subscription bootstrapping are served locally, while " +
      "writes are routed to the authoritative leader for the relevant shard group. " +
      "On leader changes, the system automatically retries through the new leader. " +
      "Live notification updates flow from shard leaders to follower nodes over " +
      "cluster RPC, so subscribers connected to any node receive real-time changes.",
  },
  {
    id: "doc-multitenant",
    title: "Multi-Tenant Isolation",
    source: "https://kalamdb.org/docs/use-cases/multi-tenant",
    body:
      "KalamDB supports multi-tenant SaaS through structured data isolation. USER " +
      "tables hold tenant-scoped operational data; SHARED tables hold global " +
      "metrics and catalog information. The model enables many tenants on shared " +
      "infrastructure with strong logical isolation. Enforcement combines role-" +
      "based access controls, optional tenant-specific service accounts, and " +
      "query-level filtering through tenant IDs — users only see records " +
      "belonging to their organization. Sensitive table types can layer " +
      "additional role-based restrictions on top.",
  },
  {
    id: "doc-security",
    title: "Security Model",
    source: "https://kalamdb.org/docs/security",
    body:
      "KalamDB treats security as an operator responsibility rather than baking " +
      "policies into the engine. Authentication uses JWT tokens — the operator " +
      "sets a strong `auth.jwt_secret` (keep it out of source control) and can " +
      "integrate with Firebase, Keycloak, or OIDC providers. Network: deploy " +
      "behind HTTPS, configure CORS and WebSocket origin restrictions, disable " +
      "remote setup after bootstrap, and restrict admin routes to trusted " +
      "networks. For clustered deployments, mTLS secures internal RPC. Rate " +
      "limits and request-size controls form the operational layer; the engine " +
      "doesn't enable protections by default, so operators must match config to " +
      "their threat model.",
  },
  {
    id: "doc-sdks",
    title: "SDKs",
    source: "https://kalamdb.org/docs/sdk",
    body:
      "KalamDB ships client libraries for several stacks: a TypeScript SDK for " +
      "browser/Node.js apps and worker services, a Dart/Flutter SDK for mobile, " +
      "and a Rust SDK for native async services and topic workers. All share " +
      "JWT bearer authentication for both SQL and real-time, with SQL routed to " +
      "an HTTP endpoint and WebSocket subscriptions handling live data streaming. " +
      "The SDKs also expose Topic consume/ack helpers (runConsumer in TypeScript) " +
      "and vector search integrated directly into SQL queries — giving a unified " +
      "surface for reactive applications that also need semantic search.",
  },

  // ---- examples/chat-starter (this app's own docs) -----------------------
  {
    id: "doc-streaming",
    title: "Token Streaming via typing_tokens (chat-starter)",
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
    title: "Stop Button via Live Query on chat.tasks (chat-starter)",
    source: "examples/chat-starter",
    body:
      "Cancellation in the chat-starter is just an UPDATE. The Stop button in the " +
      "UI runs UPDATE chat.tasks SET is_cancelled = true WHERE id = $1. The " +
      "agent has a per-task live query on its own task row; when is_cancelled " +
      "flips, the live update fires and the agent calls controller.abort() on " +
      "the in-flight LLM stream. The agent then finalizes the assistant message " +
      "with status='cancelled', and the UI renders a '(stopped)' marker.",
  },
  {
    id: "doc-approvals",
    title: "Human-in-the-Loop Approvals (chat-starter)",
    source: "examples/chat-starter",
    body:
      "Approvals in the chat-starter are a row pattern. When the LLM calls " +
      "request_approval(question), the agent INSERTs a row into chat.approvals " +
      "with status='pending', then blocks on a live query for that row. The UI " +
      "sees the new row and renders an Approve/Reject card. The user click " +
      "UPDATEs the row's status; the live query fires; the agent reads the " +
      "decision and either continues with the destructive tool call or aborts.",
  },
  {
    id: "doc-agent",
    title: "The Agent Worker (chat-starter)",
    source: "examples/chat-starter",
    body:
      "The agent is a Node.js worker process. It is NOT part of KalamDB; " +
      "KalamDB is the database. The agent reads new task events from a topic " +
      "via runConsumer, calls the LLM (OpenAI / Anthropic / mock), streams the " +
      "response back into KalamDB tables, and handles tool calls like " +
      "query_database, search_documents, delete_conversation, and " +
      "request_approval. You run as many replicas as you need; they coordinate " +
      "via the consumer group.",
  },
  {
    id: "doc-tools",
    title: "Agent Tools / Function Calling (chat-starter)",
    source: "examples/chat-starter",
    body:
      "The chat-starter exposes four tools to the LLM. request_approval pauses " +
      "for a human yes/no. query_database runs read-only SELECTs against the " +
      "chat namespace. delete_conversation cascades a destructive delete after " +
      "explicit approval — implemented as a single transactional BEGIN/COMMIT. " +
      "search_documents performs vector similarity search over this knowledge " +
      "base. The agent's dispatchTool routes each call to its handler. A " +
      "sql-guard layer rejects non-SELECT, comments outside string literals, " +
      "multi-statement input, or anything outside the chat namespace.",
  },
  {
    id: "doc-auth",
    title: "Browser Auth Broker (chat-starter)",
    source: "examples/chat-starter",
    body:
      "The browser in the chat-starter holds NO KalamDB credentials. It calls " +
      "POST /api/auth/token (served by a tiny Node backend in server/index.ts) " +
      "which logs into KalamDB with the bundled root credentials and returns " +
      "a JWT. The frontend uses that JWT to authenticate its live queries and " +
      "SQL. The backend is also where real per-user auth plugs in: validate " +
      "the user's session, mint a per-user scoped token, return it. The " +
      "starter ships an open token-vending machine for the demo and refuses " +
      "to start under NODE_ENV=production unless ALLOW_UNAUTHENTICATED_TOKENS=" +
      "true is set — a fence the operator must deliberately step over.",
  },
  {
    id: "doc-schema",
    title: "Chat Schema (chat-starter)",
    source: "examples/chat-starter/chat-app.sql",
    body:
      "The chat namespace has six tables. conversations, messages, " +
      "typing_tokens, approvals, and tasks are USER tables — they carry " +
      "_seq, _deleted, and _commit_seq system columns and participate in " +
      "live queries. docs is a SHARED table because the knowledge base is " +
      "global, not per-user. The tasks table is sourced into a topic " +
      "chat.task_events ON INSERT so runConsumer can drive the agent work " +
      "queue. The docs table has an EMBEDDING(384) column indexed USING " +
      "COSINE for RAG.",
  },
  {
    id: "doc-rag",
    title: "How RAG Works in the chat-starter",
    source: "examples/chat-starter",
    body:
      "Retrieval-augmented generation: when the user asks a fuzzy or " +
      "conceptual question (what is X, how does Y work), the LLM calls " +
      "search_documents(query). The agent embeds the query using OpenAI " +
      "text-embedding-3-small at 384 dimensions, then runs SELECT ... " +
      "ORDER BY COSINE_DISTANCE(embedding, ...) LIMIT 5 against chat.docs. " +
      "The top hits come back to the LLM as JSON with title + body. The LLM " +
      "phrases the answer in natural language and cites the doc titles. " +
      "Structured questions over the user's chat data (counts, lists) still " +
      "go through query_database — RAG is for fuzzy, conceptual queries.",
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
