import assert from 'node:assert/strict';
import test from 'node:test';
import { IntervalIndex, intervalIndexOf } from './intervalIndex.ts';
import { clipToSpans, extentOfAll, runAt, sameStretch } from './series.ts';
import type { Run, Span } from './series';

/** A product's run: its line breaks where `startsSegment` says. */
const product = (firstHour: number, lastHour: number, startsSegment = true) => ({ firstHour, lastHour, startsSegment, valueRaw: String(firstHour) });

test('any run type is found by hour', () => {
  const runs = [product(100, 123), product(124, 130, false)];
  assert.equal(runAt(runs, 99), null);
  assert.equal(runAt(runs, 110), runs[0]);
  assert.equal(runAt(runs, 130), runs[1]);
  assert.equal(runAt(runs, 131), null);
});

test('a value glides only within one unbroken stretch of runs', () => {
  const runs = [product(100, 110), product(111, 120, false), product(121, 130), product(140, 150)];
  assert.equal(sameStretch(runs, 105, 118), true);
  assert.equal(sameStretch(runs, 118, 105), true);
  assert.equal(sameStretch(runs, 105, 105), true);
  // A new country identity or a missing read starts a new stretch.
  assert.equal(sameStretch(runs, 118, 125), false);
  // A gap in the recording, or nothing recorded at one end.
  assert.equal(sameStretch(runs, 125, 145), false);
  assert.equal(sameStretch(runs, 135, 145), false);
  assert.equal(sameStretch([], 1, 1), false);
  // Plotting runs break with `breakBefore`.
  const plotted: Run[] = [{ firstHour: 0, lastHour: 9, value: 1 }, { firstHour: 10, lastHour: 19, value: 2, breakBefore: true }, { firstHour: 20, lastHour: 29, value: 3 }];
  assert.equal(sameStretch(plotted, 5, 15), false);
  assert.equal(sameStretch(plotted, 15, 25), true);
});

test('runs are cut to spans, keeping a line break only where a run starts', () => {
  const runs = [product(100, 123), product(124, 130, false)];
  const cut = clipToSpans(runs, [{ firstHour: 110, lastHour: 126 }]);
  assert.deepEqual(cut.map(run => [run.firstHour, run.lastHour, run.startsSegment, run.valueRaw]), [[110, 123, false, '100'], [124, 126, false, '124']]);
  assert.equal(runAt(cut, 109), null);
  assert.equal(runAt(cut, 127), null);
  assert.equal(clipToSpans(runs, [{ firstHour: 90, lastHour: 140 }])[0].startsSegment, true);
  assert.deepEqual(clipToSpans(runs, [{ firstHour: 131, lastHour: 140 }]), []);
  // Plotting runs cut to several spans (a country's group membership), in hour order.
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
  // Deals held for a few days and renewed over a long campaign: 80,000 of them.
  const items: (Span & { id: number })[] = [];
  for (let index = 0; items.length < 80000; index++) {
    const first = 1000 + index * 7 + (index % 5);
    items.push({ id: index, firstHour: first, lastHour: first + 20 + (index % 40) });
  }
  // A few long ones span much of the campaign, as real deals do.
  for (let index = 0; index < 20; index++) items.push({ id: -index - 1, firstHour: 900 + index, lastHour: 400000 + index });
  const index = new IntervalIndex(items);
  for (const hour of [999, 1000, 1500, 123456, 400010, 560000, 700000]) {
    index.comparisons = 0;
    const found = index.at(hour);
    // A binary search and one walk down the tree: a few dozen steps, not 80,000.
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
