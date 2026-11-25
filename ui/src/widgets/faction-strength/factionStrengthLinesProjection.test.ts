import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, createBoundedSlice, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
import type { FactionTotalsAtReply } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';
import { resolveWidgetScopedOptions } from '../manifest.ts';
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
  const linesEntry = aggPlan.entries.find(e => e.name === 'lines');
  assert.ok(linesEntry && linesEntry.read.shape === 'branch-to-hour' && linesEntry.read.product === 'faction-lines');
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
