import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, createBoundedSlice, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';
import { resolveWidgetScopedOptions } from '../manifest.ts';
import type { FactionTotalsAtReply } from '../../generated/dashboard';
import {
  territoryLinesDeclaration,
  territoryLinesDeclarationWithStyles,
  territoryLinesProjection,
} from './territoryLinesProjection.ts';

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
  readonly top?: number | null;
  readonly tags?: readonly string[];
  readonly factions?: readonly { readonly id: string; readonly tag: string; readonly firstHour: number; readonly lastHour: number }[];
  readonly membership?: Readonly<Record<string, readonly (readonly [number, number])[]>> | null;
}): FamilyPlanContext {
  const tags = overrides.tags ?? [];
  return {
    track: { source: { id: 's', revision: 1 }, axisOffset: 0 },
    branch: { id: '1', firstHour: 100, lastHour: 600, gaps: [] },
    options: overrides.options ?? {},
    countries: {
      all: false,
      top: overrides.top ?? null,
      every: tags,
      tags,
      holders: null,
      membership: overrides.membership ?? null,
      unobserved: [],
      factions: overrides.factions ?? null,
      reason: null,
    },
    following: true,
    aux: undefined,
    widget: {
      id: 'w',
      kind: 'territory',
      trackIds: ['t'],
      countries: overrides.countries ?? { kind: 'group', group: { type: 'top', count: 10 } },
      time: { mode: 'follow' },
      options: overrides.options ?? {},
    },
  } as unknown as FamilyPlanContext;
}

test('territory lines registers in TIME_CHART_VIEWS and selects series tiles for countries and faction-totals for factions', () => {
  const entry = TIME_CHART_VIEWS.find(item => item.kind === 'territory' && item.view === 'lines');
  assert.ok(entry);
  assert.equal(entry.family, 'time-chart');

  const keys = resolveWidgetScopedOptions('territory', { view: 'lines' }).map(field => field.key);
  for (const key of ['view', 'measure', 'rule', 'endLabels', 'playback', 'yMin', 'yMax', 'log']) {
    assert.ok(keys.includes(key), `lines settings include ${key}`);
  }

  const countryPlan = territoryLinesDeclaration.sources(
    makeCtx({ options: { measure: 'victoryPoints', rule: 'occupied' }, top: 10 }),
    { following: true, staticSpan: null },
  );
  const main = countryPlan.entries.find(e => e.name === 'main');
  assert.ok(main && main.read.shape === 'pixel-binned');
  assert.deepEqual(main.read.family, { kind: 'territory', measure: 'occupiedVictoryPoints' });
  assert.deepEqual(main.read.top, { n: 10 });
  assert.ok(main.read.headQuery && (main.read.headQuery as { product?: string }).product === 'series-state');

  const factionPlan = territoryLinesDeclaration.sources(
    makeCtx({
      options: { measure: 'states', rule: 'controlled' },
      countries: { kind: 'factions', count: Number.MAX_SAFE_INTEGER, membership: 'hour' },
      factions: [{ id: '10', tag: 'GER', firstHour: 100, lastHour: 600 }],
    }),
    { following: true, staticSpan: null },
  );
  const factionMain = factionPlan.entries.find(e => e.name === 'main');
  assert.ok(factionMain && factionMain.read.shape === 'pixel-binned');
  assert.deepEqual(factionMain.read.family, { kind: 'faction-totals', measure: 'controlledStates' });
});

