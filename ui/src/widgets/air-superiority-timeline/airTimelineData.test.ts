import assert from 'node:assert/strict';
import test from 'node:test';
import { valuesOf, liveKey } from '../../charts/live.ts';
import { sideOrder } from '../../dashboard/airSuperiority.ts';
import type { AirLooks } from '../../dashboard/airSuperiority';
import type { AirSide, AirSuperiorityReply, AirTimelineReply } from '../../generated/dashboard';
import { STEP, airPeriods, dayView, followedEnd, headTail, joinTiles, timelineRows, validateAirTimeline, viewTiles } from './airTimelineData.ts';

const side = (key: string, faction: string | null, leader: string): AirSide =>
  ({ key, faction, colourFaction: null, factions: faction ? [faction] : [], tag: faction ? null : leader, leader, coBelligerents: 0, anchor: null });
const SIDES = [side('f:7', '7', 'ENG'), side('c:GER', null, 'GER')];
const looks: AirLooks = { name: item => item.key, color: item => `colour ${item.key}`, viewpoint: { name: 'ENG', tag: 'ENG', color: 'colour ENG' }, enemy: 'enemy',
  scale: 100_000, badBelow: 40_000, goodAbove: 60_000, mode: 'sides' };

function reply(): AirTimelineReply {
  const shares = { kind: 'sides' as const, parts: [{ side: 0, tag: 'ENG', friendly: '300' }, { side: 1, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false };
  return { firstHour: 24, lastHour: 119, dayHours: 24, days: [47, 71, null, 119], excluded: [0, 0, 0, 2], sides: SIDES, legend: [0, 1], viewpoint: null, gaps: [], scale: 100_000, badBelow: 40_000, goodAbove: 60_000,
    warnings: [], rows: [
      { region: 5, name: 'Channel', nameMissing: null, pinned: false, cells: [[2, shares], [1, null], [1, { kind: 'reason', reason: 'outside-theatres' }]] },
      { region: 6, name: null, nameMissing: 'not in the stack', pinned: false, cells: [[1, { kind: 'sides', parts: [], viewpoints: [], unknown: [1], noAirObject: [], filtered: false }], [3, { kind: 'sides', parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false }]] },
    ] };
}

test('a timeline reply is validated: its runs cover its days and its days fall in their hours', () => {
  assert.equal(validateAirTimeline(reply()).rows.length, 2);
  const short = reply(); short.rows[0].cells.pop();
  assert.throws(() => validateAirTimeline(short), /Invalid/);
  const late = reply(); late.days[0] = 48;
  assert.throws(() => validateAirTimeline(late), /Invalid/);
  const side = reply(); side.rows[1].cells[0] = [1, { kind: 'sides', parts: [], viewpoints: [], unknown: [9], noAirObject: [], filtered: false }];
  assert.throws(() => validateAirTimeline(side), /Invalid/);
});

test('each day is a heat cell of the shares its scores give; a day not read, unknown or outside the theatres says so', () => {
  const rows = timelineRows(reply(), looks);
  assert.deepEqual(rows.map(row => [row.id, row.label]), [['5', 'Channel'], ['6', 'Region 6']]);
  const [first, second, gap, outside] = rows[0].cells;
  assert.deepEqual(first.parts.map(part => [part.id, part.share]), [['f:7', 0.75], ['c:GER', 0.25]]);
  assert.equal(first, second, 'a run shares its cell');
  assert.deepEqual([gap.parts.length, gap.unknown, gap.filtered], [0, false, false]);
  assert.deepEqual([outside.parts.length, outside.filtered], [0, true]);
  assert.equal(rows[1].cells[0].unknown, true, 'a side whose score is unknown hatches the day');
  assert.deepEqual(airPeriods(reply()).map(period => [period.firstHour, period.lastHour, period.readHour]), [[24, 47, 47], [48, 71, 71], [72, 95, null], [96, 119, 119]]);
});

test('a day view names its state and carries the sides the comparison left out that day', () => {
  const data = reply(), order = sideOrder(data.sides);
  assert.equal(dayView(null, data, looks, order, 2).state, 'gap');
  const empty = dayView(data.rows[1].cells[1][1], data, looks, order, 3);
  assert.deepEqual([empty.state, empty.excluded], ['inactive', 2]);
  const known = dayView(data.rows[0].cells[0][1], data, looks, order, 0);
  assert.deepEqual([known.state, known.total, known.parts.map(part => part.tag)], ['known', 400, ['ENG', 'GER']]);
  const view = dayView({ kind: 'view', side: null, leader: 'ENG', friendly: '0', enemy: '0' }, data, { ...looks, mode: 'viewpoint' }, order, 0);
  assert.deepEqual([view.state, view.total], ['inactive', 0], 'a viewpoint with no row there is a known zero');
});

test('following, the view ends at the end of the head\'s step while playing and at the head when paused', () => {
  assert.equal(followedEnd(STEP + 5.5, true), 2 * STEP - 1);
  assert.equal(followedEnd(STEP - 1, true), STEP - 1);
  assert.equal(followedEnd(STEP + 5.5, false), STEP + 5);
});

test('the head\'s day is drawn from the moment\'s values through the hour they are of', () => {
  const moment: AirSuperiorityReply = { missing: null, sides: SIDES, legend: [0, 1], viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [],
    regions: [{ id: 5, name: 'Channel', nameMissing: null, active: true, wings: 'none',
      cell: { kind: 'sides', parts: [{ side: 0, tag: 'ENG', live: 'ENG@5' }, { side: 1, tag: 'GER', live: 'GER@5' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 } }],
    fallback: { kind: 'sides', parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 } };
  const values = valuesOf(new Map([[liveKey('ENG@5', 'friendly'), 10], [liveKey('GER@5', 'friendly'), 30]]), 'not followed');
  const { tail, views } = headTail(moment, values, looks, [{ id: 5, name: 'Channel' }, { id: 6, name: 'Region 6' }], 130, 24);
  assert.deepEqual(tail.period, { firstHour: 120, lastHour: 130, readHour: 130 });
  assert.deepEqual(tail.cells[0].parts.map(part => [part.id, part.share]), [['f:7', 0.25], ['c:GER', 0.75]]);
  assert.deepEqual(tail.cells[1].parts, [], 'a region no country scores in is the fallback: no activity');
  assert.equal(views.get(5)?.total, 40);
  const unread = headTail(moment, valuesOf(new Map(), 'not followed'), looks, [{ id: 5, name: 'Channel' }], 130, 24);
  assert.equal(unread.tail.cells[0].unknown, true, 'a score not yet followed is unknown, never zero');
});

test("a view's cells come in tiles: those it covers whole asked through their end, the last cut at the view's hour", () => {
  assert.deepEqual(viewTiles(100, 2 * STEP - 1), { whole: [0, 1], cut: null });
  assert.deepEqual(viewTiles(100, 2 * STEP + 5), { whole: [0, 1], cut: 2 });
  assert.deepEqual(viewTiles(STEP + 5, STEP + 9), { whole: [], cut: 1 });
});

test('tiles join in hour order, each made with its own sides; a tile not read leaves its hours loading', () => {
  const first = reply();
  const second: AirTimelineReply = { ...reply(), firstHour: 120, lastHour: 167, days: [143, 167], excluded: [0, 0], sides: [SIDES[1], SIDES[0]],
    rows: [{ region: 5, name: 'Channel', nameMissing: null, pinned: false, cells: [[2, { kind: 'sides', parts: [{ side: 0, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false }]] }] };
  const regions = [{ id: 5, name: 'Channel' }, { id: 6, name: 'Region 6' }];
  const joined = joinTiles([first, undefined, second], [{ firstHour: 24, lastHour: 119 }, { firstHour: 120, lastHour: 120 }, { firstHour: 120, lastHour: 167 }], looks, regions);
  assert.deepEqual(joined.periods.map(period => period.firstHour), [24, 48, 72, 96, 120, 144]);
  assert.deepEqual(joined.loading, [{ firstHour: 120, lastHour: 120 }]);
  assert.equal(joined.rows[0].cells.length, 6);
  assert.deepEqual(joined.rows[0].cells[4].parts.map(part => part.id), ['c:GER'], "the second tile's side 0 is its own GER");
  assert.equal(joined.rows[1].cells.length, 6, 'a region a tile does not hold reads as no activity there');
});
