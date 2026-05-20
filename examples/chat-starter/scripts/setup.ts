import "dotenv/config";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { splitStatements } from "./sql-split.js";
import { logger } from "../src/lib/logger.js";

const log = logger.child({ component: "setup" });

const __dirname = dirname(fileURLToPath(import.meta.url));
const SQL_FILE = resolve(__dirname, "..", "chat-app.sql");

const URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:2900";
const USER = process.env.KALAMDB_USER ?? "root";
const PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";

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

/** Counts every row across the chat namespace's USER tables — a single
 *  non-zero return means setup will destroy real data. Returns 0 if the
 *  namespace doesn't exist yet (i.e. fresh database). */
async function countExistingData(token: string): Promise<number> {
  const tables = ["conversations", "messages", "tasks", "approvals", "typing_tokens"];
  let total = 0;
  for (const tbl of tables) {
    const res = await fetch(`${URL}/v1/api/sql`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
      body: JSON.stringify({ sql: `SELECT count(*) AS n FROM chat.${tbl}` }),
    });
    if (!res.ok) {
      // Most likely the table / namespace doesn't exist — first run.
      continue;
    }
    const body = (await res.json()) as {
      results?: Array<{ rows?: Array<Array<unknown>> }>;
    };
    const n = Number(body.results?.[0]?.rows?.[0]?.[0] ?? 0);
    total += n;
  }
  return total;
}

async function exec(token: string, sql: string): Promise<void> {
  const res = await fetch(`${URL}/v1/api/sql`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ sql }),
  });
  if (!res.ok) {
    throw new Error(
      `SQL failed (${res.status}): ${sql.slice(0, 80)}\n${await res.text().catch(() => "")}`,
    );
  }
}

async function main(): Promise<void> {
  const text = readFileSync(SQL_FILE, "utf8");
  const statements = splitStatements(text);

  const token = await login();
  log.info({ url: URL, user: USER }, "logged in");

  // chat-app.sql leads with DROP NAMESPACE — re-running setup wipes
  // every conversation, message, task, and approval across all demo
  // users. Refuse to proceed without explicit confirmation if the
  // namespace already has any user data, so a tab-completion mistake
  // or a re-typed quick-start command can't silently nuke a session.
  const force = process.argv.includes("--force");
  const existing = await countExistingData(token);
  if (existing > 0 && !force) {
    log.fatal(
      { existing_rows: existing },
      "REFUSING TO RUN: chat namespace already has data. " +
        "Re-running setup will DROP the namespace and delete every conversation " +
        "across all demo users. Pass --force if that's what you want.",
    );
    process.exit(2);
  }
  if (force && existing > 0) {
    log.warn({ existing_rows: existing }, "--force given; wiping existing data");
  }

  for (const stmt of statements) {
    const head = stmt.split("\n")[0]!.slice(0, 60);
    try {
      await exec(token, stmt);
      log.info({ stmt: head }, "ok");
    } catch (err) {
      const msg = (err as Error).message;
      const detail = msg.split("\n")[0];
      // DROP statements are best-effort on a fresh database where the
      // target doesn't exist yet.
      if (/^\s*DROP\b/i.test(stmt)) {
        log.info({ stmt: head, reason: detail }, "skip");
        continue;
      }
      // CREATE USER fails if the user already exists (KalamDB's DROP USER is
      // a soft-delete that prevents re-creation, so there's no clean way to
      // make this idempotent on the SQL side). Treat "already exists" as
      // success so re-running setup is safe.
      if (/^\s*CREATE\s+USER\b/i.test(stmt) && /already exists/i.test(msg)) {
        log.info({ stmt: head }, "user already exists, ok");
        continue;
      }
      throw err;
    }
  }
  log.info("schema setup complete");
}

main().catch((err) => {
  log.fatal({ err }, "setup failed");
  process.exit(1);
});
