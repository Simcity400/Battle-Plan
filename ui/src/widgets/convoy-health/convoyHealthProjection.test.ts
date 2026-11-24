import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { ConvoyHealthReply } from '../../generated/dashboard';
import { efficiencyColour, projectConvoyHealth } from './convoyHealthProjection.ts';

const SURFACE = '#161616';

function reply(lastHour: number): ConvoyHealthReply {
  return {
    branchId: '1', view: 'regions', tag: 'GER', firstHour: 0, lastHour, validThrough: lastHour, scale: 100000,
    known: [[{ firstHour: 0, lastHour }], [], []], unheld: [], gaps: [{ firstHour: 30, lastHour: 39 }],
    regions: [{
      region: 17, name: 'English Channel', missing: null, pinned: false, efficiency: [],
      cells: [
        { firstHour: 0, lastHour: 9, state: { kind: 'efficiency', value: 100000n } },
        { firstHour: 10, lastHour: 19, state: { kind: 'efficiency', value: 62000n } },
        { firstHour: 20, lastHour: 29, state: { kind: 'noTraffic' } },
        { firstHour: 30, lastHour: 39, state: { kind: 'notRecorded', reason: 'collector queue overflow' } },
        { firstHour: 40, lastHour, state: { kind: 'efficiency', value: 38000n } },
      ],
      readout: { kind: 'efficiency', value: 38000n },
    }],
    deals: [], leases: [], resources: [], warnings: [],
  };
}

const view = (hour: number, data: ConvoyHealthReply) => createBoundedReadView({ hour, panels: new Map([['now', data]]) });
const OPTIONS = { view: 'regions' as const, surface: SURFACE, country: null, tiles: 0 };

test('the efficiency scale runs from full at the first stop to empty at the last', () => {
  assert.equal(efficiencyColour(1), '#22364f');
  assert.equal(efficiencyColour(0), '#ff6b74');
  assert.notEqual(efficiencyColour(0.62), efficiencyColour(0.86));
});

test('no traffic is the empty lane, unrecorded hours keep their reason, and the readout is the latest efficiency', () => {
  const model = projectConvoyHealth(view(50, reply(50)), OPTIONS);
  const runs = model.rows[0]!.runs;
  assert.equal(runs[2]!.cell.kind, 'clear');
  assert.deepEqual(runs[3]!.cell, { kind: 'missing', reason: 'collector queue overflow' });
  const readout = model.rows[0]!.readout!;
  assert.equal(readout.kind === 'value' && readout.primary, '38%');
  assert.equal(readout.kind === 'value' && readout.secondary, undefined, 'the column shows the figure only');
  assert.equal(readout.kind === 'value' && readout.meaning, 'Efficiency', 'what it is goes to the tooltip, mirror and audit');
  assert.equal(readout.kind === 'value' && readout.tone, undefined, 'a damaged route is emphasised');
  const full = { ...reply(50), regions: [{ ...reply(50).regions[0]!, readout: { kind: 'efficiency' as const, value: 100000n } }] };
  const quiet = projectConvoyHealth(view(50, full), OPTIONS).rows[0]!.readout!;
  assert.equal(quiet.kind === 'value' && quiet.tone, 'muted', 'a full route is quiet');
  assert.equal(runs.at(-1)!.lastHour, 50);
  assert.equal(model.rows[0]!.region, 17, 'each lane names its region for the regions control');
  assert.deepEqual(model.regions, { pickedHeading: 'Your regions', automaticHeading: 'Most active', addLabel: 'Add region' });
});

test('a reply that reaches past the displayed hour, or for the other view, draws nothing', () => {
  assert.deepEqual(projectConvoyHealth(view(45, reply(50)), OPTIONS).rows, []);
  assert.deepEqual(projectConvoyHealth(view(50, reply(50)), { view: 'trade', surface: SURFACE, country: null, tiles: 0 }).rows, []);
});

