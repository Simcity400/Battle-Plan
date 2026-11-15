import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import { addedAt, causeAt, contributionCell, eventsThrough, layerContributions, layerUnits, otherUnits, percent, percentOf, shareSummaries, shareSummary, shareText, shareUnits,
  SUM_CELL, TENSION_CELL, validateThreatEvents, validateWorldTension, worldCells } from './tensionData.ts';
import type { ThreatEventsReply, WorldTensionReply } from '../../generated/dashboard';

const REPLY: WorldTensionReply = { tensionScale: 1000, contributionScale: 100000, ceilingRaw: '10000000', unnamed: false, missing: null, warnings: [], sources: [],
  contributions: [
    { slot: 2, tag: 'GER', remembered: false },
    { slot: 7, tag: null, remembered: false },
    { slot: 9, tag: 'AUS', remembered: true },
    { slot: 4, tag: 'ITA', remembered: false },
  ] };

const values = (cells: Record<string, number>) => ({ get: (key: string) => cells[key], missing: (key: string) => key in cells ? '' : 'Not read yet' });

test('tension and contributions read their own scales as percentages', () => {
  assert.equal(percent('50000', 1000), '50%');
  assert.equal(percent('2500000', 100000), '25%');
  assert.equal(percentOf(833333, 100000, 2), '8.33%');
  assert.throws(() => validateWorldTension({ ...REPLY, unnamed: 'no' }));
  assert.throws(() => validateWorldTension({ ...REPLY, contributions: [{ slot: 1, tag: 'GER', remembered: false, valueRaw: '1' }].map(({ slot, tag }) => ({ slot, tag })) }));
});

test('a contribution’s share of the tension shown is its part of the sum, in contribution units, as the game divides it', () => {
  // A 30% contribution of a 60% sum, with 50% shown (50,000 tension units): half of it, 25%.
  assert.equal(shareUnits(3_000_000n, 50_000n, 6_000_000n), 2_500_000n);
  assert.equal(shareUnits(1n, 50_000n, 3n), 1_666_666n);
  assert.equal(shareUnits(5n, 10n, 0n), 0n, 'nothing adds: no share');
  assert.equal(shareUnits(-2_000_000n, 10_000n, 4_000_000n), -500_000n, 'a peace treaty takes its share away');
});

test('layers are the greatest named contributions the countries chosen allow; Other is the rest of the tension or of the sum', () => {
  const layers = layerContributions(REPLY.contributions, 2, () => true);
  assert.deepEqual(layers.map(item => item.slot), [2, 9], 'a contributor no tag ever held stays in Other');
  assert.deepEqual(layerContributions(REPLY.contributions, 5, tag => tag !== 'AUS').map(item => item.slot), [2, 4]);
  const world = worldCells(values({ [TENSION_CELL]: 50_000, [SUM_CELL]: 6_000_000, [contributionCell(2)]: 3_000_000 }), false);
  assert.deepEqual([world.tension, world.sum], [50_000, 6_000_000]);
  assert.equal(layerUnits(3_000_000, world, true), 2_500_000);
  assert.equal(layerUnits(3_000_000, world, false), 3_000_000);
  assert.equal(otherUnits([2_500_000], world, true), 2_500_000, 'shares: the tension shown less the layers');
  assert.equal(otherUnits([3_000_000], world, false), 3_000_000, 'contributions: the sum less the layers');
  assert.equal(otherUnits([null], world, false), null, 'a layer not known leaves Other not known, never the whole');
  const unknown = worldCells(values({ [SUM_CELL]: 6_000_000 }), false);
  assert.equal(unknown.tension, null);
  assert.equal(unknown.why, 'Not read yet');
  assert.equal(layerUnits(3_000_000, unknown, true), null, 'no share without the tension');
});

const bucket = (first: number, last: number, min = Math.min(first, last), max = Math.max(first, last)): ChartBucket => ({ firstHour: 0, lastHour: 23,
  firstValue: first, lastValue: last, lastValueHour: 23, minHour: 0, minValue: min, maxHour: 23, maxValue: max, firstBreak: false, hasBreak: false });
