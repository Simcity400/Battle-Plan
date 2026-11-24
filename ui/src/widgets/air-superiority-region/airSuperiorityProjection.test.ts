import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { assertBarsModelValid, deriveBarsAudit, type BarsDataRow } from '../../families/bars/model.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { AirSuperiorityReply } from '../../generated/dashboard';
import { airSuperiorityRegionProjection, type AirSuperiorityProjectionOptions } from './airSuperiorityProjection.ts';

const OPTS: AirSuperiorityProjectionOptions = {
  mode: 'sides',
  filteredSides: false,
  order: 'contested',
  enemyColor: '#e5484d',
  mutedColor: '#8a8a8a',
};

const REPLY: AirSuperiorityReply = {
  missing: null,
  unlisted: null,
  empty: null,
  activeUnlisted: { ids: [], missing: null },
  sides: [
    { key: 'f:7', faction: '7', colourFaction: '7', factions: ['7'], tag: null, leader: 'ENG', coBelligerents: 1, anchor: { tag: 'ENG', hour: 0 }, name: 'Allies', color: [34, 197, 94] },
    { key: 'f:10', faction: '10', colourFaction: '10', factions: ['10'], tag: null, leader: 'GER', coBelligerents: 0, anchor: { tag: 'GER', hour: 0 }, name: 'Axis', color: [239, 68, 68] },
  ],
  legend: [0, 1],
  viewpoint: null,
  regions: [
    {
      id: 11,
      name: 'English Channel',
      nameMissing: null,
      active: true,
      pinned: true,
      wings: 'live',
      cell: {
        kind: 'sides',
        parts: [
          { side: 0, tag: 'ENG', live: 'ENG@11' },
          { side: 1, tag: 'GER', live: 'GER@11' },
        ],
        viewpoints: [],
        unknown: [],
        noAirObject: [],
        filtered: false,
        excluded: 0,
      },
    },
    {
      id: 12,
      name: 'Northern France',
      nameMissing: null,
      active: true,
      pinned: false,
      wings: 'unknown',
      cell: {
        kind: 'sides',
        parts: [],
        viewpoints: [],
        unknown: [0],
        noAirObject: [],
        filtered: false,
        excluded: 0,
      },
    },
  ],
  fallback: { kind: 'sides', parts: [], viewpoints: [], unknown: [], noAirObject: [], filtered: false, excluded: 0 },
  scale: 100000,
  badBelow: 40000,
  goodAbove: 60000,
  warnings: [],
};

test('air superiority projection preserves Rust region order, computes shares from Rust live total, and labels air wings', () => {
  const live = valuesOf(new Map([
    [liveKey('ENG@11', 'friendly'), 4100],
    [liveKey('GER@11', 'friendly'), 5900],
    [liveKey('11', 'total'), 10000],
    [liveKey('11', 'wings'), 90],
  ]), 'Not recorded at this hour');

  const view = createBoundedReadView({
    hour: displayedHour(400),
    panels: new Map([['main', REPLY]]),
    live: new Map([['main', live]]),
  });

  const model = airSuperiorityRegionProjection.project(view, OPTS);
  assertBarsModelValid(model, 400);
  assert.equal(model.form, 'share');

  assert.deepEqual(model.rows.map(r => [r.kind, r.key]), [
    ['section', '__section:picked'], ['data', '11'], ['section', '__section:auto'], ['data', '12'], ['add-region', '__add-region'],
  ], "the user's regions first, then the most active, then Add region");
  const rows = model.rows.filter((r): r is BarsDataRow => r.kind === 'data');
  assert.deepEqual(rows.map(r => [r.key, r.region, r.group]), [['11', 11, 'picked'], ['12', 12, 'auto']]);
  assert.equal(rows[0]!.readout?.primary, '90 air wings');
  assert.deepEqual(rows[0]!.segments?.map(s => [s.label, Math.round(s.share * 100), s.inlineLabel]), [
    ['Allies', 41, 'Allies 41%'],
    ['Axis', 59, 'Axis 59%'],
  ]);

  assert.equal(rows[1]!.value?.value.kind, 'notRecorded');
  assert.equal(rows[1]!.segments?.length, 0);
  assert.equal(rows[1]!.readout?.primary, 'Wings not recorded');

  const audit = deriveBarsAudit(model);
  assert.ok(audit.some(entry => entry.row === '11' && entry.key === '11:segment:f:7'));
  assert.ok(audit.some(entry => entry.row === '12' && entry.key === '12:value' && entry.value?.startsWith('["notRecorded"')));
});

