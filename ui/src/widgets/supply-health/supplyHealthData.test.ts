import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import type { SeriesState } from '../../generated/dashboard';
import { formatShare, liveShareRuns, mostDivisions, shareRunsFromLines, supplyAt, supplyParts, supplyTooltip, totalDivisions } from './supplyHealthData.ts';
import { shareStack } from '../../charts/share.ts';

const bucket = (firstHour: number, lastHour: number, firstValue: number, more: Partial<ChartBucket> = {}): ChartBucket => ({ firstHour, lastHour,
  firstValue, lastValue: firstValue, lastValueHour: lastHour, minHour: firstHour, minValue: firstValue, maxHour: firstHour, maxValue: firstValue,
  firstBreak: false, hasBreak: false, ...more });
const line = (buckets: (ChartBucket | null)[]): ChartSummary => ({ from: 0, to: 100, width: buckets.length, buckets });
const state = (tag: string, hour: number, value: string | null, missing: string | null = null): SeriesState => ({ country: tag, hour, drop: null, missing,
  run: value === null ? null : { firstHour: hour, lastHour: hour, valueRaw: value, slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: hour,
    firstExact: true, lastExact: true } });


test('stacked shares take each bin at its first hour, merge equal neighbours, and leave a bin any line lacks as a gap', () => {
  const none = line([bucket(0, 5, 0.25), bucket(6, 11, 0.25), bucket(12, 17, 0.5), null, bucket(24, 29, 0, { firstBreak: true })]);
  const strained = line([bucket(0, 5, 0.25), bucket(6, 11, 0.25), bucket(12, 17, 0), bucket(18, 23, 0.5), bucket(24, 29, 0)]);
  const held = line([bucket(0, 5, 0.5), bucket(6, 11, 0.5), bucket(12, 17, 0.5), bucket(18, 23, 0.5), bucket(24, 29, 1)]);
  const runs = shareRunsFromLines([none, strained, held]);
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.counts.map(count => count / 1e9), run.breakBefore]), [
    [0, 11, [0.25, 0.25, 0.5], false], [12, 17, [0.5, 0, 0.5], false], [24, 29, [0, 0, 1], true]]);
  const model = shareStack(runs, 3);
  assert.equal(model.edges[model.hours.indexOf(18)] ?? null, null, 'a bin one line lacks is no share');
  assert.deepEqual(shareRunsFromLines([none, strained, undefined]), [], 'a line not yet drawn draws nothing');
  const shifted = line([bucket(1, 6, 0.5), bucket(7, 12, 0.5)]);
  assert.deepEqual(shareRunsFromLines([shifted, strained, held]), [], 'bins of other views never stack');
  const tiled = (summary: ChartSummary, value: number | null): ChartSummary => ({ ...summary, tiles: { binHours: 6, valueAt: () => value } });
  const headed = shareRunsFromLines([tiled(none, 0.1), tiled(strained, 0.2), tiled(held, 0.7)], 14);
  assert.deepEqual(headed.map(run => [run.firstHour, run.lastHour, run.counts.map(count => count / 1e9)]), [[0, 11, [0.25, 0.25, 0.5]], [12, 14, [0.1, 0.2, 0.7]]],
    'the head bin is the followed shares, never its first hour’s nor an hour past the head');
  assert.deepEqual(shareRunsFromLines([tiled(none, null), tiled(strained, 0.2), tiled(held, 0.7)], 14).map(run => run.lastHour), [11],
    'no share at the head while any is not known');
  const broken = shareRunsFromLines([none, line([bucket(0, 5, 0.25), bucket(6, 11, 0.25, { hasBreak: true }), ...strained.buckets.slice(2)]), held]);
  assert.deepEqual(broken.map(run => [run.firstHour, run.lastHour, run.breakBefore]), [[0, 5, false], [12, 17, false], [24, 29, true]],
    'a bin with hours not recorded is no share');
  const unread = [{ firstHour: 13, lastHour: 13, reason: 'not available' }];
  const after = shareRunsFromLines([tiled(none, 0.1), tiled(strained, 0.2), tiled(held, 0.7)], 14, unread);
  assert.deepEqual(after.map(run => [run.firstHour, run.lastHour, run.breakBefore]), [[0, 11, false], [14, 14, true]],
    'the head bin is drawn only from the hour after its last hour not recorded');
  assert.deepEqual(shareRunsFromLines([tiled(none, 0.1), tiled(strained, 0.2), tiled(held, 0.7)], 13, unread).map(run => run.lastHour), [11],
    'no head share where the head itself is not recorded');
});

