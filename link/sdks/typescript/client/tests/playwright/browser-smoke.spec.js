import { test, expect } from '@playwright/test';

function buildPageUrl(baseURL, pagePath) {
  const url = new URL(pagePath, `${baseURL}/`);
  url.searchParams.set('backend', `${baseURL}/backend`);
  url.searchParams.set('user', process.env.KALAMDB_USER ?? 'admin');
  url.searchParams.set('password', process.env.KALAMDB_PASSWORD ?? 'kalamdb123');
  return url.toString();
}

test('browser Apollo-style query/live smoke page passes', async ({ page, baseURL }) => {
  await page.goto(buildPageUrl(baseURL, 'tests/browser-apollo-e2e.html'));
  await page.waitForFunction(() => window.__browserApolloResult !== undefined);

  const result = await page.evaluate(() => window.__browserApolloResult);
  expect(result?.ok).toBe(true);
  await expect(page.locator('#status')).toContainText('PASS');
  await expect(page.locator('#feed li')).toHaveCount(2);
});

test('browser resume harness passes under Playwright', async ({ page, baseURL }) => {
  await page.goto(buildPageUrl(baseURL, 'tests/browser-resume-e2e.html'));
  await page.waitForFunction(() => window.__browserResumeResult !== undefined);

  const result = await page.evaluate(() => window.__browserResumeResult);
  expect(result?.ok).toBe(true);
  await expect(page.locator('#status')).toContainText('PASS');
});