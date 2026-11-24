import assert from 'node:assert/strict';
import test from 'node:test';
import { valuesOf, liveKey } from '../../charts/live.ts';
import { resolveSideColours, sideColor, sideOrder } from '../../dashboard/airSuperiority.ts';
import type { AirLooks } from '../../dashboard/airSuperiority';
import type { AirSide, AirSuperiorityReply, AirTimelineReply } from '../../generated/dashboard';
import { STEP, activeLegendCutoff, airPeriods, dayView, followedEnd, headTail, joinTiles, mergeTimelineSides, tileLegendChangeHours, timelineRows, timelineViewpointSide, validateAirTimeline, viewTiles } from './airTimelineData.ts';

const side = (key: string, faction: string | null, leader: string): AirSide =>
  ({ key, faction, colourFaction: null, factions: faction ? [faction] : [], tag: faction ? null : leader, leader, coBelligerents: 0, anchor: null });
const SIDES = [side('f:7', '7', 'ENG'), side('c:GER', null, 'GER')];
const looks: AirLooks = { name: item => item.key, color: item => `colour ${item.key}`, viewpoint: { name: 'ENG', tag: 'ENG', color: 'colour ENG' }, enemy: 'enemy',
  scale: 100_000, badBelow: 40_000, goodAbove: 60_000, mode: 'sides' };

function reply(): AirTimelineReply {
  const shares = { kind: 'sides' as const, parts: [{ side: 0, tag: 'ENG', friendly: '300' }, { side: 1, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false };
  return { firstHour: 24, lastHour: 119, dayHours: 24, days: [47, 71, null, 119], excluded: [0, 0, 0, 2], sides: SIDES, viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000,
    warnings: [], rows: [
      { region: 5, name: 'Channel', nameMissing: null, cells: [[2, shares], [1, null], [1, { kind: 'reason', reason: 'outside-theatres' }]] },
      { region: 6, name: null, nameMissing: 'not in the stack', cells: [[1, { kind: 'sides', parts: [], viewpoints: [], unknown: [1], noAirObject: [], filtered: false }], [3, { kind: 'sides', parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false }]] },
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
  const moment: AirSuperiorityReply = { missing: null, sides: SIDES, viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [],
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
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[2, { kind: 'sides', parts: [{ side: 0, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false }]] }] };
  const regions = [{ id: 5, name: 'Channel' }, { id: 6, name: 'Region 6' }];
  const joined = joinTiles([first, undefined, second], [{ firstHour: 24, lastHour: 119 }, { firstHour: 120, lastHour: 120 }, { firstHour: 120, lastHour: 167 }], looks, regions);
  assert.deepEqual(joined.periods.map(period => period.firstHour), [24, 48, 72, 96, 120, 144]);
  assert.deepEqual(joined.loading, [{ firstHour: 120, lastHour: 120 }]);
  assert.equal(joined.rows[0].cells.length, 6);
  assert.deepEqual(joined.rows[0].cells[4].parts.map(part => part.id), ['c:GER'], "the second tile's side 0 is its own GER");
  assert.equal(joined.rows[1].cells.length, 6, 'a region a tile does not hold reads as no activity there');
});

test('merging timeline sides selects metadata by its last displayed day of use across valid successive tiles and dissolved coalitions', () => {
  const early: AirSide = { key: 'f:10', faction: '10', colourFaction: null, factions: ['10'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'GER', hour: 0 } };
  const late: AirSide = { key: 'f:10', faction: '10', colourFaction: '10', factions: ['10'], tag: null, leader: 'GER', coBelligerents: 1, anchor: { tag: 'GER', hour: 0 } };
  const other: AirSide = { key: 'f:20', faction: '20', colourFaction: '20', factions: ['20'], tag: null, leader: 'JAP', coBelligerents: 0, anchor: { tag: 'JAP', hour: 100 } };
  const cellOf = (sideIndex: number, tag = 'GER') => ({ kind: 'sides' as const, parts: [{ side: sideIndex, tag, friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false });
  const firstTile: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 24, lastHour: 71, days: [47, 71], excluded: [0, 0],
    sides: [early],
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[2, cellOf(0)]] }],
  });
  const secondTile: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 72, lastHour: 119, days: [95, 119], excluded: [0, 0],
    sides: [other, late],
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[1, cellOf(0, 'JAP')], [1, cellOf(1, 'GER')]] }],
  });
  assert.deepEqual(mergeTimelineSides([firstTile, secondTile], { regions: [5], throughHour: 119 }), [late, other]);

  const solo: AirSide = { key: 'f:10', faction: '10', colourFaction: '10', factions: ['10'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'GER', hour: 0 } };
  const coalition: AirSide = { key: 'f:10', faction: '10', colourFaction: '10', factions: ['10', '20'], tag: null, leader: 'GER', coBelligerents: 1, anchor: { tag: 'GER', hour: 0 } };
  const dissolved: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 24, lastHour: 95, days: [47, 71, 95], excluded: [0, 0, 0],
    sides: [solo, coalition],
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[1, cellOf(0)], [1, cellOf(1)], [1, cellOf(0)]] }],
  });
  assert.deepEqual(mergeTimelineSides([dissolved], { regions: [5], throughHour: 95 }), [solo], 'dissolved coalition on day 2 must not override solo faction reused on day 3');
});

test('an early solo cell keeps its own grey anchor colour when a later coalition with the same key enters the view', () => {
  const earlySolo: AirSide = { key: 'f:10', faction: '10', colourFaction: null, factions: ['10'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'GER', hour: 0 } };
  const lateCoalition: AirSide = { key: 'f:10', faction: '10', colourFaction: '20', factions: ['10', '20'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'JAP', hour: 100 } };
  const lateNeutralCoalition: AirSide = { key: 'f:10', faction: '10', colourFaction: null, factions: ['10', '20'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'JAP', hour: 100 } };
  const cellOf = (sideIndex: number) => ({ kind: 'sides' as const, parts: [{ side: sideIndex, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false });
  const tile: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 24, lastHour: 95, days: [47, 71, 95], excluded: [0, 0, 0],
    sides: [earlySolo, lateCoalition, lateNeutralCoalition],
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[1, cellOf(0)], [1, cellOf(1)], [1, cellOf(2)]] }],
  });
  const anchorRgb = new Map<string, [number, number, number]>([['GER@0', [90, 90, 90]], ['JAP@100', [210, 190, 140]]]);
  const factionColours = new Map<string, string>([['20', '#d2be8c']]);
  const fallbackColours = resolveSideColours(tile.sides, (tag, hour) => anchorRgb.get(`${tag}@${hour}`) ?? null, () => null, null, '#101418', false);
  const cellLooks: AirLooks = {
    ...looks,
    color: item => sideColor(item, factionColours, fallbackColours, '#888888'),
  };
  const joined = joinTiles([tile], [{ firstHour: 24, lastHour: 95 }], cellLooks, [{ id: 5, name: 'Channel' }]);
  assert.equal(joined.rows[0].cells[0].parts[0].color, '#5a5a5a', 'early solo cell keeps German grey');
  assert.equal(joined.rows[0].cells[1].parts[0].color, '#d2be8c', 'later majority coalition uses faction 20 beige');
  assert.equal(joined.rows[0].cells[2].parts[0].color, '#d2be8c', 'later fallback coalition uses its own Japanese anchor');
});

test('follow at day two shows solo, respects runs crossing the cutoff, and keeps solo after seeking back and resuming', () => {
  const solo: AirSide = { key: 'f:10', faction: '10', colourFaction: '10', factions: ['10'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'GER', hour: 0 } };
  const coalition: AirSide = { key: 'f:10', faction: '10', colourFaction: '20', factions: ['10', '20'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'JAP', hour: 100 } };
  const cellOf = (sideIndex: number) => ({ kind: 'sides' as const, parts: [{ side: sideIndex, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false });
  const emptyCell = { kind: 'sides' as const, parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false };
  const prefetched: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 24, lastHour: 95, days: [47, 71, 95], excluded: [0, 0, 0],
    sides: [solo, coalition],
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[1, cellOf(0)], [1, emptyCell], [1, cellOf(1)]] }],
  });
  const changes = tileLegendChangeHours([prefetched], [5]);
  assert.deepEqual(changes, [47, 95]);
  const dayTwoCutoff = activeLegendCutoff(71, changes, 71);
  assert.deepEqual(mergeTimelineSides([prefetched], { regions: [5], throughHour: dayTwoCutoff }), [solo], 'Follow at day 2 must not show day 3 coalition');
  assert.deepEqual(mergeTimelineSides([prefetched], { regions: [5], throughHour: activeLegendCutoff(95, changes, 95) }), [coalition], 'advancing to day 3 shows coalition');
  const cutPaused: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 24, lastHour: 71, days: [47, 71], excluded: [0, 0],
    sides: [solo],
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[1, cellOf(0)], [1, emptyCell]] }],
  });
  assert.deepEqual(mergeTimelineSides([cutPaused], { regions: [5], throughHour: 71 }), [solo], 'backward seek paused at day 2 shows solo');
  assert.deepEqual(mergeTimelineSides([prefetched], { regions: [5], throughHour: activeLegendCutoff(71, changes, 71) }), [solo], 'resuming playback with prefetched tile still shows solo at day 2');

  const crossing: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 24, lastHour: 95, days: [47, 71, 95], excluded: [0, 0, 0],
    sides: [solo, coalition],
    rows: [
      { region: 5, name: 'Channel', nameMissing: null, cells: [[3, cellOf(0)]] },
      { region: 6, name: 'Baltic', nameMissing: null, cells: [[1, cellOf(1)], [2, emptyCell]] },
    ],
  });
  assert.deepEqual(mergeTimelineSides([crossing], { regions: [5, 6], throughHour: 75 }), [solo], 'read hour 71 inside a 3-day run crossing cutoff 75 beats coalition at 47');
});

test('a pinned viewpoint uses the exact viewpoint entry returned by the query even when the final day cell is a reason', () => {
  const solo: AirSide = { key: 'f:10', faction: '10', colourFaction: '10', factions: ['10'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'GER', hour: 0 } };
  const coalition: AirSide = { key: 'f:10', faction: '10', colourFaction: '20', factions: ['10', '20'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'JAP', hour: 100 } };
  const viewOf = (sideIndex: number) => ({ kind: 'view' as const, side: sideIndex, leader: 'GER', friendly: '100', enemy: '50' });
  const pinned: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 24, lastHour: 95, days: [47, 71, 95], excluded: [0, 0, 0],
    sides: [solo, coalition],
    viewpoint: 0,
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[1, viewOf(0)], [1, viewOf(1)], [1, { kind: 'reason', reason: 'outside-theatres' }]] }],
  });
  assert.deepEqual(timelineViewpointSide([pinned], { regions: [5], throughHour: 95 }), solo);
});

