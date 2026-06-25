import test from 'node:test';
import assert from 'node:assert/strict';
import { parseFrontmatter, stripFrontmatter } from '../src/frontmatter.js';

test('parseFrontmatter reads yaml keys and list tags', () => {
  const parsed = parseFrontmatter(`---
title: Refund Policy
type: runbook
tags: [support, refunds]
---

# Body
`);

  assert.deepEqual(parsed, {
    title: 'Refund Policy',
    type: 'runbook',
    tags: ['support', 'refunds'],
  });
});

test('stripFrontmatter removes the header block', () => {
  const body = stripFrontmatter(`---
title: Demo
---

Hello OKF
`);

  assert.equal(body.trim(), 'Hello OKF');
});
