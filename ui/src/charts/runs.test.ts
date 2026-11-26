import assert from 'node:assert/strict';
import test from 'node:test';
import { clipToSpans, runAt } from './series.ts';
import type { Run, Span } from './series';

const product = (firstHour: number, lastHour: number, startsSegment = true) => ({ firstHour, lastHour, startsSegment, valueRaw: String(firstHour) });


test('run lookup keeps inclusive ends, gaps, and fractional and negative hours', () => {
  const runs = [product(-5, -2), product(0, 2), product(5, 5)];
  const cases: [number, number | null][] = [
    [-6, null], [-5, 0], [-3.5, 0], [-2, 0], [-1.5, null], [-1, null],
    [0, 1], [1.5, 1], [2, 1], [2.5, null], [3, null], [5, 2], [5.5, null],
  ];
  for (const [hour, index] of cases) assert.equal(runAt(runs, hour), index == null ? null : runs[index], `hour ${hour}`);
  assert.equal(runAt([], 0), null);
});

test('runs are cut to spans, keeping a line break only where a run starts', () => {
  const runs = [product(100, 123), product(124, 130, false)];
  const cut = clipToSpans(runs, [{ firstHour: 110, lastHour: 126 }]);
  assert.deepEqual(cut.map(run => [run.firstHour, run.lastHour, run.startsSegment, run.valueRaw]), [[110, 123, false, '100'], [124, 126, false, '124']]);
  assert.equal(runAt(cut, 109), null);
  assert.equal(runAt(cut, 127), null);
  assert.equal(clipToSpans(runs, [{ firstHour: 90, lastHour: 140 }])[0].startsSegment, true);
  assert.deepEqual(clipToSpans(runs, [{ firstHour: 131, lastHour: 140 }]), []);
  const plotted: Run[] = [{ firstHour: 0, lastHour: 20, value: 1, breakBefore: true }];
  const spans: Span[] = [{ firstHour: 15, lastHour: 30 }, { firstHour: 0, lastHour: 5 }];
  assert.deepEqual(clipToSpans(plotted, spans), [{ firstHour: 0, lastHour: 5, value: 1, breakBefore: true }, { firstHour: 15, lastHour: 20, value: 1 }]);
});
