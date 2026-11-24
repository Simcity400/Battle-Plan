import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { AirSide, AirTimelineReply } from '../../generated/dashboard';
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

const view = (hour: number, data: AirTimelineReply) => createBoundedReadView({ hour, panels: new Map([['timeline', data]]) });
const OPTIONS = { viewpoint: false, surface: SURFACE };

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
  const items = model.rows[0]!.runs[0]!.tooltip!.items!;
  assert.notEqual(items[0]!.swatch, items[1]!.swatch);
});

test('an unnamed side says its name was not recorded, never a later name', () => {
  const model = projectAirTimeline(view(50, reply(50)), OPTIONS);
  const last = model.rows[0]!.runs.at(-1)!;
  assert.equal(last.tooltip!.items!.find(item => item.label === 'Name not recorded')?.value, '50%');
  assert.deepEqual(last.tooltip!.items![0], { label: 'Contested' });
});

test('a day the recording holds no hour of is the hatch, and the lane ends at the displayed hour', () => {
  const model = projectAirTimeline(view(50, reply(50)), OPTIONS);
  const runs = model.rows[0]!.runs;
  assert.deepEqual(runs[1]!.cell, { kind: 'missing', reason: 'Not recorded on this day' });
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
