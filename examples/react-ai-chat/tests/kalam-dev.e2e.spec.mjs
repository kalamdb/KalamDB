import { expect, test } from '@playwright/test';

const APP_URL = process.env.REACT_AI_CHAT_APP_URL ?? 'http://127.0.0.1:5176';

test('creates and sends a message through kalam dev without falling back to default namespace', async ({ page }) => {
  const messageText = `kalam dev e2e ${Date.now()}`;
  const failures = [];

  page.on('console', (message) => {
    if (message.type() === 'error') {
      failures.push(`console error: ${message.text()}`);
    }
  });
  page.on('pageerror', (error) => {
    failures.push(`page error: ${error.message}`);
  });
  page.on('response', async (response) => {
    if (!response.url().includes('/v1/api/sql')) {
      return;
    }
    if (response.status() >= 400) {
      const body = await response.text().catch(() => '');
      failures.push(`sql ${response.status()}: ${body}`);
    }
  });

  await page.goto(APP_URL, { waitUntil: 'networkidle' });
  await page.getByRole('button', { name: 'New Chat', exact: true }).click();
  await page.getByPlaceholder('Message GPT-4o...').fill(messageText);
  await page.getByRole('button', { name: 'Send message' }).click();
  await page.getByRole('article').filter({ hasText: messageText }).waitFor({ timeout: 10_000 });

  expect(failures.some((failure) => failure.includes('default.conversations')), failures.join('\n')).toBe(false);
  expect(failures).toEqual([]);
});
