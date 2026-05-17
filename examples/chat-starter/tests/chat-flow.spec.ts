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

test("approvals: destructive request shows Approve/Reject", async ({ page }) => {
  await startNewChat(page);
  await sendPrompt(page, "Please delete my old account data immediately.");

  await expect(page.getByRole("button", { name: "Approve" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reject" })).toBeVisible();

  await page.getByRole("button", { name: "Approve" }).click();

  // Approval card transitions to resolved state.
  await expect(page.getByText(/Approved/i).first()).toBeVisible({ timeout: 15_000 });
  await waitForComposerReady(page);
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
