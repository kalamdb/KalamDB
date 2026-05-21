/**
 * Unit tests for KalamCellValue, wrapRowMap,
 * KalamRow.cell(), KalamRow.typedData, and query helper deduplication.
 *
 * Run with: node --test tests/cell-value.test.mjs
 */

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { describe, it } from 'node:test';
import initWasm from '../dist/wasm/kalam_client.js';

import {
  KalamCellValue,
  SeqId,
  wrapRowMap,
  KalamRow,
} from '../dist/src/index.js';

await initWasm({
  module_or_path: await readFile(new URL('../dist/wasm/kalam_client_bg.wasm', import.meta.url)),
});

/* ================================================================== */
/*  KalamCellValue — factory & type guards                             */
/* ================================================================== */

describe('KalamCellValue.from()', () => {
  it('wraps null', () => {
    const v = KalamCellValue.from(null);
    assert.equal(v.isNull(), true);
    assert.equal(v.toJson(), null);
  });

  it('wraps undefined as null', () => {
    const v = KalamCellValue.from(undefined);
    assert.equal(v.isNull(), true);
    assert.equal(v.toJson(), null);
  });

  it('wraps string', () => {
    const v = KalamCellValue.from('hello');
    assert.equal(v.isString(), true);
    assert.equal(v.isNull(), false);
    assert.equal(v.toJson(), 'hello');
  });

  it('wraps number', () => {
    const v = KalamCellValue.from(42);
    assert.equal(v.isNumber(), true);
    assert.equal(v.toJson(), 42);
  });

  it('wraps boolean', () => {
    const v = KalamCellValue.from(true);
    assert.equal(v.isBool(), true);
  });

  it('wraps object', () => {
    const v = KalamCellValue.from({ key: 'val' });
    assert.equal(v.isObject(), true);
  });

  it('wraps array', () => {
    const v = KalamCellValue.from([1, 2, 3]);
    assert.equal(v.isArray(), true);
  });
});

/* ================================================================== */
/*  KalamCellValue — typed accessors                                   */
/* ================================================================== */

describe('KalamCellValue.asString()', () => {
  it('returns string as-is', () => {
    assert.equal(KalamCellValue.from('hello').asString(), 'hello');
  });
  it('converts number to string', () => {
    assert.equal(KalamCellValue.from(42).asString(), '42');
  });
  it('converts boolean to string', () => {
    assert.equal(KalamCellValue.from(true).asString(), 'true');
    assert.equal(KalamCellValue.from(false).asString(), 'false');
  });
  it('handles Utf8 envelope', () => {
    assert.equal(KalamCellValue.from({ Utf8: 'wrapped' }).asString(), 'wrapped');
  });
  it('handles String envelope', () => {
    assert.equal(KalamCellValue.from({ String: 'wrapped' }).asString(), 'wrapped');
  });
  it('returns null for SQL NULL', () => {
    assert.equal(KalamCellValue.from(null).asString(), null);
  });
});

describe('KalamCellValue.asInt()', () => {
  it('returns integer', () => {
    assert.equal(KalamCellValue.from(42).asInt(), 42);
  });
  it('truncates float', () => {
    assert.equal(KalamCellValue.from(3.9).asInt(), 3);
  });
  it('parses string integer', () => {
    assert.equal(KalamCellValue.from('99').asInt(), 99);
  });
  it('converts boolean', () => {
    assert.equal(KalamCellValue.from(true).asInt(), 1);
    assert.equal(KalamCellValue.from(false).asInt(), 0);
  });
  it('returns null for non-numeric string', () => {
    assert.equal(KalamCellValue.from('abc').asInt(), null);
  });
  it('returns null for null', () => {
    assert.equal(KalamCellValue.from(null).asInt(), null);
  });
});