test('an off-chart live region does not override the visible legend while displayed live cells and applicable fallback do', () => {
  const solo: AirSide = { key: 'f:10', faction: '10', colourFaction: '10', factions: ['10'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'GER', hour: 0 } };
  const coalition: AirSide = { key: 'f:10', faction: '10', colourFaction: '20', factions: ['10', '20'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'JAP', hour: 100 } };
  const liveOnly: AirSide = { key: 'c:ITA', faction: null, colourFaction: null, factions: [], tag: 'ITA', leader: 'ITA', coBelligerents: 0, anchor: null };
  const fallbackSide: AirSide = { key: 'f:30', faction: '30', colourFaction: '30', factions: ['30'], tag: null, leader: 'SOV', coBelligerents: 0, anchor: { tag: 'SOV', hour: 0 } };
  const history: AirTimelineReply = validateAirTimeline({
    ...reply(), firstHour: 24, lastHour: 71, days: [47, 71], excluded: [0, 0],
    sides: [solo],
    rows: [{ region: 5, name: 'Channel', nameMissing: null, cells: [[2, { kind: 'sides', parts: [{ side: 0, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false }]] }],
  });
  const emptyLive = { kind: 'sides' as const, parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 };
  const moment: AirSuperiorityReply = {
    missing: null, sides: [coalition, liveOnly, fallbackSide], viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [],
    regions: [
      { id: 5, name: 'Channel', nameMissing: null, active: true, wings: 'none', cell: { ...emptyLive, parts: [{ side: 1, tag: 'ITA', live: 'ITA@5' }] } },
      { id: 99, name: 'Pacific', nameMissing: null, active: true, wings: 'none', cell: { ...emptyLive, parts: [{ side: 0, tag: 'GER', live: 'GER@99' }] } },
    ],
    fallback: { ...emptyLive, unknown: [2] },
  };
  assert.deepEqual(mergeTimelineSides([history], { regions: [5], throughHour: 95, moment, momentHour: 95 }), [solo, liveOnly], 'region 99 must not override region 5, while live-only ITA in region 5 enters the legend');
  assert.deepEqual(mergeTimelineSides([history], { regions: [5, 6], throughHour: 95, moment, momentHour: 95 }), [solo, liveOnly, fallbackSide], 'applicable fallback for missing displayed region 6 enters the legend');
});

test('a moment update reads zero historical cells once tiles are cached', () => {
  const solo: AirSide = { key: 'f:10', faction: '10', colourFaction: '10', factions: ['10'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'GER', hour: 0 } };
  const coalition: AirSide = { key: 'f:10', faction: '10', colourFaction: '20', factions: ['10', '20'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'JAP', hour: 100 } };
  let cellReads = 0;
  const regions = Array.from({ length: 10 }, (_, idx) => idx + 1);
  const tiles: AirTimelineReply[] = Array.from({ length: 20 }, (_, tileIndex) => {
    const firstHour = tileIndex * STEP;
    const days = Array.from({ length: 4 }, (_, day) => firstHour + (day + 1) * 24 - 1);
    const lastHour = days[days.length - 1];
    const rows = regions.map(region => {
      const backing: [number, { kind: 'sides'; parts: { side: number; tag: string; friendly: string }[]; viewpoints: [number, string][]; unknown: number[]; noAirObject: number[]; filtered: boolean }][] =
        [[4, { kind: 'sides', parts: [{ side: 0, tag: 'GER', friendly: '100' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false }]];
      const row = { region, name: `Region ${region}`, nameMissing: null, cells: backing };
      Object.defineProperty(row, 'cells', {
        enumerable: true,
        get() { cellReads++; return backing; },
      });
      return row;
    });
    return { firstHour, lastHour, dayHours: 24, days, excluded: [0, 0, 0, 0], sides: [solo], viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [], rows };
  });
  const cutoff = 20 * STEP;
  assert.deepEqual(mergeTimelineSides(tiles, { regions, throughHour: cutoff }), [solo]);
  assert.ok(cellReads > 0, 'initial tile indexing reads historical cells once');
  cellReads = 0;
  const moment: AirSuperiorityReply = {
    missing: null, sides: [coalition], viewpoint: null, scale: 100_000, badBelow: 40_000, goodAbove: 60_000, warnings: [],
    regions: [{ id: 1, name: 'Region 1', nameMissing: null, active: true, wings: 'none', cell: { kind: 'sides', parts: [{ side: 0, tag: 'GER', live: 'GER@1' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 } }],
    fallback: { kind: 'sides', parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 },
  };
  assert.deepEqual(mergeTimelineSides(tiles, { regions, throughHour: cutoff, moment, momentHour: cutoff }), [coalition]);
  assert.deepEqual(tileLegendChangeHours(tiles, regions).length > 0, true);
  assert.equal(timelineViewpointSide(tiles, { regions, throughHour: cutoff, moment, momentHour: cutoff }), null);
  assert.equal(cellReads, 0, 'moment update must read zero historical cells');
});
