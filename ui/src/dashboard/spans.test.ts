import assert from 'node:assert/strict';
import test from 'node:test';
import { validSpans } from './spans.ts';

test('recorded span validation accepts gaps and rejects malformed or overlapping hours', () => {
  assert.equal(validSpans([]), true);
  assert.equal(validSpans([{ firstHour: 1, lastHour: 2 }, { firstHour: 4, lastHour: 4 }]), true);
  for (const value of [null, {}, [null], [{ firstHour: 2, lastHour: 1 }], [{ firstHour: 1.5, lastHour: 2 }],
    [{ firstHour: 1, lastHour: 2 }, { firstHour: 2, lastHour: 3 }], [{ firstHour: 4, lastHour: 4 }, { firstHour: 1, lastHour: 2 }]]) {
    assert.equal(validSpans(value), false);
  }
});
