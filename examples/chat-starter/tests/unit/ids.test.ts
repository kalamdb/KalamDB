import { test } from "node:test";
import assert from "node:assert/strict";
import { UUID_RE, uuidLit } from "../../src/agent/ids.js";

test("UUID_RE accepts canonical v4 UUIDs", () => {
  assert.ok(UUID_RE.test("4d9f52c9-8e1b-49d3-9c96-17f18ff90058"));
  assert.ok(UUID_RE.test("AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"));
});

test("UUID_RE rejects malformed shapes", () => {
  for (const bad of [
    "",
    "not-a-uuid",
    "4d9f52c9-8e1b-49d3-9c96",
    "4d9f52c9-8e1b-49d3-9c96-17f18ff9005",
    "4d9f52c9_8e1b_49d3_9c96_17f18ff90058",
    "4d9f52c9-8e1b-49d3-9c96-17f18ff90058'; DROP TABLE x;--",
    "' OR 1=1 --",
  ]) {
    assert.equal(UUID_RE.test(bad), false, `should reject: ${bad}`);
  }
});

test("uuidLit wraps a valid UUID as a SQL literal", () => {
  assert.equal(
    uuidLit("4d9f52c9-8e1b-49d3-9c96-17f18ff90058"),
    "'4d9f52c9-8e1b-49d3-9c96-17f18ff90058'",
  );
});

test("uuidLit throws on injection attempts", () => {
  assert.throws(() => uuidLit("' OR 1=1 --"));
  assert.throws(() => uuidLit("anything\\'; DROP TABLE x;--"));
  assert.throws(() => uuidLit(""));
});
