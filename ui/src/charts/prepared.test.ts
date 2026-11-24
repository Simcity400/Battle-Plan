import test from 'node:test';
import assert from 'node:assert/strict';
import { overlapsSpans, spanRanges } from './prepared.ts';



test('marked spans clip to the half-open view and stop after the playhead hour', () => {
  const spans = [{ firstHour: 0, lastHour: 9 }, { firstHour: 20, lastHour: 29 }, { firstHour: 40, lastHour: 49 }];
  assert.deepEqual(spanRanges(spans, 5, 45), [[5, 10], [20, 30], [40, 45]]);
  assert.deepEqual(spanRanges(spans, 5, 45, 24), [[5, 10], [20, 25]]);
  assert.deepEqual(spanRanges(spans, 10, 20), []);
  assert.deepEqual(spanRanges(undefined, 0, 100), []);
  assert.equal(overlapsSpans(spans, 10, 19), false);
  assert.equal(overlapsSpans(spans, 10, 20), true);
  assert.equal(overlapsSpans(undefined, 0, 100), false);
});



