#!/usr/bin/env node
import 'dotenv/config';
import { and, asc, eq } from 'drizzle-orm';
import { createDb, createKalamClient, resolveKalamConnection, type KalamDb } from './client.js';
import { downloadFileText } from './file-store.js';
import { stripFrontmatter } from './frontmatter.js';
import { context_files } from './schema.generated.js';

type ContextRow = {
  path: string;
  sha256: string;
};

async function loadCanonicalFiles(db: KalamDb): Promise<ContextRow[]> {
  return db
    .select({ path: context_files.path, sha256: context_files.sha256 })
    .from(context_files)
    .where(and(eq(context_files.deleted, false), eq(context_files.is_conflict, false)))
    .orderBy(asc(context_files.path));
}

async function buildAgentContext(accessToken: string): Promise<string> {
  const connection = resolveKalamConnection();
  const client = createKalamClient(connection);
  await client.initialize();
  const db = createDb(client);
  const login = await client.login();
  const token = accessToken || login.access_token;

  const rows = await loadCanonicalFiles(db);
  const chunks: string[] = [];

  for (const row of rows) {
    const text = await downloadFileText(db, connection.url, row.path, token);
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
