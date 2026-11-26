import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { tilesOf } from '../../dashboard/tiles/tile.ts';
import { createBoundedReadView, createBoundedSlice, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
import { compileFollowPlan, readPlanExact } from '../../families/broker.ts';
import { ReadTarget, unitEntries } from '../../families/readUnits.ts';
import type { FactionTotalsAtReply } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';
import { resolveWidgetScopedOptions } from '../manifest.ts';
import { withFactionTotalsStyles } from '../../families/time-chart/factionTotalsLines.ts';
import { factionStrengthLinesDeclaration, factionStrengthLinesProjection } from './factionStrengthLinesProjection.ts';

function fakeIdentity(names: Readonly<Record<string, { readonly name: string; readonly color: string }>>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const hit = names[`${kind}:${key}`];
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: hit ? { status: 'ready', value: hit.name } : { status: 'absent', reason: 'Name not recorded' },
        colour: hit ? { status: 'ready', value: hit.color } : { status: 'absent', reason: 'Colour not recorded' },
        flagChoice: { status: 'absent', reason: 'Flag not recorded' },
        validInterval: [0, 1000],
        nextChangeAt: Infinity,
      };
    },
  };
}

function makeCtx(overrides: {
  readonly options?: Record<string, unknown>;
  readonly countries?: unknown;
  readonly factions?: readonly { readonly id: string; readonly tag: string; readonly firstHour: number; readonly lastHour: number }[];
  readonly tags?: readonly string[];
}): FamilyPlanContext {
  const tags = overrides.tags ?? [];
  return {
    track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
    branch: { id: '1', firstHour: 100, lastHour: 600, gaps: [] },
    options: overrides.options ?? {},
    countries: {
      all: false,
      top: null,
      every: tags,
      tags,
      holders: null,
      membership: null,
      unobserved: [],
      factions: overrides.factions ?? null,
      reason: null,
    },
    following: true,
    aux: undefined,
    widget: {
      id: 'w',
      kind: 'faction-strength',
      trackIds: ['t'],
      countries: overrides.countries ?? { kind: 'factions', count: Number.MAX_SAFE_INTEGER, membership: 'hour' },
      time: { mode: 'follow' },
      options: overrides.options ?? {},
    },
  } as unknown as FamilyPlanContext;
}

test('faction-strength lines registers in TIME_CHART_VIEWS and uses pixel-binned faction-totals tiles with series-state follow', () => {
  const entry = TIME_CHART_VIEWS.find(item => item.kind === 'faction-strength' && item.view === 'lines');
  assert.ok(entry);
  assert.equal(entry.family, 'time-chart');

  const keys = resolveWidgetScopedOptions('faction-strength', { view: 'lines' }).map(field => field.key);
  for (const key of ['view', 'measure', 'count', 'territoryMeasure', 'rule', 'endLabels', 'playback', 'yMin', 'yMax', 'log']) {
    assert.ok(keys.includes(key), `lines settings include ${key}`);
  }

  const plan = factionStrengthLinesDeclaration.sources(
    makeCtx({
      options: { measure: 'factories', count: 'built' },
      factions: [
        { id: '10', tag: 'GER', firstHour: 100, lastHour: 600 },
        { id: '20', tag: 'ENG', firstHour: 100, lastHour: 600 },
      ],
    }),
    { following: true, staticSpan: null },
  );
  const main = plan.entries.find(e => e.name === 'main');
  assert.ok(main && main.read.shape === 'pixel-binned');
  assert.deepEqual(main.read.family, { kind: 'faction-totals', measure: 'built7' });
  assert.deepEqual(main.read.tags, ['10', '20']);
  assert.ok(main.read.headQuery && (main.read.headQuery as { product?: string }).product === 'series-state');
  const hover = plan.entries.find(e => e.name === 'hover');
  assert.ok(hover && hover.read.shape === 'exact-hover' && hover.read.product === 'faction-totals-at');

  const aggPlan = factionStrengthLinesDeclaration.sources(
    makeCtx({
      options: { measure: 'territory', territoryMeasure: 'states', rule: 'controlled' },
      countries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: true },
      tags: ['GER'],
    }),
    { following: true, staticSpan: null },
  );
  const aggMain = aggPlan.entries.find(e => e.name === 'main');
  assert.ok(aggMain && aggMain.read.shape === 'pixel-binned');
  assert.deepEqual(aggMain.read.family, {
    kind: 'faction-group-lines',
    sourceKind: 'territory',
    measure: 'controlledStates',
    group: { type: 'humans', membership: 'hour' },
  });
  const aggHead = aggPlan.entries.find(e => e.name === 'head');
  assert.ok(aggHead && aggHead.read.shape === 'point' && aggHead.read.product === 'faction-totals-at');
  assert.ok(aggHead.read.followQuery && (aggHead.read.followQuery as { panel?: string }).panel === 'faction-totals-at');
  assert.equal(aggPlan.entries.some(e => e.read.shape === 'branch-to-hour'), false);
});

