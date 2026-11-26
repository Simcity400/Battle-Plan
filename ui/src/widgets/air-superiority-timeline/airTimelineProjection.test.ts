import { assertRunCounts } from '../../families/heat/runIdentity.fixture.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveLanesTooltip } from '../../families/heat/lanesModel.ts';
import type { AirSide, AirTimelineReply } from '../../generated/dashboard';
import { DAY_TILE_HOURS } from '../../families/heat/dayTiles.ts';
import { projectAirTimeline } from './airTimelineProjection.ts';

const SURFACE = '#161616';

function side(key: string, extra: Partial<AirSide>): AirSide {
  return { key, faction: null, colourFaction: null, factions: [], tag: null, leader: null, coBelligerents: 0, anchor: null, ...extra };
}

function reply(lastHour: number, extra: Partial<AirTimelineReply> = {}): AirTimelineReply {
  return {
    firstHour: 0,
    lastHour,
    dayHours: 24,
    days: [23, null, lastHour],
    excluded: [0, 0, 0],
    sides: [
      side('f:10', { name: 'Axis', color: [200, 40, 40], anchor: { tag: 'GER', hour: 0 } }),
      side('f:20', { name: 'Allies', color: [40, 80, 220], anchor: { tag: 'GER', hour: 72 } }),
      side('f:30', { nameMissing: 'The faction was unnamed at this date' }),
    ],
    legend: [0, 1],
    viewpoint: null,
    gaps: [],
    rows: [{
      region: 5,
      name: 'Southern England',
      nameMissing: null,
      pinned: false,
      readout: { kind: 'leading', side: 0, share_bp: 7100, as_of_hour: lastHour },
      cells: [
        [1, { kind: 'sides', parts: [{ side: 0, tag: 'GER', friendly: '71' }, { side: 1, tag: 'ENG', friendly: '29' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, lead: { kind: 'leading', side: 0, share_bp: 7100 } }],
        [1, null],
        [1, { kind: 'sides', parts: [{ side: 2, tag: 'ITA', friendly: '50' }, { side: 1, tag: 'ENG', friendly: '50' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, lead: { kind: 'contested' } }],
      ],
    }],
    scale: 100000,
    badBelow: 0,
    goodAbove: 0,
    warnings: [],
    rankMissing: null,
    empty: null,
    activeUnlisted: { ids: [], missing: null },
    ...extra,
  };
}

const view = (hour: number, data: AirTimelineReply) => createBoundedReadView({ hour, panels: new Map([['now', data]]) });
const OPTIONS = { viewpointCountry: null, historyStart: 0, tiles: 0, surface: SURFACE };

test('the readout names the leading side by the product and paints it in its own colour', () => {
  const model = projectAirTimeline(view(50, reply(50)), OPTIONS);
  const readout = model.rows[0]!.readout!;
  assert.equal(readout.kind, 'value');
  if (readout.kind !== 'value') return;
  assert.equal(readout.primary, '71%');
  assert.equal(readout.secondary, 'Axis');
  assert.ok(readout.swatch && readout.swatch !== 'hollow');
});

test('two sides founded by one tag at different hours keep their own colours', () => {
  const model = projectAirTimeline(view(50, reply(50)), OPTIONS);
  const items = deriveLanesTooltip(model, 0, 10)!.items!;
  assert.notEqual(items[0]!.swatch, items[1]!.swatch);
});

test('an unnamed side says its name was not recorded, never a later name', () => {
  const model = projectAirTimeline(view(50, reply(50)), OPTIONS);
  const items = deriveLanesTooltip(model, 0, 50)!.items!;
  assert.equal(items.find(item => item.label === 'Name not recorded')?.value, '50%');
  assert.deepEqual(items[0], { label: 'Contested' });
});

test('a hit is answered from its own day, and a painted gap or the readout before any run', () => {
  const contested = (side: number, tag: string) => ({ kind: 'sides' as const, parts: [{ side, tag, friendly: '50' }, { side: 1, tag: 'ENG', friendly: '50' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, lead: { kind: 'contested' as const } });
  const data = reply(71, {
    days: [23, 47, 71],
    rows: [{ ...reply(71).rows[0]!, cells: [[1, contested(0, 'GER')], [2, contested(2, 'ITA')]] }],
  });
  const model = projectAirTimeline(view(71, data), OPTIONS);
  assert.equal(model.rows[0]!.runs[0]!.cell.kind === 'solid' && model.rows[0]!.runs[1]!.cell.kind === 'solid'
    && model.rows[0]!.runs[0]!.cell.color === model.rows[0]!.runs[1]!.cell.color, true, 'both days paint as contested');
  const dayOne = deriveLanesTooltip(model, 0, 10)!;
  const dayTwo = deriveLanesTooltip(model, 0, 30)!;
  assert.equal(dayOne.hour, 23);
  assert.ok(dayOne.items!.some(item => item.label === 'Axis'));
  assert.equal(dayTwo.hour, 47, 'the second day is read at its own hour');
  assert.ok(dayTwo.items!.some(item => item.label === 'Name not recorded') && !dayTwo.items!.some(item => item.label === 'Axis'));
  const gapped = projectAirTimeline(view(71, { ...data, gaps: [{ firstHour: 24, lastHour: 47 }] }), OPTIONS);
  assert.equal(deriveLanesTooltip(gapped, 0, 30)!.missingReason, 'The recording holds no hour here');
  assert.equal(deriveLanesTooltip(model, 0, null)!.exactValue, '71%', 'the readout is answered by the readout, not by the run under the playhead');
});

test('a day the recording holds no hour of is the hatch, and the lane ends at the displayed hour', () => {
  const model = projectAirTimeline(view(50, reply(50)), OPTIONS);
  const runs = model.rows[0]!.runs;
  assert.deepEqual(runs[1]!.cell, { kind: 'missing', reason: 'Not recorded on this day' });
  assert.equal(deriveLanesTooltip(model, 0, 30)!.missingReason, 'Not recorded on this day');
  assert.equal(runs.at(-1)!.lastHour, 50);
  assert.equal(model.lastHour, 50);
  const history = reply(47);
  const now = { ...reply(50), firstHour: 48, days: [50] };
  const tiled = projectAirTimeline(createBoundedReadView({ hour: 50, panels: new Map([['now', now], ['tile:0', history]]) }), { ...OPTIONS, tiles: 1 });
  assert.deepEqual(tiled.rows[0]!.runs.find(run => run.firstHour === 24)?.cell, { kind: 'missing', reason: 'Not recorded on this day' });
  assert.equal(deriveLanesTooltip(tiled, 0, 30)!.missingReason, 'Not recorded on this day');
  const earlier = { ...history, lastHour: 23 };
  const arriving = projectAirTimeline(createBoundedReadView({ hour: 50, panels: new Map([['now', now], ['tile:0', earlier]]) }), { ...OPTIONS, tiles: 1 });
  assert.equal(arriving.waiting, true);
});

test('a reply that reaches past the displayed hour draws nothing from it', () => {
  const model = projectAirTimeline(view(40, reply(50)), OPTIONS);
  assert.deepEqual(model.rows, []);
});

test('a day with a side whose score is unknown is not recorded, never a full lead for the known side', () => {
  const data = reply(50);
  const row = { ...data.rows[0]!, cells: [[3, { kind: 'sides', parts: [{ side: 0, tag: 'GER', friendly: '71' }], viewpoints: [], unknown: [1], noAirObject: [], filtered: false, lead: { kind: 'unknown' } }]] as AirTimelineReply['rows'][number]['cells'] };
  const model = projectAirTimeline(view(50, { ...data, rows: [row] }), OPTIONS);
  assert.deepEqual(model.rows[0]!.runs[0]!.cell, { kind: 'missing', reason: 'A side’s air score was not recorded' });
});

test('each air history cell is laid out once over a hundred played hours', () => {
  const start = 24 * 1000 + 5;
  const completedTiles = Math.ceil(24 * 1000 / DAY_TILE_HOURS);
  const visits = { cells: 0 };
  const counted = <T>(items: T[]): T[] => new Proxy(items, {
    get(target, key, receiver) {
      if (typeof key === 'string' && /^\d+$/.test(key)) visits.cells++;
      return Reflect.get(target, key, receiver);
    },
  });
  const sides = reply(0).sides;
  const cell = (friendly: string) => ({ kind: 'sides' as const, parts: [{ side: 0, tag: 'GER', friendly }, { side: 1, tag: 'ENG', friendly: '10' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, lead: { kind: 'leading' as const, side: 0, share_bp: 8000 } });
  const tileDays = DAY_TILE_HOURS / 24;
  const tiles = Array.from({ length: completedTiles - 1 }, (_, k): AirTimelineReply => ({
    ...reply(0),
    firstHour: k * DAY_TILE_HOURS,
    lastHour: (k + 1) * DAY_TILE_HOURS - 1,
    days: Array.from({ length: tileDays }, (_, day) => k * DAY_TILE_HOURS + day * 24 + 23),
    sides,
    rows: Array.from({ length: 7 }, (_, region) => ({
      region, name: `Region ${region}`, nameMissing: null, pinned: false,
      cells: counted(Array.from({ length: tileDays }, (_, day): [number, AirTimelineReply['rows'][number]['cells'][number][1]] => [1, cell(String(10 + (day % 2) * 50))])),
    })),
    absent: [[tileDays, null]],
  }));
  const panels = new Map<string, unknown>(tiles.map((tile, k) => [`tile:${k}`, tile]));
  for (let hour = start; hour < start + 100; hour++) {
    const today = Math.floor(hour / 24) * 24;
    const tileStart = tiles.length * DAY_TILE_HOURS;
    const days = (today - tileStart) / 24;
    panels.set(`tile:${tiles.length}`, { ...reply(today - 1), firstHour: tileStart, lastHour: today - 1,
      days: Array.from({ length: days }, (_, i) => tileStart + (i + 1) * 24 - 1), rows: [], absent: [[days, null]] });
    panels.set('now', {
      ...reply(hour),
      firstHour: today,
      lastHour: hour,
      days: [hour],
      sides,
      rows: Array.from({ length: 7 }, (_, region) => ({ region, name: `Region ${region}`, nameMissing: null, pinned: false, readout: { kind: 'noActivity', as_of_hour: hour }, cells: [[1, cell('70')]] })),
    });
    const model = projectAirTimeline(createBoundedReadView({ hour, panels }), { ...OPTIONS, tiles: completedTiles + 1 });
    assert.equal(model.rows.length, 7);
    assert.equal(model.firstHour, 0);
  }
  assert.equal(visits.cells, tiles.length * 7 * tileDays, 'each history cell is read once over a hundred hours, not once an hour');
});

test('a thousand cells of five values ask for five colours, once across frames', () => {
  let asked = 0;
  const counted = (item: AirSide): AirSide => new Proxy(item, { get: (target, key, receiver) => { if (key === 'color') asked++; return Reflect.get(target, key, receiver); } });
  const palette: [number, number, number][] = [[200, 40, 40], [40, 80, 220], [40, 160, 60], [200, 160, 40], [140, 60, 200]];
  const sides = palette.map((color, index) => counted(side(`f:${index}`, { name: `Side ${index}`, color })));
  const cell = (lead: number) => ({ kind: 'sides' as const, parts: [{ side: lead, tag: 'GER', friendly: '70' }, { side: (lead + 1) % 5, tag: 'ENG', friendly: '30' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false, lead: { kind: 'leading' as const, side: lead, share_bp: 7000 } });
  const days = 1000;
  const tile: AirTimelineReply = {
    ...reply(0), firstHour: 0, lastHour: days * 24 - 1, sides,
    days: Array.from({ length: days }, (_, day) => day * 24 + 23),
    rows: [{ region: 5, name: 'Southern England', nameMissing: null, pinned: false, cells: Array.from({ length: days }, (_, day): [number, AirTimelineReply['rows'][number]['cells'][number][1]] => [1, cell(day % 5)]) }],
    absent: [[days, null]],
  };
  for (let hour = days * 24; hour < days * 24 + 3; hour++) {
    const now = { ...reply(hour), firstHour: days * 24, lastHour: hour, days: [hour], sides: [] };
    projectAirTimeline(createBoundedReadView({ hour, panels: new Map<string, unknown>([['now', now], ['tile:0', tile]]) }), { ...OPTIONS, tiles: 1 });
  }
  assert.equal(asked, 5, 'each side colour is solved once, for every cell, row and frame');
});

test('a correction that moves a viewer colour change by a day redraws the tile at the new boundary', () => {
  const colourAt = (change: number) => ({
    resolve: (kind: string, key: string, atHour: number) => ({
      kind, key, atHour,
      holder: { status: 'ready', value: key },
      name: { status: 'ready', value: 'German Reich' },
      colour: { status: 'ready', value: atHour < change ? '#3060c0' : '#c03030' },
      validInterval: atHour < change ? [0, change - 1] : [change, Infinity],
      nextChangeAt: atHour < change ? change : Infinity,
    }),
  });
  const view = (share: number) => ({ kind: 'view' as const, side: null, leader: null, friendly: '80', enemy: '20', share_bp: share });
  const tile: AirTimelineReply = { ...reply(0), firstHour: 0, lastHour: 95, days: [23, 47, 71, 95], viewpoint: null,
    rows: [{ region: 5, name: 'Southern England', nameMissing: null, pinned: false, cells: [[4, view(8000)]] }], absent: [[4, null]] };
  const project = (change: number) => {
    const now = { ...reply(100), firstHour: 96, lastHour: 100, days: [100], rows: [{ ...reply(100).rows[0]!, cells: [[1, view(8000)]] as AirTimelineReply['rows'][number]['cells'] }] };
    const readView = createBoundedReadView({ hour: 100, panels: new Map<string, unknown>([['now', now], ['tile:0', tile]]), identity: colourAt(change) as never });
    return projectAirTimeline(readView, { viewpointCountry: 'GER', historyStart: 0, tiles: 1, surface: SURFACE }).rows[0]!.runs.map(run => run.firstHour);
  };
  assert.deepEqual(project(48), [0, 48, 96]);
  assert.deepEqual(project(72), [0, 72, 96], 'the old boundary is not kept');
});

test('an empty timeline says why: the chart left its active regions out, the hour was read in part, or nothing was active', () => {
  const body = (empty: AirTimelineReply['empty'], rankMissing: string | null = null) => {
    const model = projectAirTimeline(view(50, reply(50, { rows: [], empty, rankMissing })), OPTIONS);
    return [model.empty, model.emptyKind ?? 'empty'];
  };
  assert.deepEqual(body({ kind: 'filtered', hidden: 1, outside: 0 }), ['The only active region at this date is removed from this chart.', 'empty']);
  assert.deepEqual(body({ kind: 'unread', reason: 'Air scores were read only in part at this hour, so the most active regions are not known' }), ['Air scores were read only in part at this hour, so the most active regions are not known.', 'unavailable']);
  assert.deepEqual(body({ kind: 'none' }), ['No air activity at this date.', 'empty']);
  const pinnedOnly = projectAirTimeline(view(50, reply(50, { rankMissing: 'Air scores were read only in part at this hour, so the most active regions are not known' })), OPTIONS);
  assert.ok(pinnedOnly.warnings?.includes('Air scores were read only in part at this hour, so the most active regions are not known'), 'beside the picks, an unranked hour is said, not hidden');
});

test('a hundred played hours share the air history and grow only the last run', () => {
  const data = reply(240, { dayHours: 120, days: [119, null, 240] });
  const at = (hour: number) => projectAirTimeline(view(hour, { ...structuredClone(data), lastHour: hour, days: [119, null, hour] }), OPTIONS);
  assertRunCounts(240, hour => at(hour).rows[0]!.runs);
  const prior = at(275);
  const corrected = reply(275, { dayHours: 120, days: [119, null, 275], rows: [{ ...data.rows[0]!, cells: [[3, null]] }] });
  assert.equal(projectAirTimeline(view(275, corrected), OPTIONS).inspect!(0, 270)?.missingReason, 'Not recorded on this day');
  assert.equal(prior.inspect!(0, 270)?.items?.[0]?.label, 'Contested');
});
