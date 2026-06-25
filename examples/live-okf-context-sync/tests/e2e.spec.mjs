import { expect, test } from '@playwright/test';

test('shows live synced files for alice', async ({ page }) => {
  await page.goto('/', { waitUntil: 'networkidle' });
  await expect(page.getByRole('heading', { name: 'Live OKF Context Sync' })).toBeVisible();
  await expect(page.getByText('profile.md')).toBeVisible({ timeout: 15_000 });
});
