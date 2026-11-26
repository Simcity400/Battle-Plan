import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { createBoundedReadView, createBoundedSlice, displayedHour } from '../../families/boundedView.ts';
import { hoverAsks, readPlanExact } from '../../families/broker.ts';
import { planFamilySources } from '../../families/host/planSources.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/plan.ts';
import { ReadTarget, unitEntries } from '../../families/readUnits.ts';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import type { EquipmentProducedRaceReply, ProducedTypesReply } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';
import {
  equipmentProducedLinesDeclaration,
  equipmentProducedLinesProjection,
  type EquipmentProducedLinesOptions,
} from './equipmentProducedLinesProjection.ts';

function fakeIdentity(names: Record<string, { name: string; color: string; tag?: string }>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = kind === 'country' ? names[key] : undefined;
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: hit?.tag ?? key },
        name: hit ? { status: 'ready', value: hit.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: hit ? { status: 'ready', value: hit.color } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

const AUX: ProducedTypesReply = {
  branchId: '1',
  mostDelivered: 'infantry_equipment',
  types: [{ key: 'infantry_equipment', name: 'Infantry Equipment', domain: 'land' }],
  warnings: [],
};

function makeCtx(overrides: {
  options?: Record<string, unknown>;
  top?: number | null;
  tags?: string[];
  time?: WidgetSpec['time'];
  staticSpan?: readonly [number, number] | null;
} = {}): FamilyPlanContext<ProducedTypesReply> {
  const track = {
    id: 't1',
    label: 'Campaign',
    source: { id: 's1', revision: 'r1' },
    branch: { id: '1', firstHour: 100, lastHour: 600, gaps: [] },
    axisOffset: 0,
  } as unknown as BoundTrack;
  const top = overrides.top === undefined ? 10 : overrides.top;
  const tags = overrides.tags ?? [];
  const time = overrides.time ?? { mode: 'follow' as const };
  return {
    track,
    branch: track.branch,
    lastRecordedHour: 600,
    widget: {
      id: 'w1',
      kind: 'equipment-produced',
      view: 'lines',
      trackIds: ['t1'],
      time,
      countries: top !== null ? { kind: 'group', group: { type: 'top', count: top } } : { kind: 'selected', countries: tags },
      options: overrides.options ?? {},
    } as unknown as WidgetSpec,
    options: (overrides.options ?? {}) as FamilyPlanContext['options'],
    following: time.mode === 'follow',
    aux: AUX,
    countryTag: null,
    countrySubject: null,
    countryName: '',
    countries: {
      all: false,
      tags,
      holders: null,
      every: tags,
      top,
      membership: null,
      membershipRuns: null,
      unobserved: [],
      filter: { passing: null, unrecorded: [], bounds: [] },
      factions: null,
      factionUnobserved: [],
      factionIncomplete: [],
      rankedAt: () => tags,
      reason: null,
      known: () => true,
      unobservedIn: () => false,
    },
    gateSpan: (a, b) => ({ firstHour: a, lastHour: b } as never),
    staticSpan: overrides.staticSpan ?? null,
  };
}

test('equipment-produced lines registers in TIME_CHART_VIEWS and threads Range fromHour through shared planning', () => {
  const entry = TIME_CHART_VIEWS.find(v => v.kind === 'equipment-produced' && v.view === 'lines');
  assert.ok(entry, 'equipment-produced/lines registered in TIME_CHART_VIEWS');

  const followPlan = equipmentProducedLinesDeclaration.sources(makeCtx({ options: { measure: 'day', count: 'units' } }), { following: true, staticSpan: null });
  const main = followPlan.entries.find(e => e.name === 'main')?.read;
  assert.equal(main?.shape, 'pixel-binned');
  if (main?.shape === 'pixel-binned') {
    assert.equal(main.family.kind, 'equipment-produced-lines');
    assert.deepEqual(main.family.types, ['infantry_equipment']);
    assert.equal(main.family.fromHour, undefined);
  }
  const displayed = () => [countryKey(7), countryKey(8)];
  assert.equal(hoverAsks(followPlan, null, displayed).length, 0);
  const asks = hoverAsks(followPlan, 295, displayed);
  assert.deepEqual(asks.map(ask => ask.request.params), [{
    branchId: '1', count: 'units', measure: 'day', types: ['infantry_equipment'], refits: false,
    models: true, countries: [7, 8], hour: 295,
  }]);
  assert.equal(hoverAsks(followPlan, 295, displayed, asks), asks);
  assert.equal(hoverAsks(followPlan, 296, displayed, asks).length, 1);
  assert.equal(hoverAsks(followPlan, null, displayed, asks).length, 0);

  const baseCtx = makeCtx({
    top: null,
    tags: [countryKey(7)],
    options: { measure: 'total', count: 'ic', refits: true },
    time: { mode: 'range', firstOffset: 100, lastOffset: 300 },
  });
  const { staticSpan: _drop, ...withoutSpan } = baseCtx;
  const ranged = planFamilySources(
    withoutSpan,
    { firstHour: 200, lastHour: 400 },
    true,
    equipmentProducedLinesDeclaration.sources,
    { following: false },
  );
  const rangedMain = ranged.plan?.entries.find(e => e.name === 'main')?.read;
  const rangedHead = ranged.plan?.entries.find(e => e.name === 'head')?.read;
  const rangedHover = ranged.plan?.entries.find(e => e.name === 'hover')?.read;
  assert.equal(rangedMain?.shape === 'pixel-binned' ? rangedMain.family.fromHour : undefined, 200);
  assert.equal(rangedHead?.shape === 'point' ? rangedHead.params?.fromHour : undefined, 200);
  assert.equal(rangedHover?.shape === 'exact-hover' ? rangedHover.params?.fromHour : undefined, 200);
  assert.equal(rangedHover?.shape === 'exact-hover' ? rangedHover.params?.refits : undefined, true);
});

test('equipment-produced lines keys by slot, keeps missing readings with their reason, and scopes hover detail', () => {
  const ger = countryKey(7);
  const ita = countryKey(8);
  const head: EquipmentProducedRaceReply = {
    branchId: '1',
    hour: 300,
    firstHour: 290,
    lastHour: 313,
    count: 'units',
    measure: 'day',
    decimals: 0,
    types: ['infantry_equipment'],
    rows: [
      { country: 7, counted: 7, tag: 'GER', value: 420, unknown: null },
      { country: 8, counted: 8, tag: 'ITA', value: null, unknown: 'windowIncomplete' },
    ],
    readHour: 290,
    observationHour: 100,
    missing: null,
    warnings: [],
  };
  const gerTile = {
    tag: ger,
    from: 100,
    binHours: 1,
    decimals: 0,
    buckets: createBoundedSlice([
      { firstHour: 290, lastHour: 300, firstValue: 420, lastValue: 420, lastValueHour: 300, minValue: 420, minHour: 290, maxValue: 420, maxHour: 300, firstBreak: true, hasBreak: false },
    ], 1),
    straddling: 'none' as const,
    missing: createBoundedSlice([], 0),
    validInterval: [100, 300] as const,
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    series: new Map([['main', new Map([[ger, gerTile]])]]),
    panels: new Map([['head', head]]),
    hovers: new Map([['hover', { hour: 295, data: { ...head, hour: 295, rows: [
      { ...head.rows[0]!, value: 5, breakdown: { models: [{ name: 'Model A', value: 3 }, { name: 'Model B', value: 2 }], more: 0, remainder: 0 } },
      head.rows[1]!,
    ] } }]]),
    identity: fakeIdentity({
      [ger]: { name: 'German Reich', color: '#6b7280', tag: 'GER' },
      [ita]: { name: 'Italy', color: '#22c55e', tag: 'ITA' },
    }),
  });
  const opts: EquipmentProducedLinesOptions = {
    tags: [ger, ita],
    top: null,
    count: 'units',
    measure: 'day',
    equipmentLabel: 'Infantry Equipment',
    noDataMessage: null,
    fromHour: null,
    holders: null,
    membership: { [ger]: [[100, 600]], [ita]: [[296, 600]] },
    endLabels: true,
    settings: { grow: 'full', log: false, yMin: null, yMax: null } as unknown as EquipmentProducedLinesOptions['settings'],
    extent: [100, 600],
    gaps: [],
    surface: '#111111',
  };
  const model = equipmentProducedLinesProjection.project(view, opts);
  assert.equal(model.series.length, 2);
  assert.equal(model.series[0]!.key, ger);
  assert.equal(model.series[0]!.headValue, 420);
  assert.equal(model.series[0]!.endLabel?.name, 'German Reich');
  assert.equal(model.series[1]!.key, ita);
  assert.equal(model.series[1]!.headValue, null, 'unknown reading must never become zero');
  assert.match(model.series[1]!.absentReason ?? '', /before this country/);
  assert.equal(model.series[1]!.endLabel, null);
  assert.equal(model.summaryText, 'Infantry Equipment · per day', 'selection line moves beside the title');
  assert.equal(model.note, undefined, 'selection line never uses a body note');
  assert.deepEqual(model.hoverDetail?.items?.map(item => item.label), ['German Reich'], 'hover excludes non-member ITA at hour 295');
  const tip = deriveTimeChartTooltip(model, layoutTimeChart(model, 600, 260, 'sans-serif', text => text.length * 7), `line:${ger}@295`);
  assert.deepEqual(tip?.payload?.items?.map(item => [item.label, item.value]), [
    ['German Reich', '5'], ['Model A', '3'], ['Model B', '2'],
  ]);

  const icHead = { ...head, count: 'ic' as const, decimals: 5, rows: [{ country: 7, counted: 7, tag: 'GER', value: 123456, unknown: null }] };
  const ic = equipmentProducedLinesProjection.project(createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['head', icHead]]),
    hovers: new Map([['hover', { hour: 295, data: icHead }]]),
  }), { ...opts, tags: [ger], count: 'ic', membership: null });
  assert.equal(ic.hoverDetail?.items?.[0]?.value, '1.23456', 'inspection carries the recorded five decimals of IC');
  const missing = equipmentProducedLinesProjection.project(createBoundedReadView({
    hour: displayedHour(300), panels: new Map([['head', head]]),
    hovers: new Map([['hover', { hour: 295, data: { ...head, rows: [
      { ...head.rows[0]!, value: 0, breakdown: { reason: 'observationRestarted' } },
    ] } }]]),
  }), opts).hoverDetail?.itemsBySeries?.get(ger);
  assert.equal(missing?.length, 1);
  assert.equal(missing[0]!.value, undefined);
  assert.equal(missing[0]!.missingReason, 'Production counting began again inside the window.');
  const byTag = equipmentProducedLinesProjection.project(createBoundedReadView({ hour: displayedHour(300), panels: new Map([['head', head]]) }), { ...opts, tags: ['GER'], membership: null });
  assert.deepEqual(byTag.series.map(s => [s.id, s.headValue]), [['GER', 420]], 'a country chosen by its tag is one line, joined to its head row by tag');
  const renamed = { ...head, rows: [
    { country: 7, counted: 7, tag: 'DEU', value: 8, unknown: null },
    { country: 7, counted: 'DEU', tag: 'DEU', value: null, unknown: 'windowIncomplete' },
  ] };
  const mixed = equipmentProducedLinesProjection.project(createBoundedReadView({
    hour: displayedHour(300), panels: new Map([['head', renamed]]), hovers: new Map([['hover', { hour: 300, data: renamed }]]),
  }), { ...opts, tags: [ger, 'DEU'], membership: null });
  assert.deepEqual(mixed.series.map(s => s.headValue), [8, null]);
  assert.equal(mixed.hoverDetail?.itemsBySeries?.get(ger)?.[0]?.value, '8');
  assert.match(mixed.hoverDetail?.itemsBySeries?.get('DEU')?.[0]?.missingReason ?? '', /before this country/);
  assert.deepEqual(ic.auditRows?.map(row => row.value), ['1.23456'], 'and so does the audit');
});

