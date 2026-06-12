import { test, expect } from '@playwright/test';

test('two tabs stay in sync through the live SQL subscription', async ({ browser, baseURL }) => {
  const context = await browser.newContext();
  const pageOne = await context.newPage();
  const pageTwo = await context.newPage();
  const uniqueMessage = `playwright-${Date.now()}`;

  await pageOne.goto(baseURL!);
  await pageTwo.goto(baseURL!);

  await expect(pageOne.getByRole('heading', { name: 'Realtime Ops Feed' })).toBeVisible();
  await expect(pageTwo.getByRole('heading', { name: 'Realtime Ops Feed' })).toBeVisible();

  await expect(pageOne.getByTestId('connection-status')).toContainText('Live', { timeout: 30_000 });
  await expect(pageTwo.getByTestId('connection-status')).toContainText('Live', { timeout: 30_000 });

  await pageOne.getByLabel('Service').fill('shipping');
  await pageOne.getByLabel('Level').selectOption('critical');
  await pageOne.getByLabel('Actor').fill('playwright');
  await pageOne.getByLabel('Message').fill(uniqueMessage);
  await pageOne.getByRole('button', { name: 'Broadcast event' }).click();

  await expect(pageOne.getByTestId('feed-list')).toContainText(uniqueMessage);
  await expect(pageTwo.getByTestId('feed-list')).toContainText(uniqueMessage);
});