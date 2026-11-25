import assert from 'node:assert/strict';
import test from 'node:test';
import { countryKey } from '../../dashboard/countryKey.ts';
import { hoverAsks } from '../../families/broker.ts';
import { createBoundedReadView, displayedHour, FutureDataError } from '../../families/boundedView.ts';
import type { FamilyPlanContext, IdentityProvider } from '../../families/contract.ts';
import type { NuclearArsenalReply, SeriesRankReply } from '../../generated/dashboard';
import { TIME_CHART_VIEWS } from '../manifest/time-chart.ts';
import { resolveWidgetScopedOptions } from '../manifest.ts';
import { nuclearArsenalLinesDeclaration, nuclearArsenalLinesProjection } from './nuclearArsenalLinesProjection.ts';

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
    },
    following: true,
    aux: undefined,
    widget: { id: 'w', kind: 'nuclear-arsenal', trackIds: ['t'], countries: { kind: 'tags', tags }, time: { mode: 'follow' }, options: overrides.options ?? {} },
  } as unknown as FamilyPlanContext;
}

test('nuclear-arsenal lines is registered in TIME_CHART_VIEWS with scoped options and builds ranked and keyed plans', () => {
  const entry = TIME_CHART_VIEWS.find(item => item.kind === 'nuclear-arsenal' && item.view === 'lines');
  assert.ok(entry);
  assert.equal(entry.family, 'time-chart');

  const keys = resolveWidgetScopedOptions('nuclear-arsenal', { view: 'lines' }).map(field => field.key);
  for (const key of ['view', 'endLabels', 'playback', 'yMin', 'yMax', 'log']) {
    assert.ok(keys.includes(key), `lines settings include ${key}`);
  }
  assert.equal(keys.includes('split'), false);

  const topPlan = nuclearArsenalLinesDeclaration.sources(makeCtx({ top: 6 }), { following: true, staticSpan: null });
  const topMain = topPlan.entries.find(e => e.name === 'main');
  assert.ok(topMain && topMain.read.shape === 'pixel-binned');
  assert.deepEqual(topMain.read.family, { kind: 'country-nuke-totals', measure: 'total' });
  assert.deepEqual(topMain.read.top, { n: 6 });
  assert.equal(topMain.read.exactProduct, 'series-rank');
  const detail = topPlan.entries.find(e => e.name === 'detail');
  assert.ok(detail && detail.read.shape === 'exact-hover' && detail.read.product === 'nuclear-arsenal');

  const slot1 = countryKey(1);
  const slot2 = countryKey(2);
  const keyedPlan = nuclearArsenalLinesDeclaration.sources(makeCtx({ tags: [slot1, slot2] }), { following: true, staticSpan: null });
  const rankEntry = keyedPlan.entries.find(e => e.name === 'rank');
  assert.ok(rankEntry && rankEntry.read.shape === 'point' && rankEntry.read.product === 'series-rank');
});

test('nuclear-arsenal lines ranks chosen slots, preserves missing readings, and renders weapon-type hover details', () => {
  const slot1 = countryKey(1);
  const slot2 = countryKey(2);
  const rankReply: SeriesRankReply = {
    branchId: '1',
    hour: 500,
    from: 500,
    until: 500,
    refused: null,
    measures: [{ name: 'total', decimals: 0 }],
    rows: [
      { country: slot2, slot: 2, readings: [] },
      { country: slot1, slot: 1, readings: [] },
    ],
  };
  const mainPanel = {
    branchId: '1',
    measure: 'total',
    decimals: 0,
    states: [
      {
        country: slot2,
        hour: 500,
        drop: null,
        missing: null,
        run: { firstHour: 450, lastHour: 500, valueRaw: '7', slot: 2, countryId: 2, startsSegment: false, segmentFirstHour: 100, firstExact: true, lastExact: true },
      },
      {
        country: slot1,
        hour: 500,
        drop: null,
        missing: 'Nukes were not read at this hour',
        run: null,
      },
    ],
  };
  const hoverReply: NuclearArsenalReply = {
    missing: null,
    warnings: [],
    types: [
      { ordinal: 0, token: 'nuclear_bomb', name: 'Nuclear Bomb' },
      { ordinal: 1, token: 'thermonuclear_bomb', name: null },
    ],
    countries: [
      { tag: 'USA', countryId: 2, slot: 2, total: 7, counts: [{ ordinal: 0, value: 5, ready: 2 }, { ordinal: 1, value: 2, ready: 0 }] },
    ],
  };
  const readView = createBoundedReadView({
    hour: displayedHour(500),
    panels: new Map<string, unknown>([['rank', rankReply], ['main', mainPanel]]),
    hovers: new Map([['detail', { hour: displayedHour(480), data: hoverReply }]]),
    identity: fakeIdentity({
      [slot1]: { name: 'German Reich', color: '#6b7280' },
      [slot2]: { name: 'United States', color: '#3b82f6' },
    }),
  });
  const opts = nuclearArsenalLinesDeclaration.projectionOptions(
    makeCtx({ tags: [slot1, slot2], options: { endLabels: true } }),
    { extent: [100, 600], surface: '#111111' },
  );
  const model = nuclearArsenalLinesProjection.project(readView, opts);
  assert.deepEqual(model.series.map(s => [s.id, s.name, s.headValue, s.step]), [
    [slot2, 'United States', 7, true],
    [slot1, 'German Reich', null, true],
  ]);
  assert.equal(model.series[0]!.endLabel?.value, '7');
  assert.equal(model.series[1]!.endLabel, null);
  assert.ok(model.auditRows?.some(row => row.key === slot1 && row.value?.includes('Nukes were not read at this hour')));
  assert.equal(model.hoverDetail?.hour, 480);
  assert.ok(model.hoverDetail?.items.some(item => item.label === 'Nuclear Bomb' && item.value === '5 (2 ready)'));
  assert.ok(model.hoverDetail?.items.some(item => item.label === 'thermonuclear_bomb' && item.value === '2'));
  assert.throws(() => readView.who('country', slot2, displayedHour(501)), FutureDataError);

  const emptyTopView = createBoundedReadView({
    hour: displayedHour(200),
    series: new Map([['main', new Map()]]),
    identity: fakeIdentity({}),
  });
  const emptyTopOpts = nuclearArsenalLinesDeclaration.projectionOptions(
    makeCtx({ top: 6, options: { endLabels: true } }),
    { extent: [100, 600], surface: '#111111' },
  );
  const emptyModel = nuclearArsenalLinesProjection.project(emptyTopView, emptyTopOpts);
  assert.deepEqual(emptyModel.empty, { message: 'No nuclear stockpiles recorded at this date.', kind: 'empty' });
});