test('territory lines projects slot-keyed countries, whole-number axis, area note, missing readings, and dated hover', () => {
  const ger = countryKey(7);
  const sov = countryKey(9);
  const view = createBoundedReadView({
    hour: displayedHour(300),
    series: new Map([
      ['main', new Map([
        [ger, {
          tag: ger,
          from: 100,
          binHours: 1,
          decimals: 0,
          buckets: createBoundedSlice([], 0),
          straddling: 'none' as const,
          missing: createBoundedSlice([], 0),
          validInterval: [100, 300] as const,
        }],
        [sov, {
          tag: sov,
          from: 100,
          binHours: 1,
          decimals: 0,
          buckets: createBoundedSlice([], 0),
          straddling: 'none' as const,
          missing: createBoundedSlice([{ firstHour: 240, lastHour: 260, reason: 'Territory not recorded' }], 1),
          validInterval: [100, 300] as const,
        }],
      ])],
    ]),
    panels: new Map([
      ['main', {
        decimals: 0,
        states: [
          { country: 7, hour: 300, run: { valueRaw: '142' }, missing: null },
          { country: 9, hour: 300, run: null, missing: 'Territory not recorded at this hour' },
        ],
      }],
    ]),
    identity: fakeIdentity({
      [`country:${ger}`]: { name: 'German Reich', color: '#4b5563' },
      [`country:${sov}`]: { name: 'Soviet Union', color: '#dc2626' },
    }),
  });
  const opts = territoryLinesDeclaration.projectionOptions(
    makeCtx({
      options: { measure: 'area', rule: 'controlled', endLabels: true },
      countries: { kind: 'explicit', tags: [ger, sov] },
      tags: [ger, sov],
    }),
    { extent: [100, 600], surface: '#111111' },
  );
  const model = territoryLinesProjection.project(view, opts);
  assert.equal(model.yAxis.format, 'number');
  assert.equal(model.yAxis.decimals, 0);
  assert.equal(model.note, 'Map area exaggerates territory near the poles.');
  assert.deepEqual(model.series.map(s => [s.id, s.name, s.headValue, s.endLabel?.value ?? null]), [
    [ger, 'German Reich', 142, '142'],
    [sov, 'Soviet Union', null, null],
  ]);
  assert.deepEqual(model.series[1]!.missingSpans, [{ firstHour: 240, lastHour: 260, reason: 'Territory not recorded' }]);
  assert.ok(model.auditRows?.some(row => row.key === sov && row.value?.includes('not recorded')));
  assert.throws(() => view.who('country', ger, displayedHour(301)), FutureDataError);
});

test('territory lines applies faction renames and colours and keeps quiet stretches free of extra requests', () => {
  const hoverReply: FactionTotalsAtReply = {
    hour: 280,
    kind: 'territory',
    measure: 'controlledStates',
    rows: [
      { id: '10', members: ['GER'], memberCountries: [7], total: 95, missing: null },
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
          { country: '10', hour: 300, run: { valueRaw: '95' }, missing: null },
        ],
      }],
    ]),
    hovers: new Map([['hover', { hour: displayedHour(280), data: hoverReply }]]),
    identity: fakeIdentity({
      'faction:10': { name: 'Axis', color: '#6b7280' },
      [`country:${countryKey(7)}`]: { name: 'German Reich', color: '#6b7280' },
    }),
  });
  const styled = territoryLinesDeclarationWithStyles(
    { '10': { name: 'Central Pact', colour: '#f59e0b' } },
    {},
  );
  const ctx = makeCtx({
    options: { measure: 'states', rule: 'controlled', endLabels: true },
    countries: { kind: 'factions', count: Number.MAX_SAFE_INTEGER, membership: 'hour' },
    factions: [{ id: '10', tag: 'GER', firstHour: 100, lastHour: 600 }],
  });
  const opts = styled.projectionOptions(ctx, { extent: [100, 600], surface: '#111111' });
  const model = territoryLinesProjection.project(view, opts);
  assert.deepEqual(model.series.map(s => [s.id, s.name, s.color, s.endLabel?.name, s.endLabel?.colorBar]), [
    ['10', 'Central Pact', '#f59e0b', 'Central Pact', '#f59e0b'],
  ]);
  const hovered = model.hoverDetail?.items.find(item => item.label === 'Central Pact');
  assert.equal(hovered?.swatch, '#f59e0b');

  const plan = territoryLinesDeclaration.sources(ctx, { following: true, staticSpan: null });
  const main = plan.entries.find(e => e.name === 'main');
  assert.ok(main && main.read.shape === 'pixel-binned');
  assert.equal(main.read.headQuery?.product, 'series-state');
  assert.equal(plan.entries.filter(e => e.read.shape === 'point').length, 0);
});