describe('KalamCellValue.asBigInt()', () => {
  it('converts number', () => {
    assert.equal(KalamCellValue.from(42).asBigInt(), 42n);
  });
  it('parses string bigint', () => {
    assert.equal(KalamCellValue.from('9007199254740993').asBigInt(), 9007199254740993n);
  });
  it('returns null for non-numeric string', () => {
    assert.equal(KalamCellValue.from('abc').asBigInt(), null);
  });
});

describe('KalamCellValue.asFloat()', () => {
  it('returns float', () => {
    assert.equal(KalamCellValue.from(3.14).asFloat(), 3.14);
  });
  it('converts integer to float', () => {
    assert.equal(KalamCellValue.from(42).asFloat(), 42);
  });
  it('parses string float', () => {
    assert.equal(KalamCellValue.from('3.14').asFloat(), 3.14);
  });
  it('converts boolean', () => {
    assert.equal(KalamCellValue.from(true).asFloat(), 1.0);
  });
  it('returns null for NaN string', () => {
    assert.equal(KalamCellValue.from('not-a-number').asFloat(), null);
  });
});

describe('KalamCellValue.asBool()', () => {
  it('returns boolean', () => {
    assert.equal(KalamCellValue.from(true).asBool(), true);
    assert.equal(KalamCellValue.from(false).asBool(), false);
  });
  it('converts number', () => {
    assert.equal(KalamCellValue.from(1).asBool(), true);
    assert.equal(KalamCellValue.from(0).asBool(), false);
  });
  it('handles string true/false', () => {
    assert.equal(KalamCellValue.from('true').asBool(), true);
    assert.equal(KalamCellValue.from('false').asBool(), false);
    assert.equal(KalamCellValue.from('1').asBool(), true);
    assert.equal(KalamCellValue.from('0').asBool(), false);
  });
  it('returns null for unrecognized string', () => {
    assert.equal(KalamCellValue.from('maybe').asBool(), null);
  });
});

describe('KalamCellValue datatype-specific accessors', () => {
  it('parses UUID values', () => {
    assert.equal(
      KalamCellValue.from('A0EBC999-9C0B-4EF8-BB6D-6BB9BD380A11').asUuid(),
      'a0ebc999-9c0b-4ef8-bb6d-6bb9bd380a11',
    );
    assert.equal(KalamCellValue.from('not-a-uuid').asUuid(), null);
  });

  it('returns decimals as exact strings', () => {
    assert.equal(KalamCellValue.from('1234.5600').asDecimal(), '1234.5600');
    assert.equal(KalamCellValue.from(42.5).asDecimal(), '42.5');
    assert.equal(KalamCellValue.from('nan-ish').asDecimal(), null);
  });

  it('parses BYTES arrays and base64 strings', () => {
    const rawBytes = new Uint8Array([7, 8, 9]);
    assert.deepEqual(Array.from(KalamCellValue.from([1, 2, 255]).asBytes()), [1, 2, 255]);
    assert.deepEqual(Array.from(KalamCellValue.from('AQID').asBytes()), [1, 2, 3]);
    assert.equal(KalamCellValue.from(rawBytes).asBytes(), rawBytes);
    assert.equal(KalamCellValue.from([1, 999]).asBytes(), null);
  });

  it('parses EMBEDDING vectors', () => {
    assert.deepEqual(KalamCellValue.from([0.1, 0.2, 0.3]).asEmbedding(), [0.1, 0.2, 0.3]);
    assert.deepEqual(KalamCellValue.from('[1,2,3]').asEmbedding(), [1, 2, 3]);
    assert.equal(KalamCellValue.from(['bad']).asEmbedding(), null);
  });

  it('parses DATE day offsets and TIME microseconds', () => {
    assert.equal(KalamCellValue.from(20573).asDateOnly().toISOString(), '2026-04-30T00:00:00.000Z');
    assert.equal(KalamCellValue.from('2026-04-30').asDateOnly().toISOString(), '2026-04-30T00:00:00.000Z');
    assert.equal(KalamCellValue.from(45_296_123_456).asTimeString(), '12:34:56.123456');
    assert.equal(KalamCellValue.from('12:34:56').asTimeString(), '12:34:56');
  });
});