test('rv18 S5 & S4: custom hover detail requests and projects only slots eligible at the hover hour and reports missing readings instead of 0', () => {
  const slot1 = countryKey(1);
  const slot2 = countryKey(2);
  const membership = {
    [slot1]: [[100, 600]] as const,
    [slot2]: [[120, 600]] as const,
  };
  const ctx = makeCtx({ tags: [slot1, slot2], membership });
  const plan = nuclearArsenalLinesDeclaration.sources(ctx, { following: true, staticSpan: null });

  const asks110 = hoverAsks(plan, 110);
  const detail110 = asks110.find(ask => ask.name === 'detail');
  assert.ok(detail110, 'detail ask is emitted when at least one slot is eligible at hour 110');
  assert.deepEqual((detail110.request.params as { countries?: unknown }).countries, [1], 'S5: slot 2 joined at 120, so hour 110 asks only slot 1');
  const hover110 = asks110.find(ask => ask.name === 'hover');
  assert.deepEqual((hover110?.request.params as { reads?: unknown })?.reads, [{ country: 1, hour: 110 }]);

  const asks130 = hoverAsks(plan, 130);
  const detail130 = asks130.find(ask => ask.name === 'detail');
  assert.deepEqual((detail130?.request.params as { countries?: unknown })?.countries, [1, 2], 'at hour 130 both slots are eligible');

  const topPlan = nuclearArsenalLinesDeclaration.sources(makeCtx({ top: 6 }), { following: true, staticSpan: null });
  const topDetail = hoverAsks(topPlan, 480, name => (name === 'main' ? [slot2] : undefined)).find(ask => ask.name === 'detail');
  assert.deepEqual((topDetail?.request.params as { countries?: unknown })?.countries, [2], 'ranked top detail asks only the displayed slots, never countries: []');
  assert.equal(hoverAsks(topPlan, 480, () => []).find(ask => ask.name === 'detail'), undefined, 'no displayed slots makes no detail ask');

  const hoverReply: NuclearArsenalReply = {
    missing: null,
    warnings: ['1 countries with a stockpile are held by no tag at this hour and are not shown'],
    types: [{ ordinal: 0, token: 'nuclear_bomb', name: 'Nuclear Bomb' }],
    countries: [
      { tag: 'USA', countryId: 2, slot: 2, total: 5, counts: [{ ordinal: 0, value: 5, ready: 1 }] },
    ],
  };
  const exactHover = {
    branchId: '1',
    measure: 'total',
    decimals: 0,
    states: [
      { country: 1, hour: 110, drop: null, missing: 'No country held slot 1 at this hour', run: null },
    ],
  };
  const view110 = createBoundedReadView({
    hour: displayedHour(500),
    hovers: new Map<string, { readonly hour: ReturnType<typeof displayedHour>; readonly data: unknown }>([
      ['detail', { hour: displayedHour(110), data: hoverReply }],
      ['hover', { hour: displayedHour(110), data: exactHover }],
    ]),
    identity: fakeIdentity({
      [slot1]: { name: 'German Reich', color: '#6b7280' },
      [slot2]: { name: 'United States', color: '#3b82f6' },
    }),
  });
  const opts = nuclearArsenalLinesDeclaration.projectionOptions(ctx, { extent: [100, 600], surface: '#111111' });
  const model110 = nuclearArsenalLinesProjection.project(view110, opts);
  assert.equal(model110.hoverDetail?.hour, 110);
  const gerColor = model110.series.find(s => s.id === slot1)!.color;
  assert.deepEqual(
    model110.hoverDetail?.items,
    [{ label: 'German Reich', swatch: gerColor, missingReason: 'No country held slot 1 at this hour', divider: false }],
    'S5 & S4: slot 2 (joins at 120) is omitted at 110, and slot 1 reports its missing reason rather than 0',
  );

  const viewUnattributed = createBoundedReadView({
    hour: displayedHour(500),
    hovers: new Map<string, { readonly hour: ReturnType<typeof displayedHour>; readonly data: unknown }>([
      ['detail', { hour: displayedHour(130), data: hoverReply }],
    ]),
    identity: fakeIdentity({
      [slot1]: { name: 'German Reich', color: '#6b7280' },
      [slot2]: { name: 'United States', color: '#3b82f6' },
    }),
  });
  const model130 = nuclearArsenalLinesProjection.project(viewUnattributed, opts);
  const gerItem = model130.hoverDetail?.items.find(item => item.label === 'German Reich');
  assert.equal(gerItem?.value, undefined, 'S4: a slot omitted from nuclear-arsenal without a known exact reading never shows 0');
  assert.ok(gerItem?.missingReason, 'S4: absent slot shows a missing reason');
});
