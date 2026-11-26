import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { createBoundedReadView, createBoundedSlice, displayedHour } from '../../families/boundedView.ts';
import { compileFollowPlan, readPlanExact } from '../../families/broker.ts';
import type { BoundedSeries, FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
import { ReadTarget, unitEntries } from '../../families/readUnits.ts';
import type { AutonomyLevelsReply, AutonomySubject } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';
import {
  autonomyLinesDeclaration,
  autonomyLinesProjection,
  type AutonomyLinesOptions,
} from './autonomyLinesProjection.ts';

function fakeIdentity(names: Readonly<Record<string, { name: string; color: string; tag: string }>>): IdentityProvider {
  return {
    resolve(_kind, key, atHour) {
      const entry = names[key];
      return {
        kind: 'country',
        key,
        atHour,
        holder: entry ? { status: 'ready', value: key } : { status: 'absent', reason: 'Missing' },
        tag: entry ? { status: 'ready', value: entry.tag } : { status: 'absent', reason: 'Missing' },
        name: entry ? { status: 'ready', value: entry.name } : { status: 'absent', reason: 'Missing' },
        colour: entry ? { status: 'ready', value: entry.color } : { status: 'absent', reason: 'Missing' },
        flagChoice: { status: 'absent', reason: 'No flag' },
        validInterval: [0, 1_000_000],
        nextChangeAt: Infinity,
      };
    },
  };
}

function makeCtx(options: Record<string, unknown> = {}): FamilyPlanContext {
  const track = {
    id: 't1',
    source: { id: 'src', revision: 'r1' },
    branch: { id: '1', firstHour: 100, lastHour: 600, gaps: [] },
    axisOffset: 0,
  } as unknown as BoundTrack;
  return {
    track,
    branch: track.branch,
    lastRecordedHour: 600,
    widget: {
      id: 'w1',
      kind: 'autonomy',
      view: 'lines',
      trackIds: ['t1'],
      time: { mode: 'follow' },
      countries: { kind: 'all' },
      options,
    } as unknown as WidgetSpec,
    options: options as FamilyPlanContext['options'],
    following: true,
    aux: undefined,
    countryTag: null,
    countrySubject: null,
    countryName: '',
    countries: {
      all: true,
      tags: [],
      holders: null,
      every: [],
      top: null,
      membership: null,
      membershipRuns: null,
      unobserved: [],
      filter: { passing: null, unrecorded: [], bounds: [] },
      factions: null,
      factionUnobserved: [],
      factionIncomplete: [],
      rankedAt: () => [],
      reason: null,
      known: () => true,
      unobservedIn: () => false,
    },
    gateSpan: (a, b) => ({ firstHour: a, lastHour: b } as never),
    staticSpan: null,
  };
}

function makeSubject(overrides: Partial<AutonomySubject> & { subjectId: number; subject: string; overlord: string }): AutonomySubject {
  return {
    overlordId: 1,
    overlordSlot: 1,
    subjectSlot: overrides.subjectId,
    level: 'dominion',
    levelBelow: 'colony',
    levelAbove: 'free',
    levelsUnknown: null,
    progressMilli: 50_000,
    progressUnknown: null,
    score: 250,
    lowerScore: 0,
    upperScore: 500,
    ...overrides,
  };
}

test('autonomy lines registers in TIME_CHART_VIEWS and uses autonomy-lines tiles + followed autonomy-levels', () => {
  const entry = TIME_CHART_VIEWS.find(v => v.kind === 'autonomy' && v.view === 'lines');
  assert.ok(entry, 'autonomy/lines registered in TIME_CHART_VIEWS');

  const plan = autonomyLinesDeclaration.sources(makeCtx({ countriesAre: 'overlords', grow: true }), { following: true, staticSpan: null });
  const main = plan.entries.find(e => e.name === 'main')?.read;
  const head = plan.entries.find(e => e.name === 'head')?.read;
  const hover = plan.entries.find(e => e.name === 'hover')?.read;
  assert.equal(main?.shape, 'pixel-binned');
  if (main?.shape === 'pixel-binned') {
    assert.equal(main.family.kind, 'autonomy-lines');
    assert.ok((main.top?.n ?? 0) > 1500, 'every subject the recording holds, with no cap below the population');
    assert.equal(main.headQuery, null);
  }
  assert.equal(head?.shape, 'point');
  if (head?.shape === 'point') {
    assert.equal(head.product, 'autonomy-levels');
    assert.deepEqual(head.followQuery, { panel: 'autonomy-levels', branchId: '1' });
  }
  assert.equal(hover?.shape, 'exact-hover');
  if (hover?.shape === 'exact-hover') {
    assert.equal(hover.product, 'autonomy-levels');
  }
});

test('autonomy lines keys by slot, preserves missing progress with reason, filters by overlord/subject, and scopes hover to hover hour', () => {
  const canKey = countryKey(11);
  const rajKey = countryKey(12);
  const sloKey = countryKey(13);
  const engKey = countryKey(1);
  const gerKey = countryKey(2);

  const reply: AutonomyLevelsReply = {
    levels: [
      { token: 'puppet', name: 'Puppet', ordered: true },
      { token: 'colony', name: 'Colony', ordered: true },
      { token: 'dominion', name: 'Dominion', ordered: true },
      { token: 'free', name: 'Free', ordered: true },
    ],
    subjects: [
      makeSubject({ overlordId: 1, overlordSlot: 1, overlord: 'ENG', subjectId: 11, subjectSlot: 11, subject: 'CAN', progressMilli: 82_000 }),
      makeSubject({ overlordId: 1, overlordSlot: 1, overlord: 'ENG', subjectId: 12, subjectSlot: 12, subject: 'RAJ', progressMilli: null, progressUnknown: 'No autonomy range', score: undefined, lowerScore: undefined, upperScore: undefined }),
      makeSubject({ overlordId: 2, overlordSlot: 2, overlord: 'GER', subjectId: 13, subjectSlot: 13, subject: 'SLO', level: 'puppet', progressMilli: 15_000 }),
    ],
    missing: null,
    warnings: [],
  };

  const canTile = {
    tag: canKey,
    from: 100,
    binHours: 1,
    decimals: 5,
    buckets: createBoundedSlice([
      { firstHour: 200, lastHour: 300, firstValue: 0.4, lastValue: 0.82, lastValueHour: 300, minValue: 0.4, minHour: 200, maxValue: 0.82, maxHour: 300, firstBreak: true, hasBreak: false },
    ], 1),
    straddling: 'none' as const,
    missing: createBoundedSlice([], 0),
    validInterval: [100, 300] as const,
  };

  const view = createBoundedReadView({
    hour: displayedHour(300),
    series: new Map([['main', new Map([[canKey, canTile]])]]),
    panels: new Map([['head', reply]]),
    hovers: new Map([['hover', { hour: 150, data: reply }]]),
    identity: fakeIdentity({
      [engKey]: { name: 'United Kingdom', color: '#e11d48', tag: 'ENG' },
      [gerKey]: { name: 'German Reich', color: '#64748b', tag: 'GER' },
      [canKey]: { name: 'Dominion of Canada', color: '#ef4444', tag: 'CAN' },
      [rajKey]: { name: 'British Raj', color: '#f97316', tag: 'RAJ' },
      [sloKey]: { name: 'Slovakia', color: '#3b82f6', tag: 'SLO' },
    }),
  });

  const ukFilter: AutonomyLinesOptions = {
    filter: {
      passing: { ENG: [[200, 600]] },
      unrecorded: [],
      bounds: [200, 601],
    },
    filterAll: false,
    bySubject: false,
    grow: true,
    holders: null,
    extent: [100, 600],
    gaps: [],
    surface: '#111111',
  };

  const model = autonomyLinesProjection.project(view, ukFilter);
  assert.deepEqual(model.view?.x, [100, 300], 'grow option limits x view to displayed hour');
  assert.equal(model.series.length, 2, 'only UK subjects shown at hour 300');
  assert.equal(model.series[0]!.key, canKey);
  assert.equal(model.series[0]!.valueScale, 100);
  assert.equal(model.series[0]!.headValue, 82);
  assert.equal(model.series[0]!.endLabel?.name, 'Dominion of Canada');
  assert.equal(model.series[0]!.endLabel?.value, '82%');
  assert.equal(model.series[1]!.key, rajKey);
  assert.equal(model.series[1]!.headValue, null, 'missing progress must never become zero');
  assert.equal(model.series[1]!.absentReason, 'No autonomy range');
  assert.equal(model.series[1]!.endLabel, null);
  assert.equal(model.hoverDetail, null, 'hover at hour 150 excludes ENG subjects because ENG only passes from 200');

  const hoverInRange = createBoundedReadView({
    hour: displayedHour(300),
    series: new Map([['main', new Map([[canKey, canTile]])]]),
    panels: new Map([['head', reply]]),
    hovers: new Map([['hover', { hour: 250, data: reply }]]),
    identity: fakeIdentity({
      [engKey]: { name: 'United Kingdom', color: '#e11d48', tag: 'ENG' },
      [gerKey]: { name: 'German Reich', color: '#64748b', tag: 'GER' },
      [canKey]: { name: 'Dominion of Canada', color: '#ef4444', tag: 'CAN' },
      [rajKey]: { name: 'British Raj', color: '#f97316', tag: 'RAJ' },
      [sloKey]: { name: 'Slovakia', color: '#3b82f6', tag: 'SLO' },
    }),
  });
  const hovered = autonomyLinesProjection.project(hoverInRange, ukFilter);
  assert.ok(hovered.hoverDetail?.items?.some(item => item.label === 'Dominion of Canada' && item.value === '82%'));
  assert.ok(hovered.hoverDetail?.items?.some(item => item.label === 'British Raj' && item.missingReason === 'No autonomy range'));
  assert.ok(!hovered.hoverDetail?.items?.some(item => item.label === 'Slovakia'));
});

test('the exact audit reads the dated autonomy levels, so a correct picture has an exact read to agree with', async () => {
  const asked: string[] = [];
  const client = { product: async (request: { product: string }) => { asked.push(request.product); return { data: {} }; } } as never;
  const exact = await readPlanExact(autonomyLinesDeclaration.sources(makeCtx(), { following: false, staticSpan: null }), 300, { client, following: false, timeMode: 'pin', branch: { firstHour: 100, lastHour: 600 }, window: null, staticSpan: null, gate: { recorded: (h: number) => h, span: (a: number, b: number) => ({ firstHour: a, lastHour: b }) } as never });
  assert.ok(exact, 'an exact read exists');
  assert.equal(asked.length, 1);
});

test('a subject that became independent keeps its history', () => {
  const formerKey = countryKey(14);
  const tile = {
    tag: formerKey, from: 100, binHours: 1, decimals: 5,
    buckets: createBoundedSlice([{ firstHour: 100, lastHour: 150, firstValue: 0.3, lastValue: 0.6, lastValueHour: 150, minValue: 0.3, minHour: 100, maxValue: 0.6, maxHour: 150, firstBreak: true, hasBreak: false }], 1),
    straddling: 'none' as const, missing: createBoundedSlice([], 0), validInterval: [100, 300] as const,
  };
  const reply: AutonomyLevelsReply = { levels: [], subjects: [], missing: null, warnings: [] };
  const ctx = makeCtx({ countriesAre: 'overlords' });
  const model = autonomyLinesProjection.project(createBoundedReadView({ hour: displayedHour(300), series: new Map([['main', new Map([[formerKey, tile]])]]), panels: new Map([['head', reply]]) }),
    autonomyLinesDeclaration.projectionOptions(ctx, { extent: [100, 600], surface: '#111111' }));
  assert.equal(model.empty ?? null, null, 'never "No subjects at this time" while recorded history exists');
  assert.deepEqual(model.series.map(s => [s.id, s.headValue, s.endLabel]), [[formerKey, null, null]], 'drawn as history, with no head and no end label');
  assert.deepEqual(model.auditRows, [], 'a former subject has no reading at the hour to audit');
});

test('a faction-joined overlord draws subjects from the join, and the keyed subject request preserves the same dates', () => {
  const selected = (countriesAre: string) => {
    const base = makeCtx({ countriesAre });
    const ctx: FamilyPlanContext = { ...base, countries: { ...base.countries, all: false, filter: { ...base.countries.filter, passing: { AAA: [[200, 320], [400, 600]] } } } };
    const read = autonomyLinesDeclaration.sources(ctx, { following: true, staticSpan: null }).entries.find(entry => entry.name === 'main')!.read;
    assert.ok(read.shape === 'pixel-binned');
    return read;
  };
  const dates = [{ firstHour: 200, lastHour: 320 }, { firstHour: 400, lastHour: 600 }];
  assert.deepEqual(selected('overlords').family.overlords, [['AAA', dates]]);
  const subjects = selected('subjects');
  assert.deepEqual(subjects.tags, ['AAA']);
  assert.deepEqual(subjects.within?.get('AAA'), dates);
  assert.equal(subjects.top, undefined);
  const subject = makeSubject({ subject: 'BBB', subjectId: 42, overlord: 'AAA' });
  const tile = { ...emptyTile, tag: countryKey(42), missing: createBoundedSlice([{ firstHour: 200, lastHour: 210, reason: 'No progress' }], 1) };
  const gap = historyModel(tile, [subject], 260, { bySubject: false, filter: { passing: { AAA: [[200, 210], [400, 600]] }, bounds: [], unrecorded: [] } });
  assert.equal(gap.series[0]?.absentReason, 'Outside the selected countries at this date');
  assert.ok(gap.hoverDetail?.items.some(item => item.missingReason === gap.series[0]?.absentReason));
});

const emptyTile: BoundedSeries = {
  tag: 'AAA', from: 100, binHours: 1, decimals: 5, buckets: createBoundedSlice([null, null], 2),
  straddling: 'none', missing: createBoundedSlice([], 0), validInterval: [100, 300],
};

function historyModel(tile: BoundedSeries, subjects: AutonomySubject[] = [], hoverHour = 250, options?: Partial<AutonomyLinesOptions>) {
  const ctx = makeCtx({ countriesAre: 'subjects' });
  const reply: AutonomyLevelsReply = { levels: [], subjects, missing: null, warnings: [] };
  const view = createBoundedReadView({
    hour: displayedHour(300), series: new Map([['main', new Map([[tile.tag, tile]])]]),
    panels: new Map([['head', reply]]), hovers: new Map([['hover', { hour: hoverHour, data: reply }]]),
    identity: fakeIdentity({ AAA: { name: 'Alpha', tag: 'AAA', color: '#888888' }, [countryKey(41)]: { name: 'Alpha', tag: 'AAA', color: '#888888' } }),
  });
  return autonomyLinesProjection.project(view, { ...autonomyLinesDeclaration.projectionOptions(ctx, { extent: [100, 600], surface: '#111111' }),
    filterAll: false, filter: { passing: { AAA: [[100, 600]], BBB: [[100, 600]] }, unrecorded: [], bounds: [] }, ...options });
}

test('a never-subject tag draws nothing while a subject with only missing progress keeps its history', () => {
  const never = historyModel(emptyTile);
  assert.deepEqual(never.series, []);
  assert.equal(never.empty?.message, 'No matching subjects at this time.');
  const gap = { firstHour: 110, lastHour: 180, reason: 'No recorded progress range' };
  const former = historyModel({ ...emptyTile, missing: createBoundedSlice([gap], 1) });
  assert.deepEqual(former.series.map(series => series.missingSpans), [[gap]]);
  assert.ok(former.hoverDetail?.items.some(item => item.label === 'Alpha' && item.missingReason));
});

test('the exact hover lists a former subject beside a current subject and its peak counts toward the axis', () => {
  const bucket = { firstHour: 120, lastHour: 160, firstValue: 0.8, lastValue: 1.35, lastValueHour: 160, minValue: 0.8, minHour: 120, maxValue: 1.35, maxHour: 160, firstBreak: true, hasBreak: false };
  const tile = { ...emptyTile, buckets: createBoundedSlice([bucket], 1) };
  const model = historyModel(tile, [makeSubject({ subject: 'BBB', subjectId: 42, overlord: 'CCC' })]);
  assert.ok(model.hoverDetail?.items?.some(item => item.value === '50%'));
  assert.ok(model.hoverDetail?.items.some(item => item.label === 'Alpha' && item.swatch && item.missingReason === 'Not a subject at this date'));
  assert.deepEqual(model.yAxis.fixed, [0, 150]);
  assert.equal(historyModel(tile, [], 110).hoverDetail, null, 'the first relationship is still in the future at the hovered hour');
  const selectedAgain = historyModel(tile, [makeSubject({ subject: 'AAA', subjectId: 41, overlord: 'CCC' })], 250,
    { filter: { passing: { AAA: [[100, 160], [300, 600]] }, unrecorded: [], bounds: [] } });
  assert.ok(selectedAgain.hoverDetail?.items.some(item => item.label === 'Alpha' && item.missingReason === 'Outside the selected countries at this date'
    && item.swatch === selectedAgain.series[0]!.color));
});

test('a quiet stretch of played hours makes zero extra requests for autonomy lines', async () => {
  const plan = autonomyLinesDeclaration.sources(makeCtx(), { following: true, staticSpan: null });
  const compiled = compileFollowPlan(plan, query => ({ query: query as never, refused: null }));
  assert.equal(compiled.subscriptions.length, 1);
  assert.equal(compiled.subscriptions[0]!.name, 'head');
  assert.equal('hour' in compiled.subscriptions[0]!.query, false, 'followed autonomy-levels query names no hour');

  const entries = unitEntries(plan, true, compiled.subscriptions);
  let currentHour = 200;
  const listeners = new Set<() => void>();
  let requests = 0;
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
      return Promise.resolve({});
    },
    () => undefined,
  );
  const branch = { id: '1', firstHour: 100, lastHour: 600, gaps: [] } as never;
  const gate = { open: true, recorded: (h: number) => h, span: (a: number, b: number) => ({ firstHour: a, lastHour: b }), coverageSpan: () => [100, 600] as const } as never;
  target.configure({
    inputs: 'autonomy-quiet',
    entries,
    active: true,
    content: 0,
    trackOffset: 0,
    following: true,
    subscribed: true,
    playback: () => ({ branch, effectiveEndHour: 600, gate, countriesKnown: () => true }),
  });
  await Promise.resolve();
  for (let h = 201; h <= 230; h++) {
    currentHour = h;
    for (const cb of listeners) cb();
  }
  await Promise.resolve();
  assert.equal(requests, 0, '30 quiet played hours make zero extra requests');
  target.dispose();
});