const summary = (buckets: (ChartBucket | null)[]): ChartSummary => ({ from: 0, to: 47, width: 2, buckets });

test('a layer’s share over time scales each bin by the tension over the sum at its ends; a bin either is not known is not drawn', () => {
  const part = summary([bucket(30, 20, 20, 30), bucket(10, 10)]);
  const tension = summary([bucket(50, 40), null]);
  const sum = summary([bucket(60, 80), bucket(10, 10)]);
  const shares = shareSummary(part, tension, sum);
  const first = shares.buckets[0]!;
  assert.deepEqual([first.firstValue, first.lastValue, first.minValue, first.maxValue], [25, 10, 10, 30 * 50 / 60]);
  assert.equal(shares.buckets[1], null);
  assert.equal(shareSummary(part, tension, summary([bucket(0, 0), null])).buckets[0]!.lastValue, 0, 'nothing adds: no share');
  const tiled = (line: ChartSummary, at: number | null): ChartSummary => ({ ...line, tiles: { binHours: 24, valueAt: () => at } });
  const head = shareSummary(tiled(part, 12), tiled(tension, 40), tiled(sum, 80)).tiles!;
  assert.equal(head.valueAt(30), 6, 'the head is a share too, never the contribution beside its shares');
  assert.equal(shareSummary(tiled(part, 12), tiled(tension, null), tiled(sum, 80)).tiles!.valueAt(30), null, 'no tension at the head: not drawn');
});

test('a share line is made once for its contribution, tension and sum lines, and again only when one of them is new', () => {
  const part = summary([bucket(30, 20)]), tension = summary([bucket(50, 40)]), sum = summary([bucket(60, 80)]);
  const shares = shareSummaries();
  const first = shares(part, tension, sum);
  assert.equal(shares(part, tension, sum), first, 'the chart reads its lines each frame: the same share line, so its stack is not built again');
  assert.deepEqual(first, shareSummary(part, tension, sum));
  const landed = summary([bucket(50, 45)]);
  assert.notEqual(shares(part, landed, sum), first, 'a tile landing for the tension makes the share again');
  assert.equal(shares(part, landed, sum).buckets[0]!.lastValue, 20 * 45 / 80);
});

test('events are listed from their first read, newest first, each adding what it held at the hour; a cause is its read at the hour', () => {
  const events: ThreatEventsReply = { branchId: '1', missing: [], warnings: [], events: [
    { hour: 5, tag: 'GER', remembered: false, reason: 'Annexed Austria', firstRead: 10, lastRead: 40, open: false,
      added: [{ hour: 10, addedRaw: '500000' }, { hour: 20, addedRaw: '700000' }] },
    { hour: 12, tag: null, remembered: false, reason: 'Justified a war goal', firstRead: 12, lastRead: 90, open: true, added: [{ hour: 12, addedRaw: '300000' }] },
    { hour: 50, tag: 'ITA', remembered: false, reason: 'Invaded Ethiopia', firstRead: 50, lastRead: 90, open: true, added: [{ hour: 50, addedRaw: '100000' }] },
  ] };
  assert.deepEqual(eventsThrough(events, 30).map(event => event.reason), ['Justified a war goal', 'Annexed Austria']);
  assert.deepEqual([addedAt(events.events[0], 15), addedAt(events.events[0], 25)], ['500000', '700000']);
  assert.deepEqual([causeAt(events.events[0], 5), causeAt(events.events[0], 25), causeAt(events.events[0], 99)], [null, '700000', '700000']);
  assert.equal(shareText(-250_000, 1_000_000), '-25.00%');
  assert.equal(shareText(1, null), 'Not known');
  assert.throws(() => validateThreatEvents({ ...events, events: [{ ...events.events[0], added: [] }] }), 'an event adds something from its first read');
});