test('the exact audit reads the dated head, so a correct picture has an exact read to agree with', async () => {
  const asked: string[] = [];
  const client = { product: async (request: { product: string }) => { asked.push(request.product); return { data: {} }; } } as never;
  const plan = equipmentProducedLinesDeclaration.sources(makeCtx({ options: { measure: 'day', count: 'units' } }), { following: false, staticSpan: null });
  const exact = await readPlanExact(plan, 300, { client, following: false, timeMode: 'pin', branch: { firstHour: 100, lastHour: 600 }, window: null, staticSpan: null, gate: { recorded: (h: number) => h, span: (a: number, b: number) => ({ firstHour: a, lastHour: b }) } as never });
  assert.ok(exact, 'an exact read exists');
  assert.deepEqual(asked, ['equipment-produced-race']);
});

test('a quiet stretch of played hours makes zero extra equipment-produced-race requests', async () => {
  const plan = equipmentProducedLinesDeclaration.sources(makeCtx({ options: { measure: 'day', count: 'units' } }), { following: true, staticSpan: null });
  const entries = unitEntries(plan, true, []);
  let currentHour = 295;
  const listeners = new Set<() => void>();
  let requests = 0;
  const reply: EquipmentProducedRaceReply = {
    branchId: '1',
    hour: 295,
    firstHour: 290,
    lastHour: 340,
    count: 'units',
    measure: 'day',
    decimals: 0,
    types: ['infantry_equipment'],
    rows: [{ country: 7, counted: 7, tag: 'GER', value: 420, unknown: null }],
    readHour: 290,
    observationHour: 100,
    missing: null,
    warnings: [],
  };
  const target = new ReadTarget(
    {
      now: () => currentHour,
      playing: () => true,
      seek: () => 0,
      subscribe: cb => { listeners.add(cb); return () => { listeners.delete(cb); }; },
      watch: () => () => {},
    },
    () => {
      requests++;
      return Promise.resolve(reply);
    },
    () => undefined,
  );
  const branch = { id: '1', firstHour: 100, lastHour: 600, gaps: [] } as never;
  const gate = { open: true, recorded: (h: number) => h, span: (a: number, b: number) => ({ firstHour: a, lastHour: b }), coverageSpan: () => [100, 600] as const } as never;
  target.configure({
    inputs: 'produced-quiet',
    entries,
    active: true,
    content: 0,
    trackOffset: 0,
    following: true,
    subscribed: true,
    playback: () => ({ branch, effectiveEndHour: 600, gate, countriesKnown: () => true }),
  });
  await Promise.resolve();
  await Promise.resolve();
  const afterInitial = requests;
  for (let h = 296; h <= 335; h++) {
    currentHour = h;
    for (const cb of listeners) cb();
  }
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(requests, afterInitial, '40 quiet played hours within [290, 340] make zero extra equipment-produced-race requests');
  target.dispose();
});