test('aggregated explicit country list asks one group tile line keyed as group and decodes the single group reply', () => {
  const ger = countryKey(7);
  const ita = countryKey(9);
  const plan = factionStrengthLinesDeclaration.sources(
    makeCtx({
      options: { measure: 'factories', count: 'free' },
      countries: { kind: 'group', group: { type: 'top', count: 2 }, membership: 'hour', aggregate: true },
      tags: [ger, ita],
    }),
    { following: true, staticSpan: null },
  );
  const main = plan.entries.find(e => e.name === 'main');
  assert.ok(main && main.read.shape === 'pixel-binned');
  assert.deepEqual(main.read.tags, ['group']);
  assert.deepEqual(main.read.coverage, { mode: 'keyed', keys: ['group'] });
  assert.deepEqual(main.read.family, {
    kind: 'faction-group-lines',
    sourceKind: 'country-factories',
    measure: 'free7',
    members: [7, 9],
  });
  const reply = {
    branchId: '1',
    measure: 'free7',
    decimals: 0,
    binHours: 1,
    index: 0,
    firstHour: 0,
    immutable: false,
    filled: 115,
    rawValues: true,
    hourBytes: 0,
    lines: [{ country: 'group', missing: [], parts: [] }],
    bins: new Uint8Array(256),
  };
  const decoded = tilesOf(reply, main.read.tags, 1, 0, tag => tag);
  assert.equal(decoded.length, 1);
  assert.equal(decoded[0]!.tag, 'group');
});

test('faction-strength lines hides future factions, keeps recorded zeros and missing totals, and builds exact hover details', () => {
  const hoverReply: FactionTotalsAtReply = {
    hour: 280,
    kind: 'country-factories',
    measure: 'free7',
    rows: [
      { id: '10', members: ['GER', 'ITA'], memberCountries: [7, 8], total: 410, missing: null },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    series: new Map([
      ['main', new Map([
        ['15', {
          tag: '15',
          from: 100,
          binHours: 1,
          decimals: 0,
          buckets: createBoundedSlice([], 0),
          straddling: 'none' as const,
          missing: createBoundedSlice([
            { firstHour: 100, lastHour: 199, reason: 'Before founding' },
            { firstHour: 220, lastHour: 240, reason: 'Member gap' },
          ], 2),
          validInterval: [100, 300] as const,
        }],
      ])],
    ]),
    panels: new Map([
      ['main', {
        decimals: 0,
        states: [
          { country: '10', hour: 300, run: { valueRaw: '410' }, missing: null },
          { country: '15', hour: 300, run: { valueRaw: '0' }, missing: null },
          { country: '20', hour: 300, run: null, missing: 'A member value is not recorded at this hour' },
        ],
      }],
    ]),
    hovers: new Map([['hover', { hour: displayedHour(280), data: hoverReply }]]),
    identity: fakeIdentity({
      'faction:10': { name: 'Axis', color: '#6b7280' },
      'faction:15': { name: 'Benelux', color: '#0ea5e9' },
      'faction:20': { name: 'Allies', color: '#e11d48' },
      [`country:${countryKey(7)}`]: { name: 'German Reich', color: '#6b7280' },
      [`country:${countryKey(8)}`]: { name: 'Italy', color: '#16a34a' },
    }),
  });
  const opts = factionStrengthLinesDeclaration.projectionOptions(
    makeCtx({
      options: { measure: 'factories', count: 'free', endLabels: true },
      factions: [
        { id: '10', tag: 'GER', firstHour: 100, lastHour: 600 },
        { id: '15', tag: 'LUX', firstHour: 200, lastHour: 600 },
        { id: '20', tag: 'ENG', firstHour: 100, lastHour: 600 },
        { id: '99', tag: 'JAP', firstHour: 450, lastHour: 600 },
      ],
    }),
    { extent: [100, 600], surface: '#111111' },
  );
  const model = factionStrengthLinesProjection.project(view, opts);
  assert.deepEqual(model.series.map(s => [s.id, s.name, s.headValue]), [
    ['10', 'Axis', 410],
    ['15', 'Benelux', 0],
    ['20', 'Allies', null],
  ]);
  assert.deepEqual(model.series[1]!.missingSpans, [{ firstHour: 220, lastHour: 240, reason: 'Member gap' }]);
  assert.equal(model.series[0]!.endLabel?.value, '410');
  assert.equal(model.series[1]!.endLabel?.value, '0');
  assert.equal(model.series[2]!.endLabel, null);
  assert.ok(model.auditRows?.some(row => row.key === '20' && row.value?.includes('not recorded')));
  assert.equal(model.hoverDetail?.hour, 280);
  assert.ok(model.hoverDetail?.items.some(item => item.label === 'Members' && item.value?.includes('German Reich')));
  assert.throws(() => view.who('faction', '10', displayedHour(301)), FutureDataError);
});

