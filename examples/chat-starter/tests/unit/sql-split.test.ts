import { test } from "node:test";
import assert from "node:assert/strict";
import { splitStatements } from "../../scripts/sql-split.js";

test("splits trivial multi-statement", () => {
  const out = splitStatements("SELECT 1; SELECT 2; SELECT 3");
  assert.deepEqual(out, ["SELECT 1", "SELECT 2", "SELECT 3"]);
});

test("ignores semicolons inside single-quoted strings", () => {
  const out = splitStatements("INSERT INTO t VALUES ('a; b; c'); SELECT 1");
  assert.deepEqual(out, ["INSERT INTO t VALUES ('a; b; c')", "SELECT 1"]);
});

test("respects doubled-quote escapes inside string literals", () => {
  const out = splitStatements("INSERT INTO t VALUES ('it''s; tricky'); SELECT 2");
  assert.deepEqual(out, ["INSERT INTO t VALUES ('it''s; tricky')", "SELECT 2"]);
});

test("strips full-line comments", () => {
  const out = splitStatements(`-- drop everything
DROP TABLE x;
-- and again
SELECT 1`);
  assert.deepEqual(out, ["DROP TABLE x", "SELECT 1"]);
});

test("returns empty array for empty / comment-only input", () => {
  assert.deepEqual(splitStatements(""), []);
  assert.deepEqual(splitStatements("   "), []);
  assert.deepEqual(splitStatements("-- only a comment"), []);
});

test("handles trailing whitespace and missing final semicolon", () => {
  assert.deepEqual(splitStatements("SELECT 1"), ["SELECT 1"]);
  assert.deepEqual(splitStatements("SELECT 1;   "), ["SELECT 1"]);
});

test("preserves the full chat-starter schema shape", () => {
  // Real-world smoke: confirm we still split the actual schema into the
  // expected number of statements.
  const sql = `
DROP TOPIC chat.task_events;
DROP NAMESPACE IF EXISTS chat;
CREATE NAMESPACE chat;
CREATE TABLE chat.tasks (
  id TEXT PRIMARY KEY
) WITH (TYPE = 'USER');
CREATE TOPIC chat.task_events;
ALTER TOPIC chat.task_events ADD SOURCE chat.tasks ON INSERT;
`;
  const out = splitStatements(sql);
  assert.equal(out.length, 6);
  assert.match(out[3]!, /CREATE TABLE chat\.tasks/);
  assert.match(out[5]!, /ADD SOURCE chat\.tasks ON INSERT/);
});
