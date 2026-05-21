import { test, expect, type Page } from "@playwright/test";

// These tests exercise the three pillars of the starter end-to-end:
//   1. streaming        — assistant text appears progressively
//   2. approvals        — destructive language triggers human-in-the-loop
//   3. cancellation     — Stop UPDATEs the task row and the agent finalizes
//
// They are deliberately written against the mock LLM adapter so behavior is
// deterministic. The mock recognizes "__slow_stream__" and emits a 20s slow
// stream that gives the cancellation test plenty of room to click Stop.
// To run them against a real model, drop the magic phrase and accept that
// the Stop test will be timing-sensitive.

// Tests use alice's partition (the default demo user). To keep test
// runs idempotent + the dev DB clean for manual testing, we wipe
// alice's data before each test starts and again when the suite ends.
const KALAMDB_URL = process.env.KALAMDB_URL ?? "http://127.0.0.1:2900";
const KALAMDB_ROOT_PASSWORD = process.env.KALAMDB_PASSWORD ?? "kalamdb-dev-password";
const TEST_USER = "alice";

async function adminLogin(): Promise<string> {
  const res = await fetch(`${KALAMDB_URL}/v1/api/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username: "root", password: KALAMDB_ROOT_PASSWORD }),
  });
  if (!res.ok) throw new Error(`admin login failed (${res.status})`);
  return ((await res.json()) as { access_token: string }).access_token;
}

async function adminExec(token: string, sql: string): Promise<void> {
  const res = await fetch(`${KALAMDB_URL}/v1/api/sql`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ sql }),
  });
  if (!res.ok) throw new Error(`exec failed (${res.status}): ${sql.slice(0, 80)}`);
}

async function countConversations(user: string): Promise<number> {
  const token = await adminLogin();
  const r = await fetch(`${KALAMDB_URL}/v1/api/sql`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({
      sql: `EXECUTE AS USER '${user}' (SELECT count(*) AS n FROM chat.conversations)`,
    }),
  });
  if (!r.ok) return -1;
  const body = (await r.json()) as { results?: Array<{ rows?: unknown[][] }> };
  const n = body.results?.[0]?.rows?.[0]?.[0];
  return Number(n ?? 0);
}

async function seedConversation(user: string, title: string): Promise<string> {
  const token = await adminLogin();
  const convId = crypto.randomUUID();
  const now = new Date().toISOString();
  await adminExec(
    token,
    `EXECUTE AS USER '${user}' (INSERT INTO chat.conversations (id, title, created_at, updated_at) VALUES ('${convId}', '${title.replace(/'/g, "''")}', '${now}', '${now}'))`,
  );
  return convId;
}

async function wipeUserPartition(user: string): Promise<void> {
  const token = await adminLogin();
  // Resolve any pending approvals so an agent process waiting on one
  // doesn't deadlock the consumer between test runs.
  const tables: Array<{ table: string; pk: string }> = [
    { table: "chat.typing_tokens", pk: "id" },
    { table: "chat.approvals", pk: "id" },
    { table: "chat.tasks", pk: "id" },
    { table: "chat.messages", pk: "id" },
    { table: "chat.conversations", pk: "id" },
  ];
  for (const { table, pk } of tables) {
    const r = await fetch(`${KALAMDB_URL}/v1/api/sql`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
      body: JSON.stringify({
        sql: `EXECUTE AS USER '${user}' (SELECT ${pk} FROM ${table})`,
      }),
    });
    if (!r.ok) continue;
    const body = (await r.json()) as { results?: Array<{ rows?: unknown[][] }> };
    const rows = body.results?.[0]?.rows ?? [];
    for (const row of rows) {
      const id = row[0];
      if (typeof id !== "string") continue;
      await adminExec(
        token,
        `EXECUTE AS USER '${user}' (DELETE FROM ${table} WHERE ${pk} = '${id}')`,
      );
    }
  }
}

test.beforeEach(async () => {
  await wipeUserPartition(TEST_USER);
  await wipeUserPartition("bob");
});

test.afterAll(async () => {
  await wipeUserPartition(TEST_USER);
  await wipeUserPartition("bob");
});

async function startNewChat(page: Page): Promise<void> {
  await page.goto("/");
  await page.getByRole("button", { name: /Start a new chat/i }).click();
  // Composer should be ready for input.
  await expect(page.locator("textarea").first()).toBeEnabled();
}

async function sendPrompt(page: Page, prompt: string): Promise<void> {
  const composer = page.locator("textarea").first();
  await composer.fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
}

async function waitForComposerReady(page: Page): Promise<void> {
  // "Ready" = textarea enabled AND the Send button visible (not Stop).
  await expect(page.locator("textarea").first()).toBeEnabled();
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
}