test('a lend-lease route stays a status in the lane, the readout and the tooltip, never an efficiency', async () => {
  const { deriveLanesTooltip } = await import('../../families/heat/lanesModel.ts');
  const data: ConvoyHealthReply = {
    ...reply(50), view: 'trade', regions: [], gaps: [],
    known: [[], [], [{ firstHour: 0, lastHour: 50 }]],
    leases: [{ handle: '7', giver: [{ firstHour: 0, lastHour: 50, tag: 'USA', name: 'United States' }], bad: [[0, 19, 1], [20, 50, 0]], convoys: [] }],
  };
  const model = projectConvoyHealth(view(50, data), { view: 'trade', surface: SURFACE, country: null, tiles: 0 });
  const [bad, clear] = model.rows[0]!.runs;
  assert.equal(bad!.cell.kind === 'solid' && bad!.cell.label, 'Route bad');
  assert.deepEqual(clear!.cell, { kind: 'clear', label: 'Route clear' });
  assert.equal(model.rows[0]!.readout?.kind === 'value' && model.rows[0]!.readout.primary, 'Route clear');
  const tip = deriveLanesTooltip(model, 0, 10)!;
  assert.equal(tip.items?.[0]?.label, 'Route bad');
  assert.ok(!tip.items?.some(item => item.label === 'Efficiency'), 'no measured efficiency is claimed');
});

test('a hundred played hours ask each completed tile once, the current tile once a day and today alone each hour, never the whole history', async () => {
  const { askUnits } = await import('../../families/readUnits.ts');
  const { DAY_TILE_HOURS } = await import('../../families/heat/dayTiles.ts');
  const { convoyHealthDeclaration } = await import('./convoyHealthProjection.ts');
  const end = 24 * 3000;
  const branch = { id: '1', firstHour: 0, lastHour: end };
  const gate = {
    recorded: (hour: number | null | undefined) => (typeof hour === 'number' ? hour : null) as never,
    span: (firstHour: number, lastHour: number) => (firstHour <= lastHour ? { firstHour, lastHour } : null) as never,
  };
  const start = 24 * 1000 + 5;
  for (const view of ['regions', 'trade']) {
    const ctx = { track: { source: { id: 'src', revision: 'rev', metadata: {} }, axisOffset: 0 }, branch, options: { view }, countryTag: 'GER' } as never;
    const plan = convoyHealthDeclaration.sources(ctx, { following: true, staticSpan: null } as never);
    const keys = new Map<string, string | null>();
    let tileAsks = 0;
    let nowAsks = 0;
    let widestNow = 0;
    let widest = 0;
    for (let hour = start; hour < start + 100; hour++) {
      const units = askUnits(plan.entries, { following: true, trackOffset: 0, content: 0, hour, playback: () => ({ branch, effectiveEndHour: end, gate }) });
      for (const unit of units) {
        if (unit.key !== null && unit.key !== keys.get(unit.name)) {
          const asked = (unit.spec!.request as unknown as { params: { from: number; to: number } }).params;
          const width = asked.to - asked.from + 1;
          widest = Math.max(widest, width);
          if (unit.name === 'now') {
            nowAsks++;
            widestNow = Math.max(widestNow, width);
          } else tileAsks++;
        }
        keys.set(unit.name, unit.key);
      }
    }
    assert.ok(widest <= DAY_TILE_HOURS, `${view}: no ask covers more than one tile`);
    assert.equal(tileAsks, Math.ceil(24 * 1000 / DAY_TILE_HOURS) + 4, `${view}: every tile up to today once, then the current tile at each of the four midnights`);
    assert.ok(nowAsks <= 100 && widestNow <= 24, `${view}: the hourly ask covers today only`);
    const now = plan.entries.find(entry => entry.name === 'now')!.read;
    assert.ok(now.shape === 'branch-to-hour');
    assert.deepEqual(now.provenInterval!({ ...reply(25), validThrough: 30 }, [24, 25], 25), [25, 30], `${view}: a reply proves the hours through its validThrough`);
  }
});

