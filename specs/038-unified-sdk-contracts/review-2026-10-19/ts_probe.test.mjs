import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
const repo = process.env.KALAM_REPO_ROOT ?? process.cwd();
const require = createRequire(resolve(repo, 'link/sdks/typescript/react/package.json'));
const { desc } = require('drizzle-orm');
const { integer, text } = require('drizzle-orm/pg-core');
const { compileLiveTableDescriptor, kTable, liveTable } = await import(pathToFileURL(resolve(repo, 'link/sdks/typescript/orm/dist/index.js')).href);
const { LiveQueryController, createRawSqlLiveDescriptor, createLiveQueryDescriptor } = await import(pathToFileURL(resolve(repo, 'link/sdks/typescript/client/dist/src/index.js')).href);

const messages = kTable('chat.messages', {
  id: text('id').primaryKey(),
  createdAt: integer('created_at').notNull(),
});
const wireRows = [{id:'a',created_at:1},{id:'b',created_at:3},{id:'c',created_at:2}];

test('ORM descriptor orders mapped camelCase rows', async () => {
  const descriptor = compileLiveTableDescriptor(messages, {orderBy:desc(messages.createdAt)});
  const fake = { async live(sql, callback, options) {
    callback(wireRows.map(options.mapRow));
    return async () => {};
  }};
  const controller = new LiveQueryController(fake, descriptor);
  await controller.start();
  assert.deepEqual(controller.getSnapshot().rows.map(r=>r.createdAt),[3,2,1]);
});

test('direct ORM liveTable honors orderBy', async () => {
  let rows;
  const fake = { async connect(){}, async live(sql,callback,options) {
    callback(wireRows.map(options.mapRow)); return async()=>{};
  }};
  await liveTable(fake, messages, value=>{rows=value}, {orderBy:desc(messages.createdAt)});
  assert.deepEqual(rows.map(r=>r.createdAt),[3,2,1]);
});

test('raw live SQL preserves whitespace inside literal values', () => {
  const descriptor = createRawSqlLiveDescriptor("SELECT * FROM chat.messages WHERE body = 'a  b'");
  assert.ok(descriptor.subscriptionSql.includes("'a  b'"));
});

test('late transport errors cannot revive disposed controller', async () => {
  let options;
  const fake = {async live(sql,cb,opts){options=opts;return async()=>{};}};
  const controller = new LiveQueryController(fake, createLiveQueryDescriptor({mode:'sql',sourceSql:'SELECT * FROM chat.messages'}));
  await controller.start();
  await controller.dispose();
  options.onError({message:'late error'});
  assert.equal(controller.getSnapshot().status,'disposed');
});
