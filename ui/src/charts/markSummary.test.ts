import assert from 'node:assert/strict';
import test from 'node:test';
import { markIndex, markClusterAt, clusterHour, markSummaryMatches } from './markSummary.ts';
import { plotFrame } from './frame.ts';

test('50-year dense marks retain every duplicate in exact pages while geometry follows width', () => {
  const events = Array.from({ length: 438_000 }, (_, hour) => ({ hour, glyph: '*', label: `Event ${hour}` }));
  events.splice(1000, 0, ...Array.from({ length: 5000 }, (_, index) => ({ hour: 1000, glyph: '!', label: `Duplicate ${index}` })));
  const index = markIndex(events);
  const summary = index.summary(0, 437_999, 360);
  assert.ok(summary.clusters.length <= 20);
  assert.equal(summary.clusters.reduce((sum, cluster) => sum + BigInt(cluster.count), 0n), BigInt(events.length));
  const duplicates = index.at(1000);
  assert.equal(duplicates.length, 5001);
  assert.equal(duplicates[0].label, 'Duplicate 0');
  assert.equal(duplicates[4999].label, 'Duplicate 4999');
  assert.equal(duplicates[5000].label, 'Event 1000');
  const cluster = summary.clusters[0];
  const first = Number(cluster.firstOrdinal), count = Number(cluster.count);
  assert.deepEqual(index.page(first, count).filter(event => event.hour === 1000), duplicates);
  const zoomed = index.summary(1000, 1000, 360);
  assert.equal(zoomed.clusters.length, 1); assert.equal(zoomed.clusters[0].count, '5001');
  assert.deepEqual(index.page(Number(zoomed.clusters[0].firstOrdinal), 5001), duplicates);
});

test('cluster hit returns its exact ordinal range and range date, with playback clipping', () => {
  const index = markIndex([{ hour: 2, glyph: 'A', label: 'First' }, { hour: 4, glyph: 'B', label: 'Second' },
    { hour: 90, glyph: 'C', label: 'Future' }]);
  const frame = plotFrame(28, 100, { left: 5, right: 5, top: 24, bottom: 10 }, [0, 100], [0, 1], false);
  const summary = index.summary(0, 10, 18);
  assert.deepEqual(summary.clusters[0], { firstHour: 2, lastHour: 4, firstOrdinal: '0', count: '2' });
  assert.equal(clusterHour(summary.clusters[0]), 3);
  assert.deepEqual(markClusterAt({ x: frame.toX(3), y: 16 }, frame, summary), summary.clusters[0]);
  assert.equal(markClusterAt({ x: frame.toX(3), y: 40 }, frame, summary), null);
  assert.equal(markSummaryMatches(summary, frame, 10), true);
  assert.equal(markSummaryMatches(summary, frame, 5), false);
  assert.equal(markSummaryMatches(summary, frame, null), false);
  assert.equal(index.summary(90, 90, 18).clusters[0].badge?.label, 'Future');
});
