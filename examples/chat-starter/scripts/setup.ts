import "dotenv/config";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { splitStatements } from "./sql-split.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const SQL_FILE = resolve(__dirname, "..", "chat-app.sql");

const URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:8080";
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

async function exec(token: string, sql: string): Promise<void> {
  const res = await fetch(`${URL}/v1/api/sql`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ sql }),
  });
  if (!res.ok) {
    throw new Error(`SQL failed (${res.status}): ${sql.slice(0, 80)}\n${await res.text().catch(() => "")}`);
  }
}

async function main(): Promise<void> {
  const text = readFileSync(SQL_FILE, "utf8");
  const statements = splitStatements(text);

  const token = await login();
  console.log(`[setup] logged into ${URL} as ${USER}`);
  for (const stmt of statements) {
    const head = stmt.split("\n")[0]!.slice(0, 60);
    try {
      await exec(token, stmt);
      console.log(`[setup] ok    ${head}`);
    } catch (err) {
      // DROP statements are best-effort on a fresh database where the
      // target doesn't exist yet. Anything else is a hard failure.
      if (/^\s*DROP\b/i.test(stmt)) {
        const detail = (err as Error).message.split("\n")[0];
        console.log(`[setup] skip  ${head}  (${detail})`);
        continue;
      }
      throw err;
    }
  }
  console.log("[setup] done");
}

main().catch((err) => {
  console.error("[setup] failed:", err);
  process.exit(1);
});