describe('KalamCellValue.asDate()', () => {
  it('converts unix millis', () => {
    const d = KalamCellValue.from(1704067200000).asDate();
    assert.ok(d instanceof Date);
    assert.equal(d.getTime(), 1704067200000);
  });
  it('parses ISO 8601 string', () => {
    const d = KalamCellValue.from('2024-01-01T00:00:00Z').asDate();
    assert.ok(d instanceof Date);
  });
  it('parses numeric timestamp string', () => {
    const d = KalamCellValue.from('1704067200000').asDate();
    assert.ok(d instanceof Date);
  });
  it('normalizes microsecond timestamp strings', () => {
    const d = KalamCellValue.from('1772964000000000').asDate();
    assert.ok(d instanceof Date);
    assert.equal(d.toISOString(), '2026-03-08T10:00:00.000Z');
  });
  it('returns null for bad date', () => {
    assert.equal(KalamCellValue.from('not-a-date').asDate(), null);
  });
  it('returns null for null', () => {
    assert.equal(KalamCellValue.from(null).asDate(), null);
  });
});

describe('KalamCellValue.asObject()', () => {
  it('returns object', () => {
    const obj = { key: 'val' };
    const v = KalamCellValue.from(obj).asObject();
    assert.deepEqual(v, { key: 'val' });
  });
  it('returns null for non-object', () => {
    assert.equal(KalamCellValue.from('str').asObject(), null);
  });
  it('returns null for array (not considered object)', () => {
    assert.equal(KalamCellValue.from([1, 2]).asObject(), null);
  });
});

describe('KalamCellValue.asArray()', () => {
  it('returns array', () => {
    const v = KalamCellValue.from([1, 2, 3]).asArray();
    assert.deepEqual(v, [1, 2, 3]);
  });
  it('returns null for non-array', () => {
    assert.equal(KalamCellValue.from('str').asArray(), null);
  });
});

describe('KalamCellValue.toString()', () => {
  it('NULL for null', () => {
    assert.equal(KalamCellValue.from(null).toString(), 'NULL');
  });
  it('string as-is', () => {
    assert.equal(KalamCellValue.from('hello').toString(), 'hello');
  });
  it('number stringified', () => {
    assert.equal(KalamCellValue.from(42).toString(), '42');
  });
  it('object as JSON', () => {
    assert.equal(KalamCellValue.from({ a: 1 }).toString(), '{"a":1}');
  });
});

/* ================================================================== */
/*  Utility functions                                                  */
/* ================================================================== */

describe('wrapRowMap()', () => {
  it('wraps each value in KalamCellValue', () => {
    const raw = { id: 1, name: 'Alice', active: true };
    const typed = wrapRowMap(raw);
    assert.equal(typed.id.asInt(), 1);
    assert.equal(typed.name.asString(), 'Alice');
    assert.equal(typed.active.asBool(), true);
  });

  it('handles null values', () => {
    const typed = wrapRowMap({ col: null });
    assert.equal(typed.col.isNull(), true);
  });
});

/* ================================================================== */
/*  KalamRow — cell() and typedData                                    */
/* ================================================================== */

describe('KalamRow.cell()', () => {
  it('returns KalamCellValue for each column', () => {
    const ctx = { baseUrl: 'http://localhost:2900', namespace: 'default', table: 'users' };
    const row = new KalamRow({ id: 1, name: 'Alice', score: 95.5 }, ctx);

    assert.equal(row.cell('id').asInt(), 1);
    assert.equal(row.cell('name').asString(), 'Alice');
    assert.equal(row.cell('score').asFloat(), 95.5);
  });

  it('handles null values', () => {
    const ctx = { baseUrl: '', namespace: '', table: '' };
    const row = new KalamRow({ x: null }, ctx);
    assert.equal(row.cell('x').isNull(), true);
  });
});

