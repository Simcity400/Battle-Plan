import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveLanesTooltip } from '../../families/heat/lanesModel.ts';
import type { AirSide, AirTimelineReply } from '../../generated/dashboard';
import { askUnits } from '../../families/readUnits.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import type { FrozenGate } from '../../families/sources.ts';
import { AIR_TILE_HOURS, airTimelineDeclaration, projectAirTimeline } from './airTimelineProjection.ts';

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
        [1, { kind: 'sides', parts: [{ side: 0, tag: 'GER', friendly: '71' }, { side: 1, tag: 'ENG', friendly: '29' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false }],
        [1, null],
        [1, { kind: 'sides', parts: [{ side: 2, tag: 'ITA', friendly: '50' }, { side: 1, tag: 'ENG', friendly: '50' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false }],
      ],
    }],
    scale: 100000,
    badBelow: 0,
    goodAbove: 0,
    warnings: [],
    ...extra,
  };
}

const view = (hour: number, data: AirTimelineReply) => createBoundedReadView({ hour, panels: new Map([['now', data]]) });
const OPTIONS = { viewpointCountry: null, tiles: 0, surface: SURFACE };

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
  const contested = (side: number, tag: string) => ({ kind: 'sides' as const, parts: [{ side, tag, friendly: '50' }, { side: 1, tag: 'ENG', friendly: '50' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false });
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
});

test('a reply that reaches past the displayed hour draws nothing from it', () => {
  const model = projectAirTimeline(view(40, reply(50)), OPTIONS);
  assert.deepEqual(model.rows, []);
});

test('a day with a side whose score is unknown is not recorded, never a full lead for the known side', () => {
  const data = reply(50);
  const row = { ...data.rows[0]!, cells: [[3, { kind: 'sides', parts: [{ side: 0, tag: 'GER', friendly: '71' }], viewpoints: [], unknown: [1], noAirObject: [], filtered: false }]] as AirTimelineReply['rows'][number]['cells'] };
  const model = projectAirTimeline(view(50, { ...data, rows: [row] }), OPTIONS);
  assert.deepEqual(model.rows[0]!.runs[0]!.cell, { kind: 'missing', reason: 'A side’s air score was not recorded' });
});

test('a hundred played hours ask each completed tile once, the current tile once a day and today alone each hour, and read the history once', () => {
  const end = 24 * 3000;
  const branch = { id: '1', firstHour: 0, lastHour: end };
  const ctx = { track: { source: { id: 'src', revision: 'rev', metadata: {} }, axisOffset: 0 }, branch, options: {}, countryTag: 'GER' } as unknown as FamilyPlanContext;
  const plan = airTimelineDeclaration.sources(ctx, { following: true, staticSpan: null });
  const gate: FrozenGate = {
    recorded: (hour: number | null | undefined) => (typeof hour === 'number' ? hour : null) as never,
    span: (firstHour: number, lastHour: number) => (firstHour <= lastHour ? { firstHour, lastHour } : null) as never,
  };
  const start = 24 * 1000 + 5;
  const keys = new Map<string, string | null>();
  let tileAsks = 0;
  let nowAsks = 0;
  let widest = 0;
  for (let hour = start; hour < start + 100; hour++) {
    const units = askUnits(plan.entries, { following: true, trackOffset: 0, content: 0, hour, playback: { branch, effectiveEndHour: end, gate } });
    for (const unit of units) {
      if (unit.key !== null && unit.key !== keys.get(unit.name)) {
        if (unit.name === 'now') {
          nowAsks++;
          const asked = (unit.spec!.request as unknown as { params: { firstHour: number; lastHour: number } }).params;
          widest = Math.max(widest, asked.lastHour - asked.firstHour + 1);
        } else tileAsks++;
      }
      keys.set(unit.name, unit.key);
    }
  }
  const completedTiles = Math.ceil(24 * 1000 / AIR_TILE_HOURS);
  assert.equal(tileAsks, completedTiles + 4, 'every tile up to today once, then the current tile at each of the four midnights');
  assert.equal(nowAsks, 100);
  assert.ok(widest <= 24, 'the hourly ask covers today only');

  const visits = { cells: 0 };
  const counted = <T>(items: T[]): T[] => new Proxy(items, {
    get(target, key, receiver) {
      if (typeof key === 'string' && /^\d+$/.test(key)) visits.cells++;
      return Reflect.get(target, key, receiver);
    },
  });
  const sides = reply(0).sides;
  const cell = (friendly: string) => ({ kind: 'sides' as const, parts: [{ side: 0, tag: 'GER', friendly }, { side: 1, tag: 'ENG', friendly: '10' }], viewpoints: [], unknown: [], noAirObject: [], filtered: false });
  const tileDays = AIR_TILE_HOURS / 24;
  const tiles = Array.from({ length: completedTiles - 1 }, (_, k): AirTimelineReply => ({
    ...reply(0),
    firstHour: k * AIR_TILE_HOURS,
    lastHour: (k + 1) * AIR_TILE_HOURS - 1,
    days: Array.from({ length: tileDays }, (_, day) => k * AIR_TILE_HOURS + day * 24 + 23),
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