test('live shares are made again when a followed answer at the head moves, and wake the chart', () => {
  let share: number | null = null, version = 0;
  const tiled = (value: () => number | null) => ({ from: 0, to: 100, width: 3, buckets: [bucket(0, 5, 0.25), bucket(6, 11, 0.25)],
    tiles: { binHours: 6, valueAt: value } });
  const lines = [() => share, () => share === null ? null : 0.2, () => share === null ? null : 0.7].map(value => {
    const held = new Map([['A', { summary: tiled(value), missing: [] }]]) as never;
    return { read: () => held, subscribe: () => () => {} };
  });
  const listeners = new Set<() => void>();
  const answers = { read: () => [version], subscribe: (listener: () => void) => { listeners.add(listener); return () => listeners.delete(listener); } };
  const live = liveShareRuns('A', runs => runs, lines, answers);
  let woken = 0;
  const stop = live.subscribe(() => woken++);
  assert.deepEqual(live.read(14).map(run => run.lastHour), [11]);
  share = 0.1; version++;
  for (const listener of listeners) listener();
  assert.equal(woken, 1, 'an answer landing wakes the chart');
  assert.deepEqual(live.read(14).map(run => [run.firstHour, run.lastHour]), [[0, 11], [12, 14]], 'the head bin is the answer now held');
  stop();
  assert.equal(listeners.size, 0);
});

test('counts at an hour are the three states, the first reason any is missing, or not read yet', () => {
  const replies = [[state('A', 10, '1'), state('B', 10, null, 'Not recorded at this hour')], [state('A', 10, '2'), state('B', 10, '0')],
    [state('A', 10, '0'), state('B', 10, '3')]];
  assert.deepEqual(supplyAt(replies, 'A', 10), { counts: { noSupply: 1, strained: 2, inSupply: 0 } });
  assert.deepEqual(supplyAt(replies, 'B', 10), { missing: 'Not recorded at this hour' });
  assert.equal(supplyAt(replies, 'A', 11), undefined);
  assert.equal(supplyAt(replies.slice(0, 2), 'A', 10), undefined);
  assert.equal(supplyAt(undefined, 'A', 10), undefined);
});

test('supply shares and tooltip retain exact counts, gaps, zero denominators and loading', () => {
  assert.equal(formatShare(1, 3), '33.33%');
  assert.equal(formatShare(2, 3), '66.67%');
  assert.equal(formatShare(0, 0), 'Not known');
  const counts = { noSupply: 1, strained: 1, inSupply: 1 };
  const tip = supplyTooltip('hover date', 'hovered name', { flag: 'hovered flag' }, { counts }, ['bad', 'warn', 'good']);
  assert.equal(tip.title, 'hover date · hovered name');
  assert.equal(tip.countryFlag?.flag, 'hovered flag');
  assert.deepEqual(tip.rows.map(row => [row.label, row.value, row.swatch]), [
    ['In supply', '33.33% · 1 division', 'good'], ['Strained', '33.33% · 1 division', 'warn'], ['No supply', '33.33% · 1 division', 'bad'],
  ]);
  assert.equal(supplyTooltip('date', 'name', undefined, { counts: { ...counts, strained: 2 } }, []).rows[1].value, '50.00% · 2 divisions');
  const missing = supplyTooltip('date', 'name', undefined, { missing: 'SupplyHealth: its input DivisionStatus is unavailable' }, []);
  assert.equal(missing.rows[0].value, 'Not known');
  assert.equal(missing.rows[0].detail, 'SupplyHealth: its input DivisionStatus is unavailable');
  assert.equal(missing.note, 'Supply was not recorded at this time.');
  const zero = supplyTooltip('date', 'name', undefined, { counts: { noSupply: 0, strained: 0, inSupply: 0 } }, []);
  assert.equal(zero.rows[0].value, '0');
  assert.match(zero.note!, /No divisions/);
  assert.equal(supplyTooltip('date', 'name', undefined, undefined, []).rows[0].value, 'Loading…');
});

test('the breakdown lists every state with its share, and a view of no divisions is a known zero', () => {
  const parts = supplyParts({ noSupply: 1, strained: 3, inSupply: 0 }, ['bad', 'warn', 'good']);
  assert.deepEqual(parts.map(part => [part.id, part.value, part.text, part.shareText, part.color]), [
    ['noSupply', 1, '1', '25.00%', 'bad'], ['strained', 3, '3', '75.00%', 'warn'], ['inSupply', 0, '0', '0.00%', 'good']]);
  assert.equal(totalDivisions({ noSupply: 1, strained: 3, inSupply: 0 }), 4);
  assert.equal(mostDivisions(line([bucket(0, 5, 0), null, bucket(12, 17, 0)])), 0);
  assert.equal(mostDivisions(line([bucket(0, 5, 0), bucket(6, 11, 4, { maxValue: 7 })])), 7);
  assert.equal(mostDivisions(line([null, null])), null);
  assert.equal(mostDivisions(undefined), null);
});
