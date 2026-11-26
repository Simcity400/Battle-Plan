import assert from 'node:assert/strict';
import test from 'node:test';
import type { SeriesTileReply } from '../../generated/dashboard';
import type { MarkCluster } from '../../charts/markSummary';
import { playbackDomain } from '../../charts/view.ts';
import { TileCache } from './cache.ts';
import { PRESENT, TILE_BINS, binHoursOf } from './tile.ts';
import { TileView, cellBins, cellClusters } from './view.ts';
import type { TileLane, TileViewport } from './view.ts';

const LAST = 105_541;
const settle = () => new Promise<void>(resolve => setImmediate(resolve));
const base64 = (bytes: Uint8Array) => { let binary = ''; for (const byte of bytes) binary += String.fromCharCode(byte); return btoa(binary); };

const dropsAt = (tag: number, bin: number) => (bin * 7 + tag * 3) % 11 < 3 ? 1 + (bin + tag) % 3 : 0;

function marked(tags: readonly string[], binHours: number, index: number): SeriesTileReply {
  const first = index * TILE_BINS * binHours, last = first + TILE_BINS * binHours - 1;
  const count = tags.length * TILE_BINS, bytes = new Uint8Array(count * (1 + 16 + 5)), words = new DataView(bytes.buffer);
  bytes.fill(PRESENT, 0, count);
  for (let at = 0; at < count; at++) for (const [k, add] of [0, 7, -5, 11].entries()) words.setInt32(count + (at * 4 + k) * 4, (at * 37) % 200 - 100 + add, true);
  for (let at = 0; at < count; at++) bytes.set([0, binHours - 1, binHours - 1, Math.floor(binHours / 3), Math.floor(binHours / 2)].map(hour => Math.min(255, hour)), count * 17 + at * 5);
  const before = (tag: number) => { let sum = 0; for (let bin = 0; bin < index * TILE_BINS; bin++) sum += dropsAt(tag, bin); return sum; };
  return { branchId: '1', measure: 'politicalPower', decimals: 0, binHours, index, firstHour: first, immutable: last < LAST, filled: Math.min(last, LAST),
    rawValues: true, hourBytes: 1, bins: base64(bytes),
    lines: tags.map((tag, at) => ({ country: tag, missing: [], parts: [{ firstHour: first, lastHour: last, slot: at, countryId: at, segmentFirstHour: 0 }],
      marks: { before: String(before(at)), bins: Array.from({ length: TILE_BINS }, (_, bin) => [bin, dropsAt(at, index * TILE_BINS + bin)] as [number, number]).filter(([, count]) => count > 0) } })) };
}

function lane(tags: readonly string[]) {
  const asked: number[] = [];
  const out: TileLane & { asked: number[] } = { key: 'source|1|1|measure', tags, lastHour: LAST, marks: true, asked,
    ask: async (binHours, index, tags) => { asked.push(index); return marked(tags, binHours, index); } };
  return out;
}

function following(head: number, days: number, width = 600): TileViewport {
  const [from, end] = playbackDomain('window', [0, LAST], head, days * 24, 24);
  return { from: Math.floor(from), to: Math.floor(Math.min(end, head)), end: Math.floor(end), width };
}

const key = (cluster: MarkCluster) => `${cluster.firstHour}:${cluster.lastHour}:${cluster.count}:${cluster.firstOrdinal}`;

test('bins group into cells on the absolute grid: drops summed, the first drop ordinal kept, hours from the first such bin to the last', () => {
  const drops: [number, number, bigint][] = [[0, 2, 0n], [24, 1, 2n], [36, 4, 3n], [96, 1, 7n], [108, 2, 8n], [132, 1, 10n]];
  assert.deepEqual(cellClusters(drops, 12, 3), [
    { firstHour: 0, lastHour: 35, firstOrdinal: '0', count: '3', bins: [[11, 2], [35, 1]] },
    { firstHour: 36, lastHour: 47, firstOrdinal: '3', count: '4', bins: [[47, 4]] },
    { firstHour: 96, lastHour: 107, firstOrdinal: '7', count: '1', bins: [[107, 1]] },
    { firstHour: 108, lastHour: 143, firstOrdinal: '8', count: '3', bins: [[119, 2], [143, 1]] },
  ]);
});

test('the count of bins in a cell is the whole number nearest eighteen pixels, kept through an hour of jitter, changed by a zoom', () => {
  assert.equal(cellBins(8760, 600, 12), 22);
  assert.equal(cellBins(8761, 600, 12, 22), 22, 'a following view one hour longer keeps its cells');
  assert.equal(cellBins(8759, 600, 12, 22), 22);
  assert.equal(cellBins(4 * 8760, 600, 48), 22);
  assert.equal(cellBins(2 * 8760, 600, 24, 22), 22);
  assert.equal(cellBins(12_000, 600, 12, 22), 30, 'a zoom regroups');
  assert.equal(cellBins(168, 600, 1), 5, 'a week at 600 pixels: cells of five hours');
  assert.equal(cellBins(24, 600, 1), 1, 'a bin wider than eighteen pixels is its own cell');
});

test('a scroll keeps every past cluster, a zoom regroups, and the ordinals are those of each cluster first drop', async () => {
  const cache = new TileCache(), marks = lane(['GER', 'ITA']);
  const view = new TileView(cache, marks, () => {});
  view.setView(following(30_000, 365));
  await settle();
  const before = view.current!.get('GER')!.marks!;
  view.setView(following(30_336, 365));
  await settle();
  const after = view.current!.get('GER')!.marks!;
  const held = new Set(after.clusters.map(key));
  const shared = before.clusters.filter(cluster => cluster.firstHour >= after.from && cluster.lastHour <= 30_000);
  assert.ok(shared.length > 10);
  for (const cluster of shared) assert.ok(held.has(key(cluster)), `${key(cluster)} kept through the scroll`);
  const binHours = binHoursOf(8760, 600);
  for (const cluster of after.clusters.slice(0, 20)) {
    let sum = 0; for (let bin = 0; bin < cluster.firstHour / binHours; bin++) sum += dropsAt(0, bin);
    assert.equal(cluster.firstOrdinal, String(sum));
  }
  view.setView(following(30_336, 730));
  await settle();
  const zoomed = view.current!.get('GER')!.marks!;
  assert.notEqual(zoomed.clusters[0].lastHour - zoomed.clusters[0].firstHour, after.clusters[0].lastHour - after.clusters[0].firstHour);
});


