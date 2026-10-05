import assert from 'node:assert/strict';
import test from 'node:test';
import { isOrderedSpans, isSpan, spansCovered } from './series.ts';

test('span admission requires a non-reversed pair of safe integer hours', () => {
  assert.equal(isSpan({ firstHour: -10, lastHour: 0, extra: 'preserved' }), true);
  assert.equal(isSpan({ firstHour: 10, lastHour: 10 }), true);
  for (const value of [null, [], '10', {}, { firstHour: 10 }, { firstHour: 11, lastHour: 10 },
    { firstHour: 10.5, lastHour: 11 }, { firstHour: 10, lastHour: 11.5 }, { firstHour: '10', lastHour: 11 },
    { firstHour: NaN, lastHour: 11 }, { firstHour: 10, lastHour: Infinity }, { firstHour: 0, lastHour: Number.MAX_SAFE_INTEGER + 1 }]) {
    assert.equal(isSpan(value), false);
  }
});

test('ordered span admission preserves adjacency and gaps and refuses overlap or disorder', () => {
  const spans = [{ firstHour: 10, lastHour: 20 }, { firstHour: 21, lastHour: 30 }, { firstHour: 35, lastHour: 40 }];
  assert.equal(isOrderedSpans(spans), true);
  assert.deepEqual(spans, [{ firstHour: 10, lastHour: 20 }, { firstHour: 21, lastHour: 30 }, { firstHour: 35, lastHour: 40 }]);
  assert.equal(isOrderedSpans([]), true);
  assert.equal(isOrderedSpans([spans[0], { firstHour: 20, lastHour: 30 }]), false);
  assert.equal(isOrderedSpans([spans[2], spans[0]]), false);
  assert.equal(isOrderedSpans([{ firstHour: 20, lastHour: 10 }]), false);
  assert.equal(isOrderedSpans([{ firstHour: 10, lastHour: 20.5 }]), false);
  assert.equal(isOrderedSpans(new Array(1)), false);
  assert.equal(isOrderedSpans(null), false);
});

test('value histories cannot bridge a gap, including one inside a long compressed run', () => {
  const observed = [{ firstHour: 10, lastHour: 20 }, { firstHour: 25, lastHour: 40 }];
  assert.equal(spansCovered([{ firstHour: 12, lastHour: 20 }, { firstHour: 25, lastHour: 35 }], observed), true);
  assert.equal(spansCovered([{ firstHour: 12, lastHour: 35 }], observed), false);
  assert.equal(spansCovered([{ firstHour: 9, lastHour: 10 }], observed), false);
  assert.equal(spansCovered([{ firstHour: 40, lastHour: 41 }], observed), false);
  assert.equal(spansCovered([], []), true);
  assert.equal(spansCovered([{ firstHour: 1, lastHour: 2 }], []), false);
  assert.equal(spansCovered([{ firstHour: 10, lastHour: 40 }], [{ firstHour: 10, lastHour: 20 }, { firstHour: 21, lastHour: 40 }]), true);
});
