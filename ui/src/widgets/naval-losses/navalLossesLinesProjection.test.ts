import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardSource } from '../../dashboard/client';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, displayedHour } from '../../families/boundedView.ts';
import { hoverAsks, readPlanExact } from '../../families/broker.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
import { ReadTarget, type TargetClock, type TargetConfig } from '../../families/readUnits.ts';
import type { FrozenGate } from '../../families/sources.ts';
import { DEFAULT_LINE_SETTINGS } from '../../families/time-chart/settings.ts';
import type { NavalLossesAtReply } from '../../generated/dashboard';
import { navalLossesAtProof } from './navalLossesAt.ts';
import {
  navalLossesLinesDeclaration,
  navalLossesLinesProjection,
  type NavalLossesLinesOptions,
} from './navalLossesLinesProjection.ts';

const SOURCE = {
  id: 'src',
  revision: 'rev1',
  name: 'Test',
  firstHour: 0,
  lastHour: 500,
  branches: [{ id: '1', firstHour: 0, lastHour: 500 }],
} as unknown as DashboardSource;

const GATE: FrozenGate = {
  recorded: (hour: number | null | undefined) => (typeof hour === 'number' ? hour : null) as never,
  span: (firstHour: number, lastHour: number) => (firstHour <= lastHour ? { firstHour, lastHour } : null) as never,
};