test('air superiority projection shows missing when live total is absent and rejects future hours', () => {
  const missingLive = valuesOf(new Map([[liveKey('11', 'wings'), 1]]), 'Air scores not recorded');
  const view = createBoundedReadView({
    hour: displayedHour(120),
    panels: new Map([['main', REPLY]]),
    live: new Map([['main', missingLive]]),
  });

  const model = airSuperiorityRegionProjection.project(view, OPTS);
  assertBarsModelValid(model, 120);
  const first = model.rows.find(row => row.key === '11') as BarsDataRow;
  assert.equal(first.value?.value.kind, 'notRecorded');
  assert.equal(first.readout?.primary, '1 air wing');
  assert.throws(() => view.who('country', 'ENG', displayedHour(121)), FutureDataError);
});

test('a pick outside the selected air theatres keeps its row, place and controls, saying so', () => {
  const outside: AirSuperiorityReply = { ...REPLY, regions: [
    { ...REPLY.regions[0]!, id: 19, name: 'Northern France', pinned: true, active: false, wings: 'none', cell: { kind: 'reason', reason: 'outside-theatres' } },
    REPLY.regions[1]!,
  ] };
  const view = createBoundedReadView({ hour: displayedHour(400), panels: new Map([['main', outside]]), live: new Map([['main', valuesOf(new Map(), 'Not recorded')]]) });
  const model = airSuperiorityRegionProjection.project(view, OPTS);
  assertBarsModelValid(model, 400);
  assert.deepEqual(model.rows.map(row => [row.kind, row.key]), [
    ['section', '__section:picked'], ['data', '19'], ['section', '__section:auto'], ['data', '12'], ['add-region', '__add-region'],
  ]);
  const pick = model.rows[1] as BarsDataRow;
  assert.deepEqual([pick.region, pick.group, pick.value?.value.kind], [19, 'picked', 'notApplicable']);
  assert.equal(pick.tooltip?.missingReason, 'Outside selected air theatres');
});

test('an empty comparison says whether the chart left every active region out, was read in part or found none', () => {
  const body = (empty: AirSuperiorityReply['empty']) => {
    const view = createBoundedReadView({ hour: displayedHour(400), panels: new Map([['main', { ...REPLY, regions: [], empty }]]) });
    const model = airSuperiorityRegionProjection.project(view, OPTS);
    return [model.empty, model.emptyKind];
  };
  assert.deepEqual(body({ kind: 'filtered', hidden: 1, outside: 0 }), ['The only active region at this date is removed from this chart.', 'empty'], 'hiding the only active region is not "no air activity"');
  assert.deepEqual(body({ kind: 'filtered', hidden: 2, outside: 3 }), ['All 5 active regions at this date are removed from this chart (2) or outside the selected air theatres (3).', 'empty']);
  assert.deepEqual(body({ kind: 'unread', reason: 'Air region wings read only in part at this hour' }), ['Air region wings read only in part at this hour.', 'unavailable']);
  assert.deepEqual(body({ kind: 'none' }), ['No air activity at this date.', 'empty']);
});

test('the automatic heading names the order shown: only an Active sort is "Most active"', () => {
  const heading = (order: AirSuperiorityProjectionOptions['order']) => {
    const view = createBoundedReadView({ hour: displayedHour(400), panels: new Map([['main', REPLY]]) });
    const model = airSuperiorityRegionProjection.project(view, { ...OPTS, order });
    return model.rows.find(row => row.key === '__section:auto')?.label;
  };
  assert.deepEqual([heading('active'), heading('contested'), heading('name')], ['Most active', 'Most contested', 'Active regions']);
});