test('faction-strength lines applies campaign faction renames and colours to lines, end labels, and hover details', () => {
  const hoverReply: FactionTotalsAtReply = {
    hour: 280,
    kind: 'country-factories',
    measure: 'free7',
    rows: [
      { id: '10', members: ['GER'], memberCountries: [7], total: 410, missing: null },
      { id: '20', members: ['ENG'], memberCountries: [9], total: 350, missing: null },
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([
      ['main', {
        decimals: 0,
        states: [
          { country: '10', hour: 300, run: { valueRaw: '410' }, missing: null },
          { country: '20', hour: 300, run: { valueRaw: '350' }, missing: null },
        ],
      }],
    ]),
    hovers: new Map([['hover', { hour: displayedHour(280), data: hoverReply }]]),
    identity: fakeIdentity({
      'faction:10': { name: 'Axis', color: '#6b7280' },
      'faction:20': { name: 'Allies', color: '#e11d48' },
      [`country:${countryKey(7)}`]: { name: 'German Reich', color: '#6b7280' },
      [`country:${countryKey(9)}`]: { name: 'United Kingdom', color: '#e11d48' },
    }),
  });
  const styled = withFactionTotalsStyles(
    factionStrengthLinesDeclaration,
    { '10': { name: 'Berlin Pact', colour: '#ff8800' } },
    { '["name","Allies"]': { colour: '#22c55e' } },
  );
  const opts = styled.projectionOptions(
    makeCtx({
      options: { measure: 'factories', count: 'free', endLabels: true },
      factions: [
        { id: '10', tag: 'GER', firstHour: 100, lastHour: 600 },
        { id: '20', tag: 'ENG', firstHour: 100, lastHour: 600 },
      ],
    }),
    { extent: [100, 600], surface: '#111111' },
  );
  const model = factionStrengthLinesProjection.project(view, opts);
  assert.deepEqual(model.series.map(s => [s.id, s.name, s.color, s.endLabel?.name, s.endLabel?.colorBar]), [
    ['10', 'Berlin Pact', '#ff8800', 'Berlin Pact', '#ff8800'],
    ['20', 'Allies', '#22c55e', 'Allies', '#22c55e'],
  ]);
  const axisHover = model.hoverDetail?.items.find(item => item.label === 'Berlin Pact');
  const alliesHover = model.hoverDetail?.items.find(item => item.label === 'Allies');
  assert.equal(axisHover?.swatch, '#ff8800');
  assert.equal(alliesHover?.swatch, '#22c55e');
});

