import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import type {
  BranchToHourSourceRead,
  ExactHoverSourceRead,
  FamilyPlanContext,
  IdentityProvider,
  PixelBinnedSourceRead,
} from '../../families/plan.ts';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import type { ArmyBreakdownReply, ForceAtReply } from '../../generated/dashboard';
import { FAMILY_VIEWS } from '../familyManifest.ts';
import { armySizeBreakdownDeclaration, armySizeBreakdownProjection } from './armySizeBreakdownProjection.ts';
import { armyBreakdownProof, armyGroupLabel, validateArmyBreakdownReply } from './armySizeMeta.ts';

function fakeIdentity(): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const slot7 = countryKey(7);
      const isAlpha = key === slot7 || key === 'AAA';
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: isAlpha ? (atHour >= 120 ? 'ZZZ' : 'AAA') : key },
        name: { status: 'ready', value: isAlpha ? (atHour >= 120 ? 'Zeta' : 'Alpha') : 'Beta' },
        colour: { status: 'ready', value: '#38bdf8' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

test('army-size breakdown manifest entry, slot-based plan, causal hour params, and nextChange proof', async () => {
  const entry = FAMILY_VIEWS.find(item => item.kind === 'army-size' && item.view === 'breakdown');
  assert.equal(entry?.family, 'time-chart');
  assert.equal(entry?.viewOption, 'view');
  assert.deepEqual(entry?.options?.map(field => field.key), ['view', 'measure', 'countFor', 'yMin', 'yMax', 'log']);
  const spec = await entry!.loadSpec();
  assert.equal(spec.projection, armySizeBreakdownProjection);

  const slotKey = countryKey(7);
  const ctx = {
    track: { source: { id: 's1', revision: 'r1' }, axisOffset: 0 },
    branch: { id: '1', firstHour: 100, lastHour: 200 },
    countryTag: 'ZZZ',
    countrySubject: slotKey,
    options: { measure: '', countFor: 'owner' },
    countries: { all: false, top: null, tags: ['ZZZ'], holders: null },
    aux: { branchId: '1', available: true, measures: ['manpower', 'divisions', 'strength', 'battalions'], warnings: [] },
  } as unknown as FamilyPlanContext<{ branchId: string; available: boolean; measures: ('manpower' | 'divisions' | 'strength' | 'battalions')[]; warnings: string[] }>;

  const plan = armySizeBreakdownDeclaration.sources(ctx, { width: 600, height: 260, following: false, staticSpan: [100, 140] });
  const breakdownRead = plan.entries.find(e => e.name === 'breakdown')?.read as BranchToHourSourceRead;
  const hoverRead = plan.entries.find(e => e.name === 'hover')?.read as ExactHoverSourceRead;
  assert.equal(breakdownRead.product, 'army-breakdown');
  assert.deepEqual(breakdownRead.dynamicKeys, ['hour']);
  assert.deepEqual(
    breakdownRead.buildSpanParams!(breakdownRead.params!, { firstHour: 100, lastHour: 119 } as Parameters<NonNullable<BranchToHourSourceRead['buildSpanParams']>>[1]),
    { branchId: '1', country: 7, countFor: 'owner', hour: 119 },
    'asks by recorded slot and bounds history to requested hour',
  );
  assert.deepEqual(hoverRead.params, { branchId: '1', countries: [7], countFor: 'owner' }, 'exact hover asks by recorded slot rather than reused tag');

  const manpowerPlan = armySizeBreakdownDeclaration.sources(
    { ...ctx, options: { measure: 'manpower', countFor: 'host' } } as unknown as typeof ctx,
    { width: 600, height: 260, following: false, staticSpan: [100, 140] },
  );
  const tiled = manpowerPlan.entries.find(e => e.name === 'main')?.read as PixelBinnedSourceRead;
  assert.deepEqual(tiled.tags, [slotKey]);
  assert.deepEqual(tiled.family, { kind: 'army-totals', measure: 'hostManpower' });
});

test('army-size breakdown projects stacked unit groups, distinct unresolved keys, unread mix coverage, and earlier-hour exact hover', () => {
  const reply: ArmyBreakdownReply = {
    hour: 135,
    nextChange: 140,
    available: true,
    types: [
      { key: 'group:alpha_inf', name: 'Alpha Line' },
      { key: 'unknown:mod_unit_a', name: 'Unit group name unavailable' },
      { key: 'unknown:mod_unit_b', name: 'Unit group name unavailable' },
    ],
    runs: [
      [100, 3, 119, '3'],
      [120, 8, 135, '8'],
    ],
    mix: [
      { firstHour: 100, lastHour: 119, values: [2, 1, 0], rawValues: ['2', '1', '0'] },
      { firstHour: 120, lastHour: 124, values: [2, 1, 4], rawValues: ['2', '1', '4'] },
      { firstHour: 130, lastHour: 135, values: [2, 1, 4], rawValues: ['2', '1', '4'] },
    ],
    missing: [
      { firstHour: 125, lastHour: 129, reason: 'Battalion unit groups were not recorded at these hours' },
    ],
    warnings: [],
  };
  assert.equal(validateArmyBreakdownReply(reply), reply);
  assert.deepEqual(armyBreakdownProof(reply, [100, 135], 138), [135, 139]);
  assert.equal(armyBreakdownProof(reply, [100, 135], 140), null);
  assert.throws(() => validateArmyBreakdownReply({ ...reply, hour: 120 }));

  const earlierHover: ForceAtReply = {
    parts: [
      { key: 'group:alpha_inf', name: 'Alpha Line', unresolved: false },
      { key: 'unknown:mod_unit_a', name: 'Unit group name unavailable', unresolved: true },
    ],
    rows: [{
      country: 7,
      slot: 7,
      tag: 'AAA',
      total: 3,
      values: [2, 1],
      missing: null,
      partsMissing: null,
    }],
    missing: null,
    warnings: [],
  };

  const view = createBoundedReadView({
    hour: displayedHour(135),
    panels: new Map<string, unknown>([['breakdown', reply]]),
    hovers: new Map([['hover', { hour: 115, data: earlierHover }]]),
    identity: fakeIdentity(),
  });

  const opts = {
    subject: countryKey(7),
    measure: 'battalions' as const,
    unavailableReason: null,
    countFor: 'owner' as const,
    extent: [100, 140] as const,
    gaps: [],
    surface: '#161616',
    yMin: null,
    yMax: null,
    log: false,
    auxWarnings: [],
  };

  assert.equal(armySizeBreakdownProjection.nextChange?.(view, opts), 140);
  const model = armySizeBreakdownProjection.project(view, opts);
  assert.equal(model.mode, 'stack');
  assert.equal(model.stack?.step, true);
  assert.deepEqual(
    model.stack?.bands.map(b => [b.id, b.name]),
    [
      ['unknown:mod_unit_b', 'mod_unit_b (unknown unit group)'],
      ['group:alpha_inf', 'Alpha Line'],
      ['unknown:mod_unit_a', 'mod_unit_a (unknown unit group)'],
      ['__unattributed__', 'Unattributed'],
    ],
    'distinct unresolved unit keys keep their recorded key and reason plus residual unattributed band',
  );
  const gapIndex = model.stack!.hours.indexOf(125);
  assert.ok(gapIndex >= 0);
  assert.equal(model.stack?.bands[0]!.values[gapIndex], null, 'unread mix window breaks stack rather than fabricating zeros');
  assert.ok(model.gaps?.some(g => g.firstHour === 125 && g.lastHour === 129));
  assert.deepEqual(
    model.auditRows?.map(r => [r.key, r.value]),
    [['__total__', '8'], ['unknown:mod_unit_b', '4'], ['group:alpha_inf', '2'], ['unknown:mod_unit_a', '1'], ['__unattributed__', '1']],
  );

  const layout = layoutTimeChart(model, 600, 260, 'sans-serif', t => t.length * 6);
  const tip = deriveTimeChartTooltip(model, layout, 'plot:115');
  assert.ok(tip?.payload?.items?.some(i => i.label === 'Battalions' && i.value === '3'), 'hover shows hour 115 total 3 rather than playhead total 8');
  assert.ok(tip?.payload?.items?.some(i => i.label === 'mod_unit_a (unknown unit group)' && i.value === '1'));
  assert.ok(!tip?.payload?.items?.some(i => i.label.includes('mod_unit_b')), 'hover at hour 115 omits hour 120 unit group');

  const partialHover: ForceAtReply = {
    parts: [],
    rows: [{ country: 7, slot: 7, tag: 'ZZZ', total: 8, values: [], missing: null, partsMissing: 'Battalion unit groups were not recorded at these hours' }],
    missing: null,
    warnings: [],
  };
  const partialView = createBoundedReadView({
    hour: displayedHour(135),
    panels: new Map<string, unknown>([['breakdown', reply]]),
    hovers: new Map([['hover', { hour: 127, data: partialHover }]]),
    identity: fakeIdentity(),
  });
  const partialModel = armySizeBreakdownProjection.project(partialView, opts);
  assert.ok(partialModel.hoverDetail?.items.some(i => i.label === 'Battalions' && i.value === '8'), 'witnessed total is preserved when mix is unread');
  assert.ok(partialModel.hoverDetail?.items.some(i => i.label === 'Unit groups' && i.missingReason === 'Battalion unit groups were not recorded at these hours'));
});

test('army-size breakdown reports unavailable for unrecorded head or explicitly chosen unavailable measure and formats unresolved groups distinctly', () => {
  assert.equal(armyGroupLabel({ key: 'group:mod_arm', name: 'Unit group name unavailable' }), 'mod_arm (name unavailable)');
  assert.equal(armyGroupLabel({ key: 'unknown:mod_cav', name: 'Unit group name unavailable' }), 'mod_cav (unknown unit group)');

  const unreadReply: ArmyBreakdownReply = {
    hour: 125,
    nextChange: 130,
    available: true,
    types: [],
    runs: [],
    mix: [],
    missing: [{ firstHour: 125, lastHour: 125, reason: 'Battalions were not recorded at these hours' }],
    warnings: [],
  };
  const unreadModel = armySizeBreakdownProjection.project(
    createBoundedReadView({ hour: displayedHour(125), panels: new Map<string, unknown>([['breakdown', unreadReply]]), identity: fakeIdentity() }),
    {
      subject: countryKey(7),
      measure: 'battalions',
      unavailableReason: null,
      countFor: 'owner',
      extent: [100, 140],
      gaps: [],
      surface: '#161616',
      yMin: null,
      yMax: null,
      log: false,
      auxWarnings: [],
    },
  );
  assert.equal(unreadModel.empty?.kind, 'unavailable');
  assert.equal(unreadModel.empty?.message, 'Battalions were not recorded at these hours');
  assert.deepEqual(unreadModel.auditRows, []);

  const refusedMeasure = armySizeBreakdownProjection.project(
    createBoundedReadView({ hour: displayedHour(125), panels: new Map(), identity: fakeIdentity() }),
    {
      subject: countryKey(7),
      measure: null,
      unavailableReason: 'Not in this recording',
      countFor: 'owner',
      extent: [100, 140],
      gaps: [],
      surface: '#161616',
      yMin: null,
      yMax: null,
      log: false,
      auxWarnings: [],
    },
  );
  assert.equal(refusedMeasure.empty?.kind, 'unavailable');
  assert.equal(refusedMeasure.empty?.message, 'Not in this recording');
});