describe('KalamRow.typedData', () => {
  it('returns all cells as KalamCellValue (RowData)', () => {
    const ctx = { baseUrl: 'http://localhost:2900', namespace: 'default', table: 'users' };
    const row = new KalamRow({ id: 1, name: 'Alice', active: true }, ctx);

    const td = row.typedData;
    assert.equal(td['id'].asInt(), 1);
    assert.equal(td['name'].asString(), 'Alice');
    assert.equal(td['active'].asBool(), true);
  });

  it('caches the result on repeated access', () => {
    const ctx = { baseUrl: '', namespace: '', table: '' };
    const row = new KalamRow({ a: 1 }, ctx);
    const td1 = row.typedData;
    const td2 = row.typedData;
    assert.equal(td1, td2); // same object reference
  });
});

describe('KalamRow — unified query & subscribe access pattern', () => {
  it('cell() works the same whether data is raw or pre-wrapped', () => {
    const ctx = { baseUrl: 'http://localhost:2900', namespace: 'default', table: 'users' };

    // Simulating query path (raw values)
    const queryRow = new KalamRow({ name: 'Alice', age: 30 }, ctx);
    assert.equal(queryRow.cell('name').asString(), 'Alice');
    assert.equal(queryRow.cell('age').asInt(), 30);

    // Simulating subscribe path (also raw values from server)
    const subRow = new KalamRow({ name: 'Bob', age: 25 }, ctx);
    assert.equal(subRow.cell('name').asString(), 'Bob');
    assert.equal(subRow.cell('age').asInt(), 25);

    // Same API pattern for both paths
  });
});

describe('KalamRow.file()', () => {
  it('returns a context-bound file ref with correct download paths', () => {
    const ctx = { baseUrl: 'http://localhost:2900', namespace: 'default', table: 'users' };
    const row = new KalamRow({
      avatar: {
        id: '12345',
        sub: 'f0001',
        name: 'photo.jpg',
        size: 1024,
        mime: 'image/jpeg',
        sha256: 'abc123',
      },
    }, ctx);

    const ref = row.file('avatar');
    assert.ok(ref !== null);
    assert.equal(ref.downloadUrl(), 'http://localhost:2900/v1/files/default/users/f0001/12345-photo.jpg');
    assert.equal(ref.relativeUrl(), '/v1/files/default/users/f0001/12345-photo.jpg');
  });
});

/* ================================================================== */
/*  FILE column support via asFile()                                   */
/* ================================================================== */

describe('KalamCellValue.asFile()', () => {
  it('parses valid FILE column JSON', () => {
    const fileData = {
      id: '12345',
      sub: 'f0001',
      name: 'photo.jpg',
      size: 1024,
      mime: 'image/jpeg',
      sha256: 'abc123',
    };
    const cell = KalamCellValue.from(fileData);
    const ref = cell.asFile();
    assert.ok(ref !== null);
    assert.equal(ref.name, 'photo.jpg');
    assert.equal(ref.mime, 'image/jpeg');
    assert.equal(ref.size, 1024);
  });

  it('returns null for non-file values', () => {
    assert.equal(KalamCellValue.from('just a string').asFile(), null);
    assert.equal(KalamCellValue.from(42).asFile(), null);
  });

  it('returns null for null', () => {
    assert.equal(KalamCellValue.from(null).asFile(), null);
  });
});

describe('KalamCellValue.asFileUrl()', () => {
  it('builds download URL from FILE reference', () => {
    const fileData = {
      id: '12345',
      sub: 'f0001',
      name: 'photo.jpg',
      size: 1024,
      mime: 'image/jpeg',
      sha256: 'abc123',
    };
    const url = KalamCellValue.from(fileData).asFileUrl('http://localhost:2900', 'default', 'users');
    assert.equal(url, 'http://localhost:2900/v1/files/default/users/f0001/12345-photo.jpg');
  });

  it('returns null for non-file values', () => {
    assert.equal(KalamCellValue.from('text').asFileUrl('http://x', 'ns', 't'), null);
  });
});