test('inside its proof the today reply reaches the displayed hour, and history comes from the tiles, read once', async () => {
  const { deriveLanesTooltip } = await import('../../families/heat/lanesModel.ts');
  const efficiency = (value: bigint) => ({ kind: 'efficiency' as const, value });
  const tile: ConvoyHealthReply = {
    ...reply(47), gaps: [], validThrough: 47,
    regions: [{ region: 17, name: 'English Channel', missing: null, pinned: false, efficiency: [], cells: [{ firstHour: 0, lastHour: 47, state: efficiency(50000n) }] }],
    absent: [{ firstHour: 0, lastHour: 47, state: { kind: 'noTraffic' } }],
  };
  const cells = { region: 17, name: 'English Channel', missing: null, pinned: false, efficiency: [], cells: [{ firstHour: 48, lastHour: 50, state: efficiency(90000n) }], readout: efficiency(90000n) };
  const now: ConvoyHealthReply = {
    ...reply(50), firstHour: 48, gaps: [], validThrough: 60,
    regions: [cells, { ...cells, region: 18, name: 'North Sea' }],
  };
  let reads = 0;
  const counted = new Proxy(tile.regions, { get: (target, key, receiver) => { if (key === 'map') reads++; return Reflect.get(target, key, receiver); } });
  const tiled = { ...tile, regions: counted };
  const at = (hour: number) => projectConvoyHealth(createBoundedReadView({ hour, panels: new Map<string, unknown>([['now', now], ['tile:0', tiled]]) }), { ...OPTIONS, tiles: 1 });
  const model = at(55);
  assert.equal(model.lastHour, 55, 'proven through hour 60, the lanes reach the displayed hour');
  assert.equal(model.firstHour, 0);
  assert.deepEqual(model.rows[0]!.runs.map(run => [run.firstHour, run.lastHour]), [[0, 47], [48, 55]]);
  assert.equal(model.rows[1]!.runs[0]!.cell.kind, 'clear', 'a region the tile does not hold draws the tile’s absent cells');
  assert.equal(deriveLanesTooltip(model, 0, 20)!.items![0]!.value, '50%', 'a past hour is answered by its tile');
  assert.equal(deriveLanesTooltip(model, 0, 54)!.items![0]!.value, '90%', 'an hour inside the proof is answered by the today reply');
  at(56);
  at(57);
  assert.equal(reads, 1, 'a tile is laid out once for every hour it serves');
  assert.equal(at(70).lastHour, 60, 'never past the proof');
});

test('a trade row is every deal of the span, its history from the tiles and its name from the latest reply that holds one', () => {
  const deal = (efficiency: [number, number, number][], exporter: ConvoyHealthReply['deals'][number]['exporter']) => ({ id: 'a1', exporter, resource: [], efficiency, factors: [], convoys: [] });
  const tile: ConvoyHealthReply = {
    ...reply(47), view: 'trade', regions: [], gaps: [], known: [[], [{ firstHour: 0, lastHour: 47 }], []],
    deals: [deal([[0, 47, 80000]], [{ firstHour: 0, lastHour: 47, tag: 'SWE', name: 'Sweden' }])],
  };
  const now: ConvoyHealthReply = { ...reply(50), view: 'trade', firstHour: 48, regions: [], gaps: [], known: [[], [{ firstHour: 48, lastHour: 50 }], []], deals: [deal([], [])] };
  const model = projectConvoyHealth(createBoundedReadView({ hour: 50, panels: new Map<string, unknown>([['now', now], ['tile:0', tile]]) }), { view: 'trade', surface: SURFACE, country: null, tiles: 1 });
  assert.equal(model.rows.length, 1);
  assert.equal(model.rows[0]!.label, 'Sweden · Resource not recorded');
  assert.deepEqual(model.rows[0]!.runs.map(run => [run.firstHour, run.lastHour]), [[0, 47]]);
  assert.deepEqual(model.rows[0]!.readout, { kind: 'clear', label: 'No deal' }, 'a deal with no run today has none at the displayed hour');
});
