import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveLanesTooltip, readoutAuditText } from '../../families/heat/lanesModel.ts';
import type { ConvoyHealthReply } from '../../generated/dashboard';
import { convoyEfficiencyColour } from '../../charts/palette.ts';
import { projectConvoyHealth } from './convoyHealthProjection.ts';

const SURFACE = '#161616';

function reply(lastHour: number): ConvoyHealthReply {
  return {
    branchId: '1', view: 'regions', tag: 'GER', firstHour: 0, lastHour, validThrough: lastHour, scale: 100000, rankMissing: null, empty: null, activeUnlisted: { ids: [], missing: null },
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
const OPTIONS = { view: 'regions' as const, surface: SURFACE, country: null, historyStart: 0, tiles: 0 };

test('R3-S4: regional and trade convoy readouts inspect the recorded efficiency', () => {
  for (const mode of ['regions', 'trade'] as const) {
    const data = reply(50);
    data.view = mode;
    data.regions[0]!.readout = { kind: 'efficiency', value: 38246n };
    if (mode === 'trade') {
      data.regions = [];
      data.known = [[], [{ firstHour: 0, lastHour: 50 }], []];
      data.deals = [{ id: 'D01', exporter: [{ firstHour: 0, lastHour: 50, tag: 'A01', name: 'A01' }], resource: [],
        efficiency: [[0, 50, 38246]], factors: [], convoys: [] }];
    }
    const model = projectConvoyHealth(view(50, data), { ...OPTIONS, view: mode });
    const readout = model.rows[0]!.readout!;
    assert.equal(readout.kind === 'value' && readout.primary, '38%');
    assert.equal(deriveLanesTooltip(model, 0, null)!.exactValue, '38.246%');
    assert.equal(readoutAuditText(readout), '38.246% (Efficiency)');
  }
});

test('the efficiency scale runs from full at the first stop to empty at the last', () => {
  assert.equal(convoyEfficiencyColour(1, '#111111'), '#22364f');
  assert.equal(convoyEfficiencyColour(0, '#111111'), '#ff6b74');
  assert.notEqual(convoyEfficiencyColour(0.62, '#111111'), convoyEfficiencyColour(0.86, '#111111'));
  assert.notEqual(convoyEfficiencyColour(1, '#22364f'), '#22364f', 'a full cell stays visible on a surface of its own colour');
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
  assert.deepEqual(model.regions, { pickedHeading: 'Your regions', automaticHeading: 'Most active', addLabel: 'Add region', active: { ids: [], missing: null } });
});

test('a reply that reaches past the displayed hour, or for the other view, draws nothing', () => {
  assert.deepEqual(projectConvoyHealth(view(45, reply(50)), OPTIONS).rows, []);
  assert.deepEqual(projectConvoyHealth(view(50, reply(50)), { view: 'trade', surface: SURFACE, country: null, historyStart: 0, tiles: 0 }).rows, []);
});

test('a held recording gap fills the played hour in the band and tooltip', () => {
  const data = reply(10);
  data.validThrough = 15;
  data.gaps = [{ firstHour: 10, lastHour: 10 }];
  data.regions[0]!.cells = [
    { firstHour: 0, lastHour: 9, state: { kind: 'efficiency', value: 100000n } },
    { firstHour: 10, lastHour: 10, state: { kind: 'notRecorded', reason: 'Collector was interrupted' } },
  ];
  const model = projectConvoyHealth(view(12, data), OPTIONS);
  assert.deepEqual(model.gaps, [{ firstHour: 10, lastHour: 12, reason: 'The recording holds no hour here' }]);
  assert.equal(deriveLanesTooltip(model, 0, 12)?.missingReason, 'The recording holds no hour here');
});

test('a lend-lease route stays a status in the lane, the readout and the tooltip, never an efficiency', async () => {
  const { deriveLanesTooltip } = await import('../../families/heat/lanesModel.ts');
  const data: ConvoyHealthReply = {
    ...reply(50), view: 'trade', regions: [], gaps: [],
    known: [[], [], [{ firstHour: 0, lastHour: 50 }]],
    leases: [{ handle: '7', giver: [{ firstHour: 0, lastHour: 50, tag: 'USA', name: 'United States' }], bad: [[0, 19, 1], [20, 50, 0]], convoys: [] }],
  };
  const model = projectConvoyHealth(view(50, data), { view: 'trade', surface: SURFACE, country: null, historyStart: 0, tiles: 0 });
  const [bad, clear] = model.rows[0]!.runs;
  assert.equal(bad!.cell.kind === 'solid' && bad!.cell.label, 'Route bad');
  assert.deepEqual(clear!.cell, { kind: 'clear', label: 'Route clear' });
  assert.equal(model.rows[0]!.readout?.kind === 'value' && model.rows[0]!.readout.primary, 'Route clear');
  const tip = deriveLanesTooltip(model, 0, 10)!;
  assert.equal(tip.items?.[0]?.label, 'Route bad');
  assert.ok(!tip.items?.some(item => item.label === 'Efficiency'), 'no measured efficiency is claimed');
});

test('both convoy views follow every history tile and ask for the same closed days on a pin or audit', async () => {
  const { convoyHealthDeclaration } = await import('./convoyHealthProjection.ts');
  const { DAY_TILE_HOURS } = await import('../../families/heat/dayTiles.ts');
  const branch = { id: '1', firstHour: 5, lastHour: DAY_TILE_HOURS + 60 };
  for (const view of ['regions', 'trade']) {
    const ctx = { track: { source: { id: 'src' }, axisOffset: 0 }, branch, options: { view }, countryTag: 'GER' } as never;
    const plan = convoyHealthDeclaration.sources(ctx, { following: true, staticSpan: null } as never);
    assert.deepEqual(plan.entries.map(entry => entry.name), ['now', 'tile:0', 'tile:1']);
    const spans = plan.entries.map(({ name, read }) => {
      assert.ok(read.shape === 'point' && read.channel === 'panel');
      if (name === 'now') {
        assert.equal(read.followQuery?.panel, 'convoy-health');
        assert.equal(read.followQuery.listFrom, view === 'trade' ? 5 : undefined);
      } else {
        assert.equal(read.followQuery?.panel, 'convoy-health');
        const span = name === 'tile:0' ? [5, DAY_TILE_HOURS - 1] : [DAY_TILE_HOURS, DAY_TILE_HOURS + 47];
        assert.deepEqual([read.followQuery.from, read.followQuery.to], span);
      }
      const exact = (hour: number) => read.buildExactParams!(read.params!, hour as never, 'exact');
      const first = exact(10);
      const later = exact(DAY_TILE_HOURS + 25);
      return [first && [first.from, first.to], later && [later.from, later.to]];
    });
    assert.deepEqual(spans, [[[5, 10], [DAY_TILE_HOURS + 24, DAY_TILE_HOURS + 25]], [null, [5, DAY_TILE_HOURS - 1]], [null, [DAY_TILE_HOURS, DAY_TILE_HOURS + 23]]]);
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
  const model = projectConvoyHealth(createBoundedReadView({ hour: 50, panels: new Map<string, unknown>([['now', now], ['tile:0', tile]]) }), { view: 'trade', surface: SURFACE, country: null, historyStart: 0, tiles: 1 });
  assert.equal(model.rows.length, 1);
  assert.equal(model.rows[0]!.label, 'Sweden · Resource not recorded');
  assert.deepEqual(model.rows[0]!.runs.map(run => [run.firstHour, run.lastHour]), [[0, 47]]);
  assert.deepEqual(model.rows[0]!.readout, { kind: 'clear', label: 'No deal' }, 'a deal with no run today has none at the displayed hour');
});