test('a Top 1 hover reads the plotted faction at the hovered hour, never the faction ranked first then', () => {
  const ctx = makeCtx({
    options: { measure: 'factories', count: 'free' },
    countries: { kind: 'factions', count: 1, membership: 'hour' },
    factions: [{ id: '10', tag: 'GER', firstHour: 100, lastHour: 600 }, { id: '20', tag: 'ENG', firstHour: 100, lastHour: 600 }],
  });
  const hover = factionStrengthLinesDeclaration.sources(ctx, { following: true, staticSpan: null }).entries.find(entry => entry.name === 'hover')!;
  assert.equal((hover.read as { params: { top?: number } }).params.top, undefined, 'the hover asks every faction under the plotted rule');
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', { decimals: 0, states: [
      { country: '10', hour: 300, run: { valueRaw: '410' }, missing: null },
      { country: '20', hour: 300, run: { valueRaw: '350' }, missing: null },
    ] }]]),
    hovers: new Map([['hover', { hour: displayedHour(150), data: {
      hour: 150, kind: 'country-factories', measure: 'free7', missing: null, warnings: [],
      rows: [{ id: '20', members: ['ENG'], memberCountries: [9], total: 500, missing: null }, { id: '10', members: ['GER'], memberCountries: [7], total: 300, missing: null }],
    } satisfies FactionTotalsAtReply }]]),
    identity: fakeIdentity({ 'faction:10': { name: 'Axis', color: '#6b7280' }, 'faction:20': { name: 'Allies', color: '#e11d48' } }),
  });
  const model = factionStrengthLinesProjection.project(view, factionStrengthLinesDeclaration.projectionOptions(ctx, { extent: [100, 600], surface: '#111111' }));
  assert.deepEqual(model.series.map(s => s.id), ['10']);
  assert.deepEqual(model.hoverDetail?.items.filter(item => item.value !== undefined && item.label !== 'Members').map(item => `${item.label}=${item.value}`), ['Axis=300']);
});

test('an unread faction roster says so, never "No factions", and a Top N it could change is not shown as proved', () => {
  const unread = { ...makeCtx({ options: { measure: 'factories', count: 'free' }, factions: [] }) };
  const ctx = { ...unread, countries: { ...unread.countries, factionUnobserved: [[100, 600]] } } as unknown as FamilyPlanContext;
  const model = factionStrengthLinesProjection.project(createBoundedReadView({ hour: displayedHour(300) }),
    factionStrengthLinesDeclaration.projectionOptions(ctx, { extent: [100, 600], surface: '#111111' }));
  assert.equal(model.empty?.kind, 'unavailable');
  assert.match(model.empty?.message ?? '', /not recorded/);
});

test('an aggregate group reads its dated head for the exact audit, and its hover keeps the one group row', async () => {
  const group = makeCtx({ options: { measure: 'factories', count: 'free' }, countries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: true } });
  const asked: string[] = [];
  const client = { product: async (request: { product: string }) => { asked.push(request.product); return { data: {} }; } } as never;
  const exact = await readPlanExact(factionStrengthLinesDeclaration.sources(group, { following: false, staticSpan: null }), 300,
    { client, following: false, timeMode: 'pin', branch: { firstHour: 100, lastHour: 600 }, window: null, staticSpan: null, gate: { recorded: (h: number) => h, span: (a: number, b: number) => ({ firstHour: a, lastHour: b }) } as never });
  assert.ok(exact, 'an exact read exists');
  assert.ok(asked.includes('faction-totals-at'));

  const faction = makeCtx({ options: { measure: 'factories', count: 'free' }, countries: { kind: 'group', group: { type: 'faction', id: '10' }, membership: 'hour', aggregate: true },
    factions: [{ id: '10', tag: 'GER', firstHour: 100, lastHour: 600 }] });
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', { decimals: 0, states: [{ country: '10', hour: 300, run: { valueRaw: '410' }, missing: null }] }]]),
    hovers: new Map([['hover', { hour: displayedHour(200), data: {
      hour: 200, kind: 'country-factories', measure: 'free7', missing: null, warnings: [],
      rows: [{ id: 'group', members: ['GER'], memberCountries: [7], total: 300, missing: null }],
    } satisfies FactionTotalsAtReply }]]),
    identity: fakeIdentity({}),
  });
  const model = factionStrengthLinesProjection.project(view, factionStrengthLinesDeclaration.projectionOptions(faction, { extent: [100, 600], surface: '#111111' }));
  assert.ok(model.hoverDetail?.items.some(item => item.value === '300'), 'the group row is shown, never "No members"');
});