function makeCtx(overrides: Partial<FamilyPlanContext> = {}): FamilyPlanContext {
  return {
    track: { id: 't1', source: SOURCE, branch: { id: '1', firstHour: 0, lastHour: 500, gaps: [] }, axisOffset: 0 } as never,
    branch: { id: '1', firstHour: 0, lastHour: 500, gaps: [] } as never,
    lastRecordedHour: 500,
    widget: {
      id: 'w1',
      kind: 'naval-losses',
      trackIds: ['t1'],
      countries: { kind: 'group', group: { type: 'top', count: 5 }, membership: 'hour', aggregate: false },
      options: { view: 'lines', kind: 'both', attribution: 'lost', period: 'running' },
      time: { mode: 'follow' },
    } as never,
    options: { view: 'lines', kind: 'both', attribution: 'lost', period: 'running', endLabels: true },
    following: true,
    aux: undefined,
    countryTag: null,
    countrySubject: null,
    countryName: '',
    countries: {
      all: false,
      tags: [],
      holders: null,
      every: [countryKey(1), countryKey(2), countryKey(3)],
      top: 5,
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
    gateSpan: (first, last) => ({ firstHour: first, lastHour: last }) as never,
    staticSpan: null,
    ...overrides,
  };
}

function makeOptions(overrides: Partial<NavalLossesLinesOptions> = {}): NavalLossesLinesOptions {
  return {
    tags: [],
    top: 5,
    kind: 'both',
    attribution: 'lost',
    period: 'running',
    measureTitle: 'Ships and convoys lost',
    holders: null,
    membership: null,
    endLabels: true,
    settings: DEFAULT_LINE_SETTINGS,
    extent: [0, 500],
    gaps: [],
    surface: '#111111',
    ...overrides,
  };
}

function fakeIdentity(names: Readonly<Record<string, { readonly name: string; readonly colour: string }>>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = kind === 'country' ? names[key] : undefined;
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: hit ? { status: 'ready', value: hit.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: hit ? { status: 'ready', value: hit.colour } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

function navalReply(overrides: Partial<NavalLossesAtReply> = {}): NavalLossesAtReply {
  return {
    firstHour: 100,
    lastHour: 250,
    parts: [
      { key: 'ships', name: 'Ships', unresolved: false },
      { key: 'convoys', name: 'Convoys', unresolved: false },
    ],
    rows: [
      {
        country: 1,
        slot: 1,
        tag: 'GER',
        total: 42,
        referenceTotal: null,
        missing: null,
        values: [12, 30],
        partsMissing: null,
      },
      {
        country: 2,
        slot: 2,
        tag: 'ENG',
        total: 18,
        referenceTotal: null,
        missing: null,
        values: [5, 13],
        partsMissing: null,
      },
    ],
    missing: null,
    warnings: [],
    ...overrides,
  };
}

test('naval losses lines projects slot-keyed series, whole-number axis, and end labels', () => {
  const k1 = countryKey(1);
  const k2 = countryKey(2);
  const identity = fakeIdentity({
    [k1]: { name: 'German Reich', colour: '#4c566a' },
    [k2]: { name: 'United Kingdom', colour: '#bf616a' },
  });
  const reason = 'One ship sinking could not be counted.';
  const reply = navalReply();
  reply.rows[0] = { ...reply.rows[0]!, partial: reason, partsPartial: [reason, null] };
  const readView = createBoundedReadView({
    hour: displayedHour(150),
    identity,
    panels: new Map([['head', reply]]),
    hovers: new Map([['hover', { hour: 150, data: reply }]]),
  });
  const model = navalLossesLinesProjection.project(readView, makeOptions());
  assert.equal(model.yAxis.decimals, 0);
  assert.equal(model.series.length, 2);
  assert.equal(model.series[0]!.id, k1);
  assert.equal(model.series[0]!.headValue, 42);
  assert.equal(model.series[0]!.endLabel?.name, 'German Reich');
  assert.equal(model.series[0]!.endLabel?.value, '42 (partial)');
  assert.deepEqual(model.series[0]!.partialSpans, [{ firstHour: 150, lastHour: 150, reason }]);
  assert.equal(model.auditRows?.[0]?.value, `42, partial: ${reason}`);
  assert.deepEqual(model.hoverDetail?.items.slice(0, 3).map(item => [item.value, item.partialReason]), [
    ['42', reason], ['12', reason], ['30', undefined],
  ]);
  assert.equal(model.series[1]!.id, k2);
  assert.equal(model.series[1]!.endLabel?.value, '18');
});

test('unrecorded naval losses stay not recorded with their reason, never zero or blank', () => {
  const readView = createBoundedReadView({
    hour: displayedHour(120),
    panels: new Map([['head', navalReply({ missing: 'Naval losses are not known at this hour', rows: [] })]]),
  });
  const model = navalLossesLinesProjection.project(readView, makeOptions());
  assert.equal(model.series.length, 0);
  assert.equal(model.empty?.kind, 'unavailable');
  assert.equal(model.empty?.message, 'Naval losses are not known at this hour');
  assert.deepEqual(model.auditRows?.map(r => r.value), ['Naval losses are not known at this hour']);
});

test('hover detail scopes requests and rows to dated eligible slots and preserves absent countries as missing', () => {
  const k1 = countryKey(1);
  const k2 = countryKey(2);
  const ctx = makeCtx({
    widget: {
      id: 'w1',
      kind: 'naval-losses',
      trackIds: ['t1'],
      countries: { kind: 'tags', tags: [k1, k2] },
      options: { view: 'lines', kind: 'both', attribution: 'lost', period: 'running' },
      time: { mode: 'follow' },
    } as never,
    countries: {
      ...makeCtx().countries,
      top: null,
      tags: [k1, k2],
      membership: {
        [k1]: [[100, 300]],
        [k2]: [[200, 300]],
      },
    },
  });
  const plan = navalLossesLinesDeclaration.sources(ctx, { following: true, staticSpan: null });
  const asksAt150 = hoverAsks(plan, 150);
  assert.equal(asksAt150.length, 1);
  assert.deepEqual((asksAt150[0]!.request.params as { countries: unknown[] }).countries, [1]);

  const identity = fakeIdentity({
    [k1]: { name: 'German Reich', colour: '#4c566a' },
    [k2]: { name: 'United Kingdom', colour: '#bf616a' },
  });
  const readView = createBoundedReadView({
    hour: displayedHour(250),
    identity,
    panels: new Map([['head', navalReply()]]),
    hovers: new Map([['hover', {
      hour: 150,
      data: navalReply({ rows: [] }),
    }]]),
  });
  const model = navalLossesLinesProjection.project(readView, makeOptions({
    top: null,
    tags: [k1, k2],
    membership: { [k1]: [[100, 300]], [k2]: [[200, 300]] },
  }));
  assert.ok(model.hoverDetail);
  assert.equal(model.hoverDetail.hour, 150);
  assert.equal(model.hoverDetail.items.length, 1);
  assert.equal(model.hoverDetail.items[0]!.label, 'German Reich');
  assert.equal(model.hoverDetail.items[0]!.missingReason, 'This country is not present at this date.');
});

test('a country chosen by its tag is one line, joined to its head row by tag or slot, never two', () => {
  const model = navalLossesLinesProjection.project(createBoundedReadView({ hour: displayedHour(200), panels: new Map([['head', navalReply()]]) }),
    makeOptions({ top: null, tags: ['GER'], membership: null }));
  assert.deepEqual(model.series.map(s => [s.id, s.headValue]), [['GER', 42]]);
});

test('the exact audit reads the dated head, so a correct picture has an exact read to agree with', async () => {
  const asked: string[] = [];
  const client = { product: async (request: { product: string }) => { asked.push(request.product); return { data: navalReply() }; } } as never;
  const exact = await readPlanExact(navalLossesLinesDeclaration.sources(makeCtx(), { following: false, staticSpan: null }), 200,
    { client, following: false, timeMode: 'pin', branch: { firstHour: 100, lastHour: 600 }, window: null, staticSpan: null, gate: { recorded: (h: number) => h, span: (a: number, b: number) => ({ firstHour: a, lastHour: b }) } as never });
  assert.ok(exact, 'an exact read exists');
  assert.deepEqual(asked, ['naval-losses-at']);
  assert.equal(navalLossesLinesProjection.project(exact, makeOptions()).auditRows?.length, 2, 'both ranked countries, from the head');
});

test('a quiet stretch of played hours makes no extra naval-losses-at request', async () => {
  const ctx = makeCtx();
  const plan = navalLossesLinesDeclaration.sources(ctx, { following: true, staticSpan: null });
  const headEntry = plan.entries.find(e => e.name === 'head')!;
  assert.ok(headEntry);
  assert.deepEqual(navalLossesAtProof(navalReply({ firstHour: 100, lastHour: 250 }), 100), [100, 250]);

  let hour = 100;
  const watches = new Set<{ lo: number; hi: number; left: () => void }>();
  const clock: TargetClock = {
    now: () => hour,
    playing: () => true,
    seek: () => 0,
    subscribe: () => () => {},
    watch([lo, hi], left) {
      const w = { lo, hi, left };
      watches.add(w);
      return () => { watches.delete(w); };
    },
  };
  let requests = 0;
  const target = new ReadTarget(clock, async () => {
    requests++;
    return navalReply({ firstHour: 100, lastHour: 250 });
  });
  const config: TargetConfig = {
    inputs: 'naval-quiet',
    following: true,
    entries: [headEntry],
    active: true,
    content: 0,
    trackOffset: 0,
    subscribed: false,
    playback: () => ({ branch: { firstHour: 0, lastHour: 500 }, effectiveEndHour: 500, gate: GATE }),
  };
  target.configure(config);
  await new Promise<void>(resolve => setImmediate(resolve));
  assert.equal(requests, 1);

  for (let h = 101; h <= 250; h++) {
    hour = h;
    for (const w of [...watches]) {
      if (hour < w.lo || hour > w.hi) {
        watches.delete(w);
        w.left();
      }
    }
    await new Promise<void>(resolve => setImmediate(resolve));
  }
  assert.equal(requests, 1, 'no requests during the proven quiet stretch 100..250');
  target.dispose();
});
