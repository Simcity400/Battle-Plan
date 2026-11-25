import assert from 'node:assert/strict';
import test from 'node:test';
import { IntervalIndex, intervalIndexOf } from './intervalIndex.ts';
import { clipToSpans, extentOfAll, lastSampleHour, lineEndHour, runAt } from './series.ts';
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

test('the value range of several lines spans them all', () => {
  const a: Run[] = [{ firstHour: 0, lastHour: 9, value: 3 }];
  const b: Run[] = [{ firstHour: 5, lastHour: 20, value: -2 }];
  assert.deepEqual(extentOfAll([a, b], 0, 20), [-2, 3]);
  assert.deepEqual(extentOfAll([a, b], 12, 20), [-2, -2]);
  assert.equal(extentOfAll([a, b], 30, 40), null);
});

test('an 80,000-item interval index answers a lookup without scanning it', () => {
  const items: (Span & { id: number })[] = [];
  for (let index = 0; items.length < 80000; index++) {
    const first = 1000 + index * 7 + (index % 5);
    items.push({ id: index, firstHour: first, lastHour: first + 20 + (index % 40) });
  }
  for (let index = 0; index < 20; index++) items.push({ id: -index - 1, firstHour: 900 + index, lastHour: 400000 + index });
  const index = new IntervalIndex(items);
  for (const hour of [999, 1000, 1500, 123456, 400010, 560000, 700000]) {
    index.comparisons = 0;
    const found = index.at(hour);
    assert.ok(index.comparisons < 80, `${index.comparisons} comparisons at ${hour}`);
    const expected = items.filter(item => item.firstHour <= hour && hour <= item.lastHour);
    assert.deepEqual(new Set(found), new Set(expected), `hour ${hour}`);
    assert.equal(found.length, expected.length);
  }
  assert.deepEqual(new IntervalIndex([]).at(5), []);
});

test('an interval index is built once per owner, and answers at its items’ edges', () => {
  const owner = { items: [{ firstHour: 100, lastHour: 123 }, { firstHour: 120, lastHour: 130 }] };
  let builds = 0;
  const index = () => intervalIndexOf(owner, () => { builds++; return owner.items; });
  assert.equal(index(), index());
  assert.equal(builds, 1);
  assert.deepEqual(index().at(99), []);
  assert.deepEqual(index().at(100), [owner.items[0]]);
  assert.equal(index().at(123).length, 2);
  assert.deepEqual(index().at(124), [owner.items[1]]);
  assert.deepEqual(index().at(131), []);
});

test('a line ending early is labelled at its own last sample', () => {
  const run = (firstHour: number, lastHour: number) => ({ firstHour, lastHour, value: 50, breakBefore: false });
  const runs = [run(0, 99), run(150, 200)];
  assert.equal(lastSampleHour(runs, 300), 200);
  assert.equal(lastSampleHour(runs, 120), 99);
  assert.equal(lastSampleHour(runs, 160), 160);
  assert.equal(lastSampleHour([], 10), null);
  assert.equal(lastSampleHour(runs, -5), null);
});

test('end labels sit where a line is last drawn in view, even when the playhead is in a gap after it', () => {
  const reads = [{ firstHour: 1017, lastHour: 1040 }, { firstHour: 1100, lastHour: 1120 }];
  assert.equal(lineEndHour(reads, 1000, 1060), 1040);
  assert.equal(lineEndHour(reads, 1000, 1110.6), 1110);
  assert.equal(lineEndHour(reads, 1050, 1060), null);
  assert.equal(lineEndHour(reads, 1000, 1010), null);
  assert.equal(lineEndHour([], 1000, 1060), null);
});
