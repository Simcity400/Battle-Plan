import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
import type { SeriesRankReply } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';
import { resolveWidgetScopedOptions } from '../manifest.ts';
import { gameScoreLinesDeclaration, gameScoreLinesProjection } from './gameScoreLinesProjection.ts';

function fakeIdentity(names: Readonly<Record<string, { readonly name: string; readonly color: string }>>): IdentityProvider {
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
  readonly top?: number | null;
  readonly tags?: readonly string[];
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
      membership: null,
      unobserved: [],
    },
    following: true,
    aux: undefined,
    widget: { id: 'w', kind: 'game-score', trackIds: ['t'], countries: { kind: 'tags', tags }, time: { mode: 'follow' }, options: overrides.options ?? {} },
  } as unknown as FamilyPlanContext;
}

test('game-score lines is registered in TIME_CHART_VIEWS with scoped options and builds ranked and keyed plans', () => {
  const entry = TIME_CHART_VIEWS.find(item => item.kind === 'game-score' && item.view === 'lines');
  assert.ok(entry);
  assert.equal(entry.family, 'time-chart');

  const keys = resolveWidgetScopedOptions('game-score', { view: 'lines' }).map(field => field.key);
  for (const key of ['view', 'topMeasure', 'endLabels', 'playback', 'yMin', 'yMax', 'log']) {
    assert.ok(keys.includes(key), `lines settings include ${key}`);
  }
  assert.equal(keys.includes('split'), false);
  assert.equal(keys.includes('factions'), false);

  const topPlan = gameScoreLinesDeclaration.sources(makeCtx({ top: 6, options: { topMeasure: 'army' } }), { following: true, staticSpan: null });
  const topMain = topPlan.entries.find(e => e.name === 'main');
  assert.ok(topMain && topMain.read.shape === 'pixel-binned');
  assert.deepEqual(topMain.read.family, { kind: 'country-game-scores', measure: 'army' });
  assert.deepEqual(topMain.read.top, { n: 6 });
  assert.equal(topMain.read.exactProduct, 'series-rank');

  const slot1 = countryKey(1);
  const slot2 = countryKey(2);
  const keyedPlan = gameScoreLinesDeclaration.sources(makeCtx({ tags: [slot1, slot2], options: { topMeasure: 'total' } }), { following: true, staticSpan: null });
  const keyedMain = keyedPlan.entries.find(e => e.name === 'main');
  assert.ok(keyedMain && keyedMain.read.shape === 'pixel-binned');
  assert.deepEqual(keyedMain.read.family, { kind: 'country-game-scores', measure: 'total' });
  assert.equal(keyedMain.read.headQuery?.coverage, 'row');
  const rankEntry = keyedPlan.entries.find(e => e.name === 'rank');
  assert.ok(rankEntry && rankEntry.read.shape === 'point' && rankEntry.read.product === 'series-rank');
});

test('game-score lines orders chosen slots by series-rank, formats 5-decimal scores as whole numbers, and keeps missing scores unrecorded', () => {
  const slot1 = countryKey(1);
  const slot2 = countryKey(2);
  const rankReply: SeriesRankReply = {
    branchId: '1',
    hour: 500,
    from: 500,
    until: 500,
    refused: null,
    measures: [{ name: 'total', decimals: 5 }],
    rows: [
      { country: slot2, slot: 2, readings: [] },
      { country: slot1, slot: 1, readings: [] },
    ],
  };
  const mainPanel = {
    branchId: '1',
    measure: 'total',
    decimals: 5,
    states: [
      {
        country: slot2,
        hour: 500,
        drop: null,
        missing: null,
        run: { firstHour: 400, lastHour: 500, valueRaw: '420000000', slot: 2, countryId: 2, startsSegment: false, segmentFirstHour: 100, firstExact: true, lastExact: true },
      },
      {
        country: slot1,
        hour: 500,
        drop: null,
        missing: 'Scores were not read at this hour',
        run: null,
      },
    ],
  };
  const readView = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map<string, unknown>([['rank', rankReply], ['main', mainPanel]]),
    identity: fakeIdentity({
      [slot1]: { name: 'German Reich', color: '#6b7280' },
      [slot2]: { name: 'United States', color: '#3b82f6' },
    }),
  });
  const opts = gameScoreLinesDeclaration.projectionOptions(
    makeCtx({ tags: [slot1, slot2], options: { topMeasure: 'total', endLabels: true } }),
    { extent: [100, 600], surface: '#111111' },
  );
  const model = gameScoreLinesProjection.project(readView, opts);
  assert.deepEqual(model.series.map(s => [s.id, s.name, s.headValue]), [
    [slot2, 'United States', 4200],
    [slot1, 'German Reich', null],
  ]);
  assert.equal(model.series[0]!.endLabel?.value, '4,200');
  assert.equal(model.series[1]!.endLabel, null);
  assert.ok(model.auditRows?.some(row => row.key === slot2 && row.value?.includes('4,200')));
  assert.ok(model.auditRows?.some(row => row.key === slot1 && row.value?.includes('Scores were not read at this hour')));
  assert.throws(() => readView.who('country', slot2, displayedHour(501)), FutureDataError);
});