test("streaming: assistant reply appears and completes", async ({ page }) => {
  await startNewChat(page);
  await sendPrompt(page, "Hello! In one short sentence, what is KalamDB?");

  // While streaming, the Stop button replaces Send.
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible({ timeout: 15_000 });

  // Eventually the reply finalizes and the composer is usable again.
  await waitForComposerReady(page);

  // The assistant message bubble is present. Narrow by the Sparkles avatar
  // SVG that only assistant rows render — guards against the user bubble
  // matching by accident if its prompt also contains "KalamDB".
  const assistantBubble = page
    .locator("li")
    .filter({ has: page.locator(".lucide-sparkles") })
    .last();
  await expect(assistantBubble).toBeVisible();
});

test("approvals: destructive request shows Approve/Reject and actually deletes", async ({
  page,
}) => {
  await startNewChat(page);
  await sendPrompt(page, "Please delete my old account data immediately.");

  await expect(page.getByRole("button", { name: "Approve" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reject" })).toBeVisible();

  // Sanity: one conversation exists for alice right now.
  expect(await countConversations(TEST_USER)).toBe(1);

  await page.getByRole("button", { name: "Approve" }).click();

  // Approval card transitions to resolved state.
  await expect(page.getByText(/Approved/i).first()).toBeVisible({ timeout: 15_000 });

  // The conversation row must be gone from the DB. If delete_conversation's
  // cascade SQL is broken (e.g. parser rejects the BEGIN/COMMIT wrap),
  // the UI flow still passes but this assertion fails — exactly the
  // gap that hid the multi-statement EXECUTE AS USER bug last time.
  await expect(async () => {
    expect(await countConversations(TEST_USER)).toBe(0);
  }).toPass({ timeout: 10_000 });

  // Once deleted, the conversation disappears from the UI and we drop back
  // to the empty state. Composer doesn't render — that's expected.
  await expect(page.getByRole("button", { name: /Start a new chat/i })).toBeVisible({
    timeout: 10_000,
  });
});

test("bulk delete: 'delete all my conversations' approval wipes everything", async ({ page }) => {
  // Seed three extra conversations as alice so we can verify they ALL get
  // deleted (not just the one the user is currently chatting in).
  await seedConversation(TEST_USER, "old chat 1");
  await seedConversation(TEST_USER, "old chat 2");
  await seedConversation(TEST_USER, "old chat 3");

  await startNewChat(page);
  // 4 = 3 seeded + 1 created by startNewChat
  expect(await countConversations(TEST_USER)).toBe(4);

  await sendPrompt(page, "delete all my conversations");

  // Bulk-delete approval card mentions ALL.
  await expect(page.getByText(/Permanently delete ALL of your conversations/i)).toBeVisible({
    timeout: 15_000,
  });
  await page.getByRole("button", { name: "Approve" }).click();

  // EVERYTHING is gone — including the conversation the user was chatting in.
  // UI drops back to the empty state.
  await expect(async () => {
    expect(await countConversations(TEST_USER)).toBe(0);
  }).toPass({ timeout: 10_000 });
  await expect(page.getByRole("button", { name: /Start a new chat/i })).toBeVisible({
    timeout: 10_000,
  });
});

test("multi-tenant: alice's data is invisible from bob's partition", async ({ page }) => {
  // Seed two conversations as bob — a separate user, partitioned by KalamDB
  // at the engine level. alice (the default test user) must never see them.
  await seedConversation("bob", "bob's secret 1");
  await seedConversation("bob", "bob's secret 2");

  await page.goto("/");
  // Default user is alice. She must have zero conversations (beforeEach wiped).
  // Sidebar should not list "bob's secret …".
  await expect(page.getByText(/bob's secret/i)).toHaveCount(0);
  expect(await countConversations(TEST_USER)).toBe(0);
  // bob's side still has 2.
  expect(await countConversations("bob")).toBe(2);
});

test("cancellation: Stop mid-stream finalizes message as (stopped)", async ({ page }) => {
  await startNewChat(page);
  // The "__slow_stream__" magic phrase triggers the mock adapter's slow path
  // (200 chunks × 100ms ≈ 20s) so we have a deterministic streaming window.
  await sendPrompt(page, "__slow_stream__ please write a long response that I will stop");

  const stopBtn = page.getByRole("button", { name: "Stop" });
  await expect(stopBtn).toBeVisible({ timeout: 10_000 });

  // Wait for a couple of chunks to land before clicking, to ensure the
  // streaming path is fully engaged.
  await page.waitForTimeout(800);
  await stopBtn.click();

  // The cancelled message renders the "(stopped)" marker.
  await expect(page.getByText("(stopped)")).toBeVisible({ timeout: 15_000 });

  // Composer becomes ready again once the agent finalizes.
  await waitForComposerReady(page);
});
