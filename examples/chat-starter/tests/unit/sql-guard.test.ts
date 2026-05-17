import { test } from "node:test";
import assert from "node:assert/strict";
import { guardSelect, SQL_GUARD } from "../../src/agent/sql-guard.js";

test("accepts a simple SELECT against an allowed namespace", () => {
  const r = guardSelect("SELECT count(*) FROM chat.conversations");
  assert.equal(r.ok, true);
  assert.match(r.sql!, /LIMIT 200$/);
});

test("preserves an explicit LIMIT instead of overriding", () => {
  const r = guardSelect("SELECT id FROM chat.messages LIMIT 5");
  assert.equal(r.ok, true);
  assert.equal(r.sql, "SELECT id FROM chat.messages LIMIT 5");
});

test("accepts SELECTs against chat.docs (RAG knowledge base)", () => {
  const r = guardSelect("SELECT id, title FROM chat.docs ORDER BY created_at DESC LIMIT 10");
  assert.equal(r.ok, true);
});

test("accepts JOIN across allowed tables", () => {
  const r = guardSelect(
    "SELECT m.id FROM chat.messages m JOIN chat.conversations c ON m.conversation_id = c.id",
  );
  assert.equal(r.ok, true);
});

test("strips trailing semicolon and still accepts", () => {
  const r = guardSelect("SELECT id FROM chat.tasks;");
  assert.equal(r.ok, true);
  assert.equal(r.sql, "SELECT id FROM chat.tasks LIMIT 200");
});

test("rejects DDL (DROP)", () => {
  const r = guardSelect("DROP TABLE chat.messages");
  assert.equal(r.ok, false);
  assert.match(r.reason!, /Only SELECT/);
});

test("rejects DML (DELETE)", () => {
  const r = guardSelect("DELETE FROM chat.messages WHERE 1=1");
  assert.equal(r.ok, false);
  assert.match(r.reason!, /Only SELECT/);
});

test("rejects DML (UPDATE)", () => {
  const r = guardSelect("UPDATE chat.messages SET body = '' WHERE 1=1");
  assert.equal(r.ok, false);
  assert.match(r.reason!, /Only SELECT/);
});

test("rejects DML (INSERT)", () => {
  const r = guardSelect("INSERT INTO chat.messages (id) VALUES ('x')");
  assert.equal(r.ok, false);
});

test("rejects chained statements via semicolon", () => {
  const r = guardSelect("SELECT 1 FROM chat.tasks; DROP TABLE chat.messages");
  assert.equal(r.ok, false);
  assert.match(r.reason!, /single statement/);
});

test("rejects line comments (-- ...)", () => {
  const r = guardSelect("SELECT * FROM chat.tasks -- DROP TABLE x");
  assert.equal(r.ok, false);
  assert.match(r.reason!, /Comments/);
});

test("rejects block comments (/* ... */)", () => {
  const r = guardSelect("SELECT * /* sneaky */ FROM chat.tasks");
  assert.equal(r.ok, false);
});

test("rejects queries that don't reference an allowed namespace", () => {
  const r = guardSelect("SELECT * FROM system.users");
  assert.equal(r.ok, false);
  assert.match(r.reason!, /chat/);
});

test("rejects bare-table queries (no namespace prefix)", () => {
  const r = guardSelect("SELECT * FROM messages");
  assert.equal(r.ok, false);
});

test("rejects empty input", () => {
  assert.equal(guardSelect("").ok, false);
  assert.equal(guardSelect("   \n  ").ok, false);
});

test("ACCEPTS double-hyphen INSIDE a string literal (not a comment)", () => {
  const r = guardSelect("SELECT id FROM chat.messages WHERE body LIKE '%--%'");
  assert.equal(r.ok, true);
});

test("ACCEPTS semicolon INSIDE a string literal (not a statement chain)", () => {
  const r = guardSelect("SELECT id FROM chat.messages WHERE body = 'a;b'");
  assert.equal(r.ok, true);
});

test("ACCEPTS escaped quotes (it''s) inside literals without confusing the scanner", () => {
  const r = guardSelect("SELECT id FROM chat.messages WHERE body = 'it''s; tricky'");
  assert.equal(r.ok, true);
});

test("still REJECTS real comment outside any string", () => {
  const r = guardSelect("SELECT id FROM chat.messages -- DROP TABLE x");
  assert.equal(r.ok, false);
  assert.match(r.reason!, /Comments/);
});

test("still REJECTS real semicolon chain outside any string", () => {
  const r = guardSelect("SELECT 1 FROM chat.tasks; DROP TABLE chat.messages");
  assert.equal(r.ok, false);
  assert.match(r.reason!, /single statement/);
});

test("rejects oversized SQL", () => {
  const huge = "SELECT * FROM chat.messages WHERE id IN (" + "'x',".repeat(2000) + "'y')";
  const r = guardSelect(huge);
  assert.equal(r.ok, false);
  assert.match(r.reason!, /exceeds/);
});

test("SQL_GUARD constants expose the policy", () => {
  assert.equal(SQL_GUARD.defaultLimit, 200);
  assert.deepEqual([...SQL_GUARD.allowedNamespaces], ["chat"]);
});
