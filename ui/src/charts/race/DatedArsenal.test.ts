import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('a moving race rejects a worker head beyond the page clock while a pinned race may lead it', () => {
  const source = readFileSync(new URL('./DatedArsenal.tsx', import.meta.url), 'utf8');
  assert.match(source, /latest\.current\.model\.fixed === undefined && m\.head !== undefined && Math\.floor\(m\.head\)/,
    'the worker-head guard is limited to moving races');
});