/* ================================================================== */
/*  Edge cases                                                         */
/* ================================================================== */

describe('edge cases', () => {
  it('KalamCellValue wraps nested objects correctly', () => {
    const v = KalamCellValue.from({ nested: { deep: true } });
    const obj = v.asObject();
    assert.deepEqual(obj, { nested: { deep: true } });
  });
});

/* ================================================================== */
/*  SeqId                                                              */
/* ================================================================== */

describe('SeqId', () => {
  it('creates from number', () => {
    const seq = SeqId.from(123456789);
    assert.equal(seq.toString(), '123456789');
    assert.equal(seq.toNumber(), 123456789);
  });

  it('creates from bigint', () => {
    const seq = SeqId.from(123456789n);
    assert.equal(seq.toBigInt(), 123456789n);
  });

  it('creates from string', () => {
    const seq = SeqId.from('123456789');
    assert.equal(seq.toNumber(), 123456789);
  });

  it('zero() returns value 0', () => {
    const seq = SeqId.zero();
    assert.equal(seq.toNumber(), 0);
    assert.equal(seq.toBigInt(), 0n);
  });

  it('extracts Snowflake fields', () => {
    const timestampOffset = 5000n;
    const workerId = 7n;
    const sequence = 21n;
    const id = (timestampOffset << 22n) | (workerId << 12n) | sequence;
    const seq = SeqId.from(id);

    assert.equal(seq.timestampMillis(), Number(SeqId.EPOCH + timestampOffset));
    assert.equal(seq.workerId(), 7);
    assert.equal(seq.sequence(), 21);
  });

  it('toDate returns a Date', () => {
    const seq = SeqId.from(1000n << 22n);
    const d = seq.toDate();
    assert.ok(d instanceof Date);
    assert.equal(d.getTime(), Number(SeqId.EPOCH + 1000n));
  });

  it('equals compares values', () => {
    assert.ok(SeqId.from(42).equals(SeqId.from(42n)));
    assert.ok(!SeqId.from(42).equals(SeqId.from(43)));
  });

  it('compareTo orders correctly', () => {
    const a = SeqId.from(10);
    const b = SeqId.from(20);
    assert.ok(a.compareTo(b) < 0);
    assert.ok(b.compareTo(a) > 0);
    assert.equal(a.compareTo(a), 0);
  });

  it('toJSON returns number', () => {
    const seq = SeqId.from(42);
    const json = JSON.stringify({ seq });
    assert.equal(json, '{"seq":42}');
  });

  it('throws on invalid input', () => {
    assert.throws(() => SeqId.from('abc'));
    assert.throws(() => SeqId.from(''));
    assert.throws(() => SeqId.from(NaN));
    assert.throws(() => SeqId.from(1.5));
  });
});

/* ================================================================== */
/*  KalamCellValue.asSeqId()                                           */
/* ================================================================== */

describe('KalamCellValue.asSeqId()', () => {
  it('converts number to SeqId', () => {
    const seq = KalamCellValue.from(42).asSeqId();
    assert.ok(seq !== null);
    assert.equal(seq.toNumber(), 42);
  });

  it('converts string to SeqId', () => {
    const seq = KalamCellValue.from('99').asSeqId();
    assert.ok(seq !== null);
    assert.equal(seq.toNumber(), 99);
  });

  it('returns null for non-numeric string', () => {
    assert.equal(KalamCellValue.from('abc').asSeqId(), null);
  });

  it('returns null for null', () => {
    assert.equal(KalamCellValue.from(null).asSeqId(), null);
  });
});

console.log('cell-value.test.mjs: all tests registered');
