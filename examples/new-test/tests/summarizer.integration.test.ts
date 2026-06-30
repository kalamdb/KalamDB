import { execFileSync } from 'node:child_process';
import path from 'node:path';
import test from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as sleep } from 'node:timers/promises';
import { config as loadEnv } from 'dotenv';
import { Auth, createClient } from '@kalamdb/client';
import { buildSummary, startSummarizerAgent } from '../src/agent.js';

const exampleRoot = path.resolve(process.cwd());

function readRuntimeConfig() {
  loadEnv({ path: path.join(exampleRoot, '.env.local'), quiet: true });

  return {
    serverUrl: process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900',
    user: process.env.KALAMDB_USER ?? 'root',
    password: process.env.KALAMDB_PASSWORD ?? 'kalamdb123',
  };
}

async function waitForSummary(
  client: ReturnType<typeof createClient>,
  content: string,
): Promise<string | null> {
  const deadline = Date.now() + 20_000;

  while (Date.now() < deadline) {
    const row = await client.queryOne(
      'SELECT blog_id, summary FROM blog.blogs WHERE content = $1 ORDER BY created DESC LIMIT 1',
      [content],
    );
    const summary = row?.summary?.asString() ?? null;
    if (summary) {
      return summary;
    }
    await sleep(400);
  }

  return null;
}

async function waitForCommittedOffset(
  client: ReturnType<typeof createClient>,
  groupId: string,
  minOffset = 0,
): Promise<number> {
  const deadline = Date.now() + 20_000;

  while (Date.now() < deadline) {
    const row = await client.queryOne(
      'SELECT last_acked_offset FROM system.topic_offsets WHERE topic_id = $1 AND group_id = $2 AND partition_id = 0',
      ['blog.summarizer', groupId],
    );
    const offset = row?.last_acked_offset?.asInt() ?? null;
    if (offset !== null && offset >= minOffset) {
      return offset;
    }
    await sleep(300);
  }

  throw new Error(`Timed out waiting for committed offset for group ${groupId}`);
}

async function waitForStableCommittedOffset(
  client: ReturnType<typeof createClient>,
  groupId: string,
): Promise<number> {
  const deadline = Date.now() + 20_000;
  let lastOffset = await waitForCommittedOffset(client, groupId);
  let stableSince = Date.now();

  while (Date.now() < deadline) {
    await sleep(250);
    const currentOffset = await waitForCommittedOffset(client, groupId, lastOffset);
    if (currentOffset !== lastOffset) {
      lastOffset = currentOffset;
      stableSince = Date.now();
      continue;
    }

    if (Date.now() - stableSince >= 1_000) {
      return currentOffset;
    }
  }

  throw new Error(`Timed out waiting for committed offset to stabilize for group ${groupId}`);
}

test('agent writes summaries back into blog.blogs', async () => {
  execFileSync(process.execPath, ['setup.mjs'], {
    cwd: exampleRoot,
    stdio: 'inherit',
    env: {
      ...process.env,
      KALAMDB_URL: process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900',
    },
  });

  const { serverUrl, user, password } = readRuntimeConfig();

  const controller = new AbortController();
  const agentRun = startSummarizerAgent({
    stopSignal: controller.signal,
    groupId: `blog-summarizer-test-${Date.now()}`,
    start: 'earliest',
  });
  await sleep(750);

  const client = createClient({
    url: serverUrl,
    authProvider: async () => Auth.basic(user, password),
  });

  const content = `KalamDB topics wake lightweight workers immediately after a row changes ${Date.now()}. The worker can enrich the row without polling.`;
  const expected = buildSummary(content);

  try {
    await client.query('INSERT INTO blog.blogs (content, summary) VALUES ($1, $2)', [content, null]);
    const summary = await waitForSummary(client, content);

    assert.equal(summary, expected);
  } finally {
    controller.abort();
    await Promise.race([agentRun, sleep(3_000)]);
    await client.disconnect();
  }
});

test('agent resumes the same group without replaying completed messages', async () => {
  execFileSync(process.execPath, ['setup.mjs'], {
    cwd: exampleRoot,
    stdio: 'inherit',
    env: {
      ...process.env,
      KALAMDB_URL: process.env.KALAMDB_URL ?? 'http://127.0.0.1:2900',
    },
  });

  const { serverUrl, user, password } = readRuntimeConfig();
  const client = createClient({
    url: serverUrl,
    authProvider: async () => Auth.basic(user, password),
  });

  const groupId = `blog-summarizer-resume-${Date.now()}`;
  const firstController = new AbortController();
  const firstRun = startSummarizerAgent({
    stopSignal: firstController.signal,
    groupId,
    start: 'latest',
  });
  await sleep(750);

  const firstContent = `First resumable summary payload ${Date.now()} with enough text to trigger the summarizer.`;
  const firstExpected = buildSummary(firstContent);

  let secondController: AbortController | null = null;
  let secondRun: Promise<void> | null = null;

  try {
    await client.query('INSERT INTO blog.blogs (content, summary) VALUES ($1, $2)', [firstContent, null]);
    assert.equal(await waitForSummary(client, firstContent), firstExpected);

    const firstAckedOffset = await waitForStableCommittedOffset(client, groupId);
    assert.ok(firstAckedOffset >= 0, 'expected the first run to commit an offset');

    firstController.abort();
    await Promise.race([firstRun, sleep(3_000)]);

    const replayRows = await client.queryAll(
      `CONSUME FROM blog.summarizer GROUP '${groupId}' FROM EARLIEST LIMIT 10`,
    );
    assert.equal(
      replayRows.length,
      0,
      'completed summarizer work should not remain replayable for the same group',
    );

    secondController = new AbortController();
    secondRun = startSummarizerAgent({
      stopSignal: secondController.signal,
      groupId,
      start: 'earliest',
    });
    await sleep(750);

    const secondContent = `Second resumable summary payload ${Date.now()} proves the group continues from its committed offset.`;
    const secondExpected = buildSummary(secondContent);

    await client.query('INSERT INTO blog.blogs (content, summary) VALUES ($1, $2)', [secondContent, null]);
    assert.equal(await waitForSummary(client, secondContent), secondExpected);

    const secondAckedOffset = await waitForCommittedOffset(client, groupId, firstAckedOffset + 1);
    assert.ok(
      secondAckedOffset > firstAckedOffset,
      'restarted agent should advance the same consumer-group offset',
    );
  } finally {
    firstController.abort();
    if (secondController) {
      secondController.abort();
    }
    await Promise.race([firstRun, sleep(3_000)]);
    if (secondRun) {
      await Promise.race([secondRun, sleep(3_000)]);
    }
    await client.disconnect();
  }
});