import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { execSync } from 'child_process';
import { readFileSync, unlinkSync, existsSync } from 'fs';
import { join } from 'path';
import { requirePassword, createTestClient, URL, PASS } from './helpers.mjs';

requirePassword();

const cliPath = join(import.meta.dirname, '..', 'dist', 'cli.js');
const outFile = join(import.meta.dirname, '..', 'test-output-schema.ts');
let client;

before(async () => {
  client = createTestClient();
  await client.initialize();
  await client.query('CREATE NAMESPACE IF NOT EXISTS test_cli_gen');
  await client.query('DROP TABLE IF EXISTS test_cli_gen.cli_options');
  await client.query(`
    CREATE TABLE test_cli_gen.cli_options (
      id BIGINT PRIMARY KEY DEFAULT SNOWFLAKE_ID(),
      name TEXT NOT NULL,
      created_at TIMESTAMP DEFAULT NOW()
    )
  `);
});

after(async () => {
  if (existsSync(outFile)) unlinkSync(outFile);
  await client?.query('DROP TABLE IF EXISTS test_cli_gen.cli_options').catch(() => {});
  await client?.query('DROP NAMESPACE IF EXISTS test_cli_gen').catch(() => {});
  await client?.disconnect();
});

describe('CLI', () => {
  it('generates a schema file', () => {
    execSync(`node ${cliPath} --url ${URL} --password ${PASS} --out ${outFile} --include-system`);
    assert.ok(existsSync(outFile));
    const content = readFileSync(outFile, 'utf-8');
    assert.ok(content.includes('kTable'));
    assert.ok(content.includes('system_users'));
  });

  it('excludes system tables by default', () => {
    execSync(`node ${cliPath} --url ${URL} --password ${PASS} --out ${outFile}`);
    const content = readFileSync(outFile, 'utf-8');
    assert.ok(!content.includes('system_users'));
    assert.ok(!content.includes('dba_'));
  });

  it('exits with error when password is missing', () => {
    assert.throws(
      () => execSync(`node ${cliPath} --url ${URL}`, {
        env: { ...process.env, KALAMDB_PASSWORD: '' },
        stdio: 'pipe',
      }),
    );
  });

  it('reads password from KALAMDB_PASSWORD env var', () => {
    execSync(`node ${cliPath} --url ${URL} --out ${outFile} --include-system`, {
      env: { ...process.env, KALAMDB_PASSWORD: PASS },
    });
    const content = readFileSync(outFile, 'utf-8');
    assert.ok(content.includes('kTable'));
    assert.ok(content.includes('system_users'));
  });

  it('honors namespace, system column, bigint, and type alias options', () => {
    execSync([
      `node ${cliPath}`,
      `--url ${URL}`,
      `--password ${PASS}`,
      `--out ${outFile}`,
      '--namespace test_cli_gen',
      '--include-system-columns all',
      '--bigint-mode bigint',
      '--no-type-aliases',
    ].join(' '));

    const content = readFileSync(outFile, 'utf-8');
    assert.ok(content.includes('export const test_cli_gen_cli_options = kTable'));
    assert.ok(content.includes('"test_cli_gen.cli_options"'));
    assert.ok(!content.includes('export const cli_options = kTable'));
    assert.ok(!content.includes('system_users'));
    assert.ok(content.includes('...kSystemColumns(["_seq","_deleted","_commit_seq"] as const),'));
    assert.ok(content.includes('bigint("id", { mode: "bigint" })'));
    assert.ok(!content.includes('$inferSelect'));
    assert.ok(!content.includes('$inferInsert'));
  });

  it('accepts repeated and comma-separated namespaces', () => {
    execSync([
      `node ${cliPath}`,
      `--url ${URL}`,
      `--password ${PASS}`,
      `--out ${outFile}`,
      '--namespace system,dba',
      '--namespace test_cli_gen',
      '--include-system-columns _seq',
    ].join(' '));

    const content = readFileSync(outFile, 'utf-8');
    assert.ok(content.includes('system_users'));
    assert.ok(content.includes('dba_'));
    assert.ok(content.includes('test_cli_gen_'));
    assert.ok(content.includes('...kSystemColumns(["_seq"] as const),'));
  });

  it('rejects unsupported bigint mode before writing a schema', () => {
    assert.throws(
      () => execSync(`node ${cliPath} --url ${URL} --password ${PASS} --out ${outFile} --bigint-mode unsafe`, { stdio: 'pipe' }),
      /Unsupported --bigint-mode/,
    );
  });

  it('rejects unsupported system columns before writing a schema', () => {
    assert.throws(
      () => execSync(`node ${cliPath} --url ${URL} --password ${PASS} --out ${outFile} --include-system-columns _seq,_tenant`, { stdio: 'pipe' }),
      /Unsupported system column/,
    );
  });
});
