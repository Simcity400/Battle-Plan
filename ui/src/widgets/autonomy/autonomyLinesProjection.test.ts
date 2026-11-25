import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { createBoundedReadView, createBoundedSlice, displayedHour } from '../../families/boundedView.ts';
import { compileFollowPlan } from '../../families/broker.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
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
    assert.deepEqual(main.top, { n: 1500 });
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
      { firstHour: 100, lastHour: 300, firstValue: 0.4, lastValue: 0.82, lastValueHour: 300, minValue: 0.4, minHour: 100, maxValue: 0.82, maxHour: 300, firstBreak: true, hasBreak: false },
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