test('an unread roster at the displayed hour is said beside the earlier factions\' lines', () => {
  const base = makeCtx({ options: { measure: 'factories', count: 'free' }, factions: [{ id: '10', tag: 'GER', firstHour: 100, lastHour: 200 }] });
  const ctx = { ...base, countries: { ...base.countries, factionUnobserved: [[250, 600]] } } as unknown as FamilyPlanContext;
  const model = factionStrengthLinesProjection.project(createBoundedReadView({ hour: displayedHour(300), identity: fakeIdentity({}) }),
    factionStrengthLinesDeclaration.projectionOptions(ctx, { extent: [100, 600], surface: '#111111' }));
  assert.deepEqual(model.series.map(s => s.id), ['10'], 'the earlier faction keeps its line');
  assert.ok(model.warnings?.some(text => /not recorded/.test(text)), 'and the unread roster is said');
});

test('a shared faction colour is found under the identity the colour editor saved it under', () => {
  const view = createBoundedReadView({
    hour: displayedHour(300),
    panels: new Map([['main', { decimals: 0, states: [{ country: '10', hour: 300, run: { valueRaw: '410' }, missing: null }] }]]),
    identity: fakeIdentity({}),
  });
  const styled = withFactionTotalsStyles(factionStrengthLinesDeclaration, undefined, { '["leader","ITA"]': { colour: '#22c55e' } }, new Map([['10', '["leader","ITA"]']]));
  const opts = styled.projectionOptions(makeCtx({ options: { measure: 'factories', count: 'free' }, factions: [{ id: '10', tag: 'GER', firstHour: 100, lastHour: 600 }, { id: '10', tag: 'ITA', firstHour: 100, lastHour: 600 }] }),
    { extent: [100, 600], surface: '#111111' });
  assert.equal(factionStrengthLinesProjection.project(view, opts).series[0]!.color, '#22c55e', 'the founding leader the editor named, not the member listed first');
});

