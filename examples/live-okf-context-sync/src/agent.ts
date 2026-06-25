#!/usr/bin/env node
import 'dotenv/config';
import type { RowData } from '@kalamdb/client';
import { createKalamClient, resolveKalamConnection, TABLE } from './client.js';
import { downloadFileText } from './file-store.js';
import { stripFrontmatter } from './frontmatter.js';

type ContextRow = {
  path: string;
  sha256: string;
};

async function loadCanonicalFiles(): Promise<ContextRow[]> {
  const connection = resolveKalamConnection();
  const client = createKalamClient(connection);
  await client.initialize();

  const rows = await client.queryAll(
    `SELECT path, sha256
     FROM ${TABLE}
     WHERE deleted = false AND is_conflict = false
     ORDER BY path`,
  );

  await client.disconnect();
  return rows.map((row: RowData) => ({
    path: row.path?.asString() ?? '',
    sha256: row.sha256?.asString() ?? '',
  }));
}

async function buildAgentContext(accessToken: string): Promise<string> {
  const connection = resolveKalamConnection();
  const client = createKalamClient(connection);
  await client.initialize();
  const login = await client.login();
  const token = accessToken || login.access_token;

  const rows = await loadCanonicalFiles();
  const chunks: string[] = [];

  for (const row of rows) {
    const text = await downloadFileText(client, row.path, token);
    const body = row.path.endsWith('.md') ? stripFrontmatter(text) : text;
    chunks.push(`## ${row.path}\n${body.trim()}`);
  }

  await client.disconnect();
  return chunks.join('\n\n');
}

function answerFromContext(question: string, context: string): string {
  const lower = question.toLowerCase();

  if (lower.includes('profile') || lower.includes('who')) {
    const profile = context.match(/## profile\.md\s+([\s\S]*?)(?=\n## |\s*$)/i);
    if (profile) {
      return profile[1].trim();
    }
  }

  if (lower.includes('project') || lower.includes('kalamdb')) {
    const project = context.match(/## projects\/kalamdb\.md\s+([\s\S]*?)(?=\n## |\s*$)/i);
    if (project) {
      return project[1].trim();
    }
  }

  return context.slice(0, 1200) || 'No live context is available yet. Edit a file under context/alice/ and wait for sync.';
}

async function main(): Promise<void> {
  const question = process.argv.slice(2).join(' ') || 'Tell me about the profile.';
  const context = await buildAgentContext('');
  const answer = answerFromContext(question, context);

  console.log('[agent] question:', question);
  console.log('[agent] answer:\n');
  console.log(answer);
}

void main().catch((error) => {
  console.error('[agent] fatal error:', error);
  process.exit(1);
});