test('aggregate and ever-member lines never show a stale value at an unread hour and refuse unproved Top N (S6)', () => {
  const staleTile = {
    tag: 'group',
    from: 100,
    binHours: 1,
    decimals: 0,
    buckets: createBoundedSlice([
      { firstHour: 110, lastHour: 110, firstValue: 40, lastValue: 40, lastValueHour: 110, minValue: 40, minHour: 110, maxValue: 40, maxHour: 110, firstBreak: true, hasBreak: false },
    ], 1),
    straddling: 'none' as const,
    missing: createBoundedSlice([{ firstHour: 111, lastHour: 111, reason: 'A member value is not recorded at this hour' }], 1),
    validInterval: [100, 111] as const,
  };
  const aggUnreadHead: FactionTotalsAtReply = {
    hour: 111,
    kind: 'territory',
    measure: 'controlledStates',
    rows: [
      { id: 'group', members: ['GER'], memberCountries: [7], total: null, missing: 'A member value is not recorded at this hour' },
    ],
    missing: null,
    warnings: [],
  };
  const aggView = createBoundedReadView({
    hour: displayedHour(111),
    series: new Map([['main', new Map([['group', staleTile]])]]),
    panels: new Map([['head', aggUnreadHead]]),
    identity: fakeIdentity({ [`country:${countryKey(7)}`]: { name: 'German Reich', color: '#6b7280' } }),
  });
  const aggOpts = factionStrengthLinesDeclaration.projectionOptions(
    makeCtx({
      options: { measure: 'territory', territoryMeasure: 'states', rule: 'controlled', endLabels: true },
      countries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: true },
      tags: ['GER'],
    }),
    { extent: [100, 600], surface: '#111111' },
  );
  const aggModel = factionStrengthLinesProjection.project(aggView, aggOpts);
  assert.equal(aggModel.series.length, 1);
  assert.equal(aggModel.series[0]!.headValue, null);
  assert.equal(aggModel.series[0]!.absentReason, 'A member value is not recorded at this hour');
  assert.equal(aggModel.series[0]!.endLabel, null);
  assert.deepEqual(aggModel.auditRows?.map(r => [r.key, r.value]), [
    ['group', 'A member value is not recorded at this hour'],
  ]);

  const everTopRefused: FactionTotalsAtReply = {
    hour: 111,
    kind: 'territory',
    measure: 'controlledStates',
    rows: [],
    missing: 'The top 5 factions cannot be proved: a faction\'s total is not known at this hour',
    warnings: [],
  };
  const everView = createBoundedReadView({
    hour: displayedHour(111),
    series: new Map([['main', new Map([['10', { ...staleTile, tag: '10' }]])]]),
    panels: new Map([['head', everTopRefused]]),
    identity: fakeIdentity({ 'faction:10': { name: 'Axis', color: '#6b7280' } }),
  });
  const everOpts = factionStrengthLinesDeclaration.projectionOptions(
    makeCtx({
      options: { measure: 'territory', territoryMeasure: 'states', rule: 'controlled', endLabels: true },
      countries: { kind: 'factions', count: 5, membership: 'ever' },
      factions: [{ id: '10', tag: 'GER', firstHour: 100, lastHour: 600 }],
    }),
    { extent: [100, 600], surface: '#111111' },
  );
  const everModel = factionStrengthLinesProjection.project(everView, everOpts);
  assert.equal(everModel.series.length, 0);
  assert.equal(everModel.empty?.kind, 'unavailable');
  assert.match(everModel.empty?.message ?? '', /cannot be proved/);
});

test('a quiet stretch of played hours makes zero requests for aggregate groups and ever-member factions (S7)', () => {
  const cases = [
    {
      label: 'aggregate humans',
      ctx: makeCtx({
        options: { measure: 'territory', territoryMeasure: 'states', rule: 'controlled' },
        countries: { kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: true },
        tags: ['GER'],
      }),
    },
    {
      label: 'ever-member factions',
      ctx: makeCtx({
        options: { measure: 'factories', count: 'free' },
        countries: { kind: 'factions', count: 5, membership: 'ever' },
        factions: [{ id: '10', tag: 'GER', firstHour: 100, lastHour: 600 }],
      }),
    },
  ];

  for (const scenario of cases) {
    const plan = factionStrengthLinesDeclaration.sources(scenario.ctx, { following: true, staticSpan: null });
    const compiled = compileFollowPlan(plan, query => ({ query: query as never, refused: null }));
    assert.equal(compiled.refused, null, `${scenario.label}: follow plan compiles`);
    assert.equal(compiled.subscriptions.length, 1, `${scenario.label}: head uses follow subscription`);
    const entries = unitEntries(plan, true, compiled.subscriptions);
    assert.equal(entries.length, 0, `${scenario.label}: zero unfollowed per-hour units in follow mode`);

    let currentHour = 110;
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
        return Promise.resolve(null);
      },
      () => undefined,
    );
    const branch = { id: '1', firstHour: 100, lastHour: 600, gaps: [] } as never;
    const gate = { open: true, recorded: (h: number) => h, span: (a: number, b: number) => ({ firstHour: a, lastHour: b }), coverageSpan: () => [100, 600] as const } as never;
    target.configure({
      inputs: scenario.label,
      entries,
      active: true,
      content: 0,
      trackOffset: 0,
      following: true,
      subscribed: true,
      playback: () => ({ branch, effectiveEndHour: 600, gate, countriesKnown: () => true }),
    });
    for (let h = 111; h <= 140; h++) {
      currentHour = h;
      for (const cb of listeners) cb();
    }
    assert.equal(requests, 0, `${scenario.label}: 30 quiet played hours make zero product requests`);
    target.dispose();
  }
});
