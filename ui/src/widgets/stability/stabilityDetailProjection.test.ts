import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { describe, it } from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createBoundedReadView, createBoundedSlice, displayedHour, emptySlice } from '../../families/boundedView.ts';
import { hoverAsks } from '../../families/broker.ts';
import type { BoundedSeries, FamilyPlanContext } from '../../families/contract.ts';
import { SceneTiles } from '../../families/sceneTiles.ts';
import { deriveTimeChartTooltip } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import { knownValue } from '../../families/value.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { keyId, type KeyedState, type KeyedStates } from '../../dashboard/keyedSeriesData.ts';
import type { BreakdownLine } from '../../generated/dashboard';
import {
  buildStabilityNextChangeParams,
  buildStabilityPartsChartModel,
  buildStabilityPartsTableModel,
  PARTS_FAMILY,
  STABILITY_PARTS_CHART_KIND,
  STABILITY_PARTS_TABLE_KIND,
  stabilityPartsChartDeclaration,
  stabilityPartsTableProjection,
  type StabilityDetailOptions,
} from './stabilityDetailProjection.ts';
import type { StabilityPartName } from './types';

const TOTAL_PART = '1';
const POS_PART = '2';
const NEG_PART = '3';
const SMALL_POS = '4';
const SMALL_NEG = '5';
const REMAINDER_PART = '6';

function lineOf(identity: string, label: string, valueRaw: string): BreakdownLine {
  return {
    line: 0,
    kind: 'idea',
    identity,
    identityId: 0,
    secondaryRaw: '0',
    valueRaw,
    text: null,
    label,
    group: null,
    groupKind: null,
  };
}

const NAMES: readonly StabilityPartName[] = [
  { part: TOTAL_PART, measure: 'stability', kind: 'total', id: null, term: null, line: null, missing: null },
  { part: POS_PART, measure: 'stability', kind: 'term', id: 'base', term: 'base', line: null, missing: null },
  { part: NEG_PART, measure: 'stability', kind: 'term', id: 'war', term: 'war', line: null, missing: null },
  { part: SMALL_POS, measure: 'stability', kind: 'line', id: 'line:a', term: null, line: lineOf('a', 'Minor bonus', '1000'), missing: null },
  { part: SMALL_NEG, measure: 'stability', kind: 'line', id: 'line:b', term: null, line: lineOf('b', 'Minor penalty', '-1000'), missing: null },
  { part: REMAINDER_PART, measure: 'stability', kind: 'remainder', id: 'remainder', term: null, line: null, missing: null },
];

function state(part: string, hour: number, valueRaw: string): KeyedState {
  return {
    tag: 'GER',
    part,
    hour,
    drop: null,
    missing: null,
    run: {
      firstHour: hour,
      lastHour: hour,
      valueRaw,
      slot: 0,
      countryId: 0,
      startsSegment: false,
      segmentFirstHour: hour,
      firstExact: true,
      lastExact: true,
    },
  };
}

function baseOptions(overrides: Partial<StabilityDetailOptions> = {}): StabilityDetailOptions {
  return {
    ownerKey: 'GER',
    spans: null,
    parts: [TOTAL_PART, POS_PART, NEG_PART, REMAINDER_PART],
    partBits: 4,
    names: NAMES,
    namesReady: true,
    warnings: [],
    measure: 'stability',
    extent: [100, 200],
    selectedHour: null,
    surface: '#111111',
    ...overrides,
  };
}

function seriesOf(firstHour: number, lastHour: number, value: number): BoundedSeries {
  const runs = [{ firstHour, lastHour, value }];
  return {
    tag: 'GER',
    decimals: 5,
    binHours: 1,
    from: firstHour,
    validInterval: [firstHour, lastHour],
    runs: createBoundedSlice(runs, runs.length),
    buckets: emptySlice(),
    missing: emptySlice(),
    straddling: 'none',
  };
}

describe('stability detail projections', () => {
  it('builds a signed parts table with Other last, exact Total, and unavailable states', () => {
    const unavailable = buildStabilityPartsTableModel(150, null, baseOptions());
    assert.equal(unavailable.emptyKind, 'unavailable');
    assert.equal(unavailable.rows.length, 0);

    const noTotal = buildStabilityPartsTableModel(150, [state(POS_PART, 150, '50000')], baseOptions());
    assert.equal(noTotal.emptyKind, 'unavailable');

    const model = buildStabilityPartsTableModel(
      150,
      [
        state(TOTAL_PART, 150, '40000'),
        state(POS_PART, 150, '50000'),
        state(NEG_PART, 150, '-15000'),
        state(REMAINDER_PART, 150, '5000'),
      ],
      baseOptions(),
    );
    assert.deepEqual(model.rows.map(row => row.key), ['base', 'war', 'remainder']);
    assert.equal(model.rows[0]!.cells.value?.value.kind, 'known');
    assert.equal(model.rows[0]!.cells.value?.value.kind === 'known' ? model.rows[0]!.cells.value.value.value : '', '+50.0%');
    assert.equal(model.rows[1]!.cells.value?.tone, 'negative');
    assert.equal(model.rows[1]!.cells.value?.bar?.alert, true);
    assert.equal(model.rows[2]!.name, 'Other');
    assert.equal(model.totalRow?.name, 'Total');
    assert.equal(model.totalRow?.cells.value?.value.kind === 'known' ? model.totalRow.cells.value.value.value : '', '40.0%');

    const loadingNames = buildStabilityPartsTableModel(
      150,
      [state(TOTAL_PART, 150, '10000'), state('99', 150, '10000')],
      baseOptions({ parts: [TOTAL_PART, '99'], namesReady: false }),
    );
    assert.equal(loadingNames.rows[0]!.name, 'Not identified');
    assert.equal(loadingNames.endNote, 'Loading names…');
  });

  it('splits opposite-sign members of a grouped layer onto positive and negative stack bands and preserves selectedHour and hover detail', () => {
    const manyNames: StabilityPartName[] = [NAMES[0]!];
    const parts: string[] = [TOTAL_PART];
    const seriesMap = new Map<string, BoundedSeries>([
      [keyId({ tag: 'GER', part: TOTAL_PART }), seriesOf(100, 200, 0.5)],
    ]);
    for (let i = 1; i <= 12; i++) {
      const p = String(100 + i);
      parts.push(p);
      manyNames.push({
        part: p,
        measure: 'stability',
        kind: 'line',
        id: `big:${i}`,
        term: null,
        line: lineOf(`big:${i}`, `Big ${i}`, '20000'),
        missing: null,
      });
      seriesMap.set(keyId({ tag: 'GER', part: p }), seriesOf(100, 200, 0.2));
    }
    parts.push(SMALL_POS, SMALL_NEG);
    manyNames.push(NAMES[3]!, NAMES[4]!);
    seriesMap.set(keyId({ tag: 'GER', part: SMALL_POS }), seriesOf(100, 200, 0.01));
    seriesMap.set(keyId({ tag: 'GER', part: SMALL_NEG }), seriesOf(100, 200, -0.01));

    const hoverStates: KeyedStates = {
      branchId: '1',
      decimals: 5,
      states: [
        state(TOTAL_PART, 140, '50000'),
        state(SMALL_POS, 140, '1000'),
        state(SMALL_NEG, 140, '-1000'),
      ],
    };
    const view = createBoundedReadView({
      hour: displayedHour(200),
      series: new Map([['parts', seriesMap]]),
      hovers: new Map([['hover', { hour: 140, data: hoverStates }]]),
    });
    const model = buildStabilityPartsChartModel(
      view,
      baseOptions({ parts, names: manyNames, selectedHour: 140 }),
    );
    assert.equal(model.selectedHour, 140);
    const smallerBands = (model.stack?.bands ?? []).filter(band => band.id.startsWith('__smaller'));
    assert.equal(smallerBands.length, 2, 'opposite-sign grouped members split into separate positive and negative bands');
    assert.ok(smallerBands.some(band => (band.values[0] ?? 0) > 0));
    assert.ok(smallerBands.some(band => (band.values[0] ?? 0) < 0));
    assert.equal(model.hoverDetail?.hour, 140);
    assert.equal(model.hoverDetail?.items.at(-1)?.label, 'Total');

    const emptyHoverView = createBoundedReadView({
      hour: displayedHour(200),
      series: new Map([['parts', seriesMap]]),
      hovers: new Map([['hover', { hour: 145, data: { branchId: '1', decimals: 5, states: [] } }]]),
    });
    const missingHover = buildStabilityPartsChartModel(emptyHoverView, baseOptions({ parts, names: manyNames }));
    assert.equal(missingHover.hoverDetail?.items[0]?.missingReason, 'Not recorded');

    const outsideSpanHover = buildStabilityPartsChartModel(
      view,
      baseOptions({ parts, names: manyNames, spans: [{ firstHour: 100, lastHour: 130 }] }),
    );
    assert.equal(outsideSpanHover.hoverDetail, undefined, 'hover outside membership spans is suppressed');

    const plan = stabilityPartsChartDeclaration.sources({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1' },
      options: baseOptions({ ownerKey: countryKey(1), parts: [TOTAL_PART, POS_PART], spans: [{ firstHour: 100, lastHour: 150 }] }),
    } as unknown as FamilyPlanContext, { width: 600, height: 220, following: false, staticSpan: [100, 200] });
    assert.equal(hoverAsks(plan, 120).length, 1);
    assert.equal(hoverAsks(plan, 160).length, 0, 'stability hover outside membership issues no request');
  });

  it('uses a signed vertical axis for negative modifiers, clamps range draws to extent, and keeps bucket extrema across breaks and tied hours', () => {
    const measure = (text: string) => text.length * 6.5;
    const signedView = createBoundedReadView({
      hour: displayedHour(1000),
      series: new Map([['parts', new Map<string, BoundedSeries>([
        [keyId({ tag: 'GER', part: TOTAL_PART }), seriesOf(100, 200, 0.4)],
        [keyId({ tag: 'GER', part: POS_PART }), seriesOf(100, 200, 0.5)],
        [keyId({ tag: 'GER', part: NEG_PART }), seriesOf(100, 200, -0.1)],
      ])]]),
    });
    const signedModel = buildStabilityPartsChartModel(signedView, baseOptions({ parts: [TOTAL_PART, POS_PART, NEG_PART], extent: [100, 200] }));
    assert.equal(signedModel.hour, 200, 'range breakdown clamps chart hour to range end');
    const layout = layoutTimeChart(signedModel, 600, 220, '12px sans-serif', measure);
    assert.ok(layout.yDomain[0] < 0, `negative modifier stays on signed y-axis (got ${layout.yDomain.join('..')})`);
    assert.equal(layout.xDomain[1], 200, 'range breakdown x-axis stops at range end');
    assert.equal(deriveTimeChartTooltip(signedModel, layout, 'plot:500'), null, 'tooltip beyond range end reports no data');

    const coarseBuckets = [
      {
        firstHour: 100,
        lastHour: 147,
        firstValue: 0.1,
        lastValue: 0.15,
        lastValueHour: 147,
        minHour: 112,
        minValue: -0.2,
        maxHour: 130,
        maxValue: 0.9,
        firstBreak: false,
        hasBreak: true,
      },
      {
        firstHour: 148,
        lastHour: 195,
        firstValue: 0.1,
        lastValue: 0.1,
        lastValueHour: 195,
        minHour: 160,
        minValue: 0.02,
        maxHour: 160,
        maxValue: 0.85,
        firstBreak: false,
        hasBreak: false,
      },
    ];
    const coarseSeries: BoundedSeries = {
      tag: 'GER',
      decimals: 5,
      binHours: 48,
      from: 100,
      validInterval: [100, 195],
      runs: emptySlice(),
      buckets: createBoundedSlice(coarseBuckets, coarseBuckets.length),
      missing: emptySlice(),
      straddling: 'none',
    };
    const coarseView = createBoundedReadView({
      hour: displayedHour(200),
      series: new Map([['parts', new Map<string, BoundedSeries>([
        [keyId({ tag: 'GER', part: TOTAL_PART }), seriesOf(100, 195, 0.1)],
        [keyId({ tag: 'GER', part: POS_PART }), coarseSeries],
      ])]]),
    });
    const coarseModel = buildStabilityPartsChartModel(coarseView, baseOptions({ parts: [TOTAL_PART, POS_PART], extent: [100, 200] }));
    const posValues = coarseModel.stack!.bands.flatMap(b => b.values).filter((v): v is number => v !== null);
    assert.ok(posValues.some(v => Math.abs(v - -20) < 1e-6), 'bucket with break preserves min extremum');
    assert.ok(posValues.some(v => Math.abs(v - 90) < 1e-6), 'bucket with break preserves max extremum');
    assert.ok(posValues.some(v => Math.abs(v - 2) < 1e-6), 'tied-hour bucket preserves min extremum');
    assert.ok(posValues.some(v => Math.abs(v - 85) < 1e-6), 'tied-hour bucket preserves max extremum');

    const headLeakSeries: BoundedSeries = {
      ...seriesOf(100, 190, 0.2),
      headValue: knownValue(0.8),
    };
    const headLeakView = createBoundedReadView({
      hour: displayedHour(300),
      series: new Map([['parts', new Map<string, BoundedSeries>([
        [keyId({ tag: 'GER', part: TOTAL_PART }), seriesOf(100, 200, 0.2)],
        [keyId({ tag: 'GER', part: POS_PART }), headLeakSeries],
      ])]]),
    });
    const headLeakModel = buildStabilityPartsChartModel(headLeakView, baseOptions({ parts: [TOTAL_PART, POS_PART], extent: [100, 200] }));
    const leakValues = headLeakModel.stack!.bands.flatMap(b => b.values).filter((v): v is number => v !== null);
    assert.ok(!leakValues.some(v => Math.abs(v - 80) < 1e-6), 'range end never stamps future headValue from hour 300 onto hour 200');

    const collidingBuckets = [
      {
        firstHour: 100,
        lastHour: 147,
        firstValue: 0.1,
        lastValue: 0.15,
        lastValueHour: 140,
        minHour: 100,
        minValue: 0.03,
        maxHour: 100,
        maxValue: 0.88,
        firstBreak: false,
        hasBreak: true,
      },
      {
        firstHour: 148,
        lastHour: 195,
        firstValue: 0.1,
        lastValue: 0.4,
        lastValueHour: 170,
        minHour: 148,
        minValue: 0.02,
        maxHour: 148,
        maxValue: 0.85,
        firstBreak: false,
        hasBreak: false,
      },
    ];
    const collidingSeries: BoundedSeries = {
      tag: 'GER',
      decimals: 5,
      binHours: 48,
      from: 100,
      validInterval: [100, 195],
      runs: emptySlice(),
      buckets: createBoundedSlice(collidingBuckets, collidingBuckets.length),
      missing: emptySlice(),
      straddling: 'none',
    };
    const collidingView = createBoundedReadView({
      hour: displayedHour(200),
      series: new Map([['parts', new Map<string, BoundedSeries>([
        [keyId({ tag: 'GER', part: TOTAL_PART }), seriesOf(100, 195, 0.1)],
        [keyId({ tag: 'GER', part: POS_PART }), collidingSeries],
      ])]]),
    });
    const collidingModel = buildStabilityPartsChartModel(collidingView, baseOptions({ parts: [TOTAL_PART, POS_PART], extent: [100, 200] }));
    const collidingBand = collidingModel.stack!.bands[0]!;
    const collidingVals = collidingBand.values.filter((v): v is number => v !== null);
    for (const expected of [10, 3, 88, 15, 2, 85, 40]) {
      assert.ok(collidingVals.some(v => Math.abs(v - expected) < 1e-6), `colliding bucket preserves ${expected}%`);
    }
    const idx170 = collidingModel.stack!.hours.indexOf(170);
    assert.ok(idx170 >= 0 && Math.abs((collidingBand.values[idx170] ?? 0) - 40) < 1e-6, 'last plateau starts at lastValueHour 170, not lastHour 195');
    const collidingLayout = layoutTimeChart(collidingModel, 600, 220, '12px sans-serif', measure);
    const reducedTip = deriveTimeChartTooltip(collidingModel, collidingLayout, 'plot:101');
    assert.equal(reducedTip?.payload?.items?.[0]?.missingReason, 'This hour is inside a grouped span; its figures come from the read of this hour');

    const zeroObservedView = createBoundedReadView({
      hour: displayedHour(200),
      series: new Map([['parts', new Map<string, BoundedSeries>([
        [keyId({ tag: 'GER', part: TOTAL_PART }), seriesOf(100, 200, 0)],
      ])]]),
    });
    const zeroObservedModel = buildStabilityPartsChartModel(zeroObservedView, baseOptions({ parts: [TOTAL_PART], extent: [100, 200] }));
    assert.equal(zeroObservedModel.empty?.message, 'No active modifiers in this time.', 'observed zero stability is not reported as unrecorded');

    const straddlingTotal: BoundedSeries = {
      tag: 'GER',
      decimals: 5,
      binHours: 6,
      from: 0,
      validInterval: [0, 2],
      runs: emptySlice(),
      buckets: emptySlice(),
      missing: emptySlice(),
      straddling: 'loading',
    };
    const straddlingModel = buildStabilityPartsChartModel(
      createBoundedReadView({
        hour: displayedHour(2),
        series: new Map([['parts', new Map<string, BoundedSeries>([[keyId({ tag: 'GER', part: TOTAL_PART }), straddlingTotal]])]]),
      }),
      baseOptions({ parts: [TOTAL_PART], extent: [0, 100] }),
    );
    assert.equal(straddlingModel.empty, undefined, 'straddling loading bin omits empty instead of reporting breakdown unrecorded');

    const windowedModel = buildStabilityPartsChartModel(
      signedView,
      baseOptions({ parts: [TOTAL_PART, POS_PART], extent: [0, 1000], windowHours: 168 }),
    );
    assert.deepEqual(windowedModel.view?.x, [832, 1000], 'recent windowHours propagates to Stability parts chart view');

    const MOD_A = '10';
    const MOD_B = '11';
    const keyTotal = keyId({ tag: 'GER', part: TOTAL_PART });
    const keyA = keyId({ tag: 'GER', part: MOD_A });
    const keyB = keyId({ tag: 'GER', part: MOD_B });
    const openBucket = (firstValue: number, minValue: number, maxValue: number, lastValue: number) => ({
      firstHour: 0,
      lastHour: 17,
      firstValue,
      lastValue,
      lastValueHour: 5,
      minHour: firstValue === minValue ? 0 : 5,
      minValue,
      maxHour: firstValue === maxValue ? 0 : 5,
      maxValue,
      firstBreak: false,
      hasBreak: false,
    });
    const openTiles = new SceneTiles();
    const openSeries = openTiles.series(
      {
        widget: 1,
        subs: [],
        pictures: [],
        model: {},
        hour: 6,
        seek: 0,
        speed: 0,
        binding: { generation: 1, surfaces: [{ width: 600, height: 220, dpr: 1 }], offset: 0, pin: 6 },
        inspect: null,
        now: 1000,
      } as never,
      {
        sources: {},
        staticPanels: {
          parts: {
            branchId: '1',
            decimals: 5,
            states: [
              { country: 'GER', part: TOTAL_PART, hour: 6, run: { firstHour: 0, lastHour: 17, valueRaw: '50000' }, drop: null, missing: null },
              { country: 'GER', part: MOD_A, hour: 6, run: { firstHour: 5, lastHour: 17, valueRaw: '30000' }, drop: null, missing: null },
              { country: 'GER', part: MOD_B, hour: 6, run: { firstHour: 5, lastHour: 17, valueRaw: '20000' }, drop: null, missing: null },
            ],
          },
        },
      } as never,
      {
        tiledBySource: {
          parts: {
            [keyTotal]: { source: 'parts', tag: keyTotal, head: 'h', decimals: 5, binHours: 18, from: 0, to: 17, width: 1, buckets: [openBucket(0.5, 0.5, 0.5, 0.5)], missing: [], parts: [] },
            [keyA]: { source: 'parts', tag: keyA, head: 'h', decimals: 5, binHours: 18, from: 0, to: 17, width: 1, buckets: [openBucket(0.2, 0.2, 0.3, 0.3)], missing: [], parts: [] },
            [keyB]: { source: 'parts', tag: keyB, head: 'h', decimals: 5, binHours: 18, from: 0, to: 17, width: 1, buckets: [openBucket(0.3, 0.2, 0.3, 0.2)], missing: [], parts: [] },
          },
        },
        headSourceBySeries: {
          parts: {
            answerName: 'parts',
            tags: ['GER'],
            keys: [{ tag: 'GER', part: TOTAL_PART }, { tag: 'GER', part: MOD_A }, { tag: 'GER', part: MOD_B }],
          },
        },
      },
      displayedHour(6),
    );
    const openModel = buildStabilityPartsChartModel(
      createBoundedReadView({ hour: displayedHour(6), series: openSeries }),
      baseOptions({
        parts: [TOTAL_PART, MOD_A, MOD_B],
        names: [
          ...NAMES,
          { part: MOD_A, measure: 'stability', kind: 'term', id: 'mod-a', term: 'Base', line: null, missing: null },
          { part: MOD_B, measure: 'stability', kind: 'term', id: 'mod-b', term: 'Popularity', line: null, missing: null },
        ],
        extent: [0, 17],
      }),
    );
    const openHours = openModel.stack!.hours;
    const bandA = openModel.stack!.bands.find(b => b.id === 'mod-a')!;
    const bandB = openModel.stack!.bands.find(b => b.id === 'mod-b')!;
    const idx0 = openHours.indexOf(0);
    const idx5 = openHours.indexOf(5);
    assert.ok(idx0 >= 0 && idx5 >= 0, 'open coarse bucket emits cuts at 0 and 5');
    assert.equal(bandA.values[idx0], null, 'unresolved hours 0..4 stay missing instead of drawing 0%');
    assert.equal(bandB.values[idx0], null, 'unresolved hours 0..4 stay missing instead of drawing 0%');
    assert.equal(bandA.values[idx5], 30, 'head run at 5..6 draws modifier A at 30%');
    assert.equal(bandB.values[idx5], 20, 'head run at 5..6 draws modifier B at 20%');

    const closedBreakBucket: BoundedSeries = {
      tag: keyA,
      decimals: 5,
      binHours: 18,
      from: 0,
      validInterval: [0, 17],
      runs: emptySlice(),
      buckets: createBoundedSlice([{
        firstHour: 0,
        lastHour: 17,
        firstValue: 0.1,
        lastValue: 0.5,
        lastValueHour: 12,
        minHour: 0,
        minValue: 0.1,
        maxHour: 12,
        maxValue: 0.5,
        firstBreak: false,
        hasBreak: true,
      }], 1),
      missing: emptySlice(),
      straddling: 'none',
    };
    const closedBreakModel = buildStabilityPartsChartModel(
      createBoundedReadView({
        hour: displayedHour(17),
        series: new Map([['parts', new Map<string, BoundedSeries>([
          [keyTotal, seriesOf(0, 17, 0.5)],
          [keyA, closedBreakBucket],
        ])]]),
      }),
      baseOptions({
        parts: [TOTAL_PART, MOD_A],
        names: [
          ...NAMES,
          { part: MOD_A, measure: 'stability', kind: 'term', id: 'mod-a', term: 'Base', line: null, missing: null },
        ],
        extent: [0, 17],
      }),
    );
    const cbHours = closedBreakModel.stack!.hours;
    const cbBand = closedBreakModel.stack!.bands.find(b => b.id === 'mod-a')!;
    assert.equal(cbBand.values[cbHours.indexOf(0)], 10, 'closed break bucket keeps witnessed first sample');
    assert.equal(cbBand.values[cbHours.indexOf(1)], null, 'unresolved hours 1..11 inside closed break bucket stay null instead of 0%');
    assert.equal(cbBand.values[cbHours.indexOf(12)], 50, 'closed break bucket keeps sample at 12');
    assert.equal(cbBand.values[cbHours.indexOf(13)], null, 'hours 13..17 after summary boundary stay null instead of 0% or invented run');
  });

  it('surfaces recorded missing reasons on chart and table, builds exact series-next-change nav params, and resolves both Stability drawer kinds in a fresh worker process', () => {
    const missingTotal: BoundedSeries = {
      tag: keyId({ tag: 'GER', part: TOTAL_PART }),
      decimals: 5,
      binHours: 1,
      from: 0,
      validInterval: [0, 100],
      runs: emptySlice(),
      buckets: emptySlice(),
      missing: createBoundedSlice([{ firstHour: 0, lastHour: 100, reason: 'Stability parts were not recorded in this build' }], 1),
      straddling: 'none',
    };
    const missingChart = buildStabilityPartsChartModel(
      createBoundedReadView({
        hour: displayedHour(50),
        series: new Map([['parts', new Map([[keyId({ tag: 'GER', part: TOTAL_PART }), missingTotal]])]]),
      }),
      baseOptions({ parts: [TOTAL_PART], extent: [0, 100] }),
    );
    assert.deepEqual(missingChart.empty, {
      message: 'Stability parts were not recorded in this build',
      kind: 'unavailable',
    }, 'wholly-missing tile series surfaces its recorded reason as unavailable');

    const listedMissingChart = buildStabilityPartsChartModel(
      createBoundedReadView({ hour: displayedHour(50), series: new Map() }),
      baseOptions({ parts: [], extent: [0, 100], missingReasons: ['Breakdown table absent in older format'] }),
    );
    assert.deepEqual(listedMissingChart.empty, {
      message: 'Breakdown table absent in older format',
      kind: 'unavailable',
    }, 'empty part catalogue with listed.missing surfaces its recorded reason as unavailable');

    const missingTable = buildStabilityPartsTableModel(
      50,
      [{ tag: 'GER', part: TOTAL_PART, hour: 50, drop: null, missing: 'Breakdown unread at this hour', run: null }],
      baseOptions({ parts: [TOTAL_PART], extent: [0, 100], missingReasons: ['Other hour gap'] }),
    );
    assert.equal(missingTable.empty, 'Breakdown unread at this hour');
    assert.equal(missingTable.emptyKind, 'unavailable');

    const followedNotRecorded = stabilityPartsTableProjection.project(
      createBoundedReadView({
        hour: displayedHour(50),
        answers: new Map([['state', { kind: 'notRecorded', reason: 'Stability parts withheld on this game build' }]]),
      }),
      baseOptions({ parts: [TOTAL_PART], extent: [0, 100] }),
    );
    assert.equal(followedNotRecorded.empty, 'Stability parts withheld on this game build');
    assert.equal(followedNotRecorded.emptyKind, 'unavailable');

    const slot7 = countryKey(7);
    const navParams = buildStabilityNextChangeParams({
      branchId: '1',
      owner: { key: slot7, spans: [{ firstHour: 10, lastHour: 80 }] },
      parts: [TOTAL_PART, POS_PART],
      key: 'ArrowRight',
      fromHour: 24,
      visible: [0, 120],
      playhead: 60,
      gate: {
        span: (first, last) => (first <= last ? { firstHour: first, lastHour: last } as never : null),
        within: hour => (hour as never),
      },
    });
    assert.deepEqual(navParams, {
      branchId: '1',
      family: PARTS_FAMILY,
      lines: [
        { country: 7, part: TOTAL_PART },
        { country: 7, part: POS_PART },
      ],
      within: [
        [7, [{ firstHour: 10, lastHour: 80 }]],
      ],
      key: 'ArrowRight',
      from: 24,
      firstHour: 0,
      lastHour: 60,
    }, 'StabilityDrill keyboard nav targets series-next-change with dated slot, within spans, and playhead-clamped visible range');

    const drawnUrl = pathToFileURL(fileURLToPath(new URL('../../draw/views/drawn.ts', import.meta.url))).href;
    const registryUrl = pathToFileURL(fileURLToPath(new URL('../../families/registry.ts', import.meta.url))).href;
    const probe = [
      `import { drawnViews } from ${JSON.stringify(drawnUrl)};`,
      `import { ensureFamilyProjection } from ${JSON.stringify(registryUrl)};`,
      `drawnViews('table', () => {});`,
      `const checks = [`,
      `  ['table', ${JSON.stringify(STABILITY_PARTS_TABLE_KIND)}],`,
      `  ['time-chart', ${JSON.stringify(STABILITY_PARTS_CHART_KIND)}],`,
      `];`,
      `await Promise.all(checks.map(([family, kind]) => new Promise((resolve, reject) => {`,
      `  const first = ensureFamilyProjection(family, kind, undefined, resolve);`,
      `  if (first === 'unknown') reject(new Error(\`worker unknown: \${family}/\${kind}\`));`,
      `  else if (first !== 'loading') resolve();`,
      `})));`,
      `const resolved = Object.fromEntries(checks.map(([family, kind]) => {`,
      `  const loaded = ensureFamilyProjection(family, kind, undefined, () => {});`,
      `  return [kind, typeof loaded === 'object' && loaded.kind === kind];`,
      `}));`,
      `console.log(JSON.stringify(resolved));`,
    ].join('\n');
    const stdout = execFileSync(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', probe], { encoding: 'utf8' });
    const resolved = JSON.parse(stdout.trim()) as Record<string, boolean>;
    assert.equal(resolved[STABILITY_PARTS_TABLE_KIND], true, 'worker resolves stability-parts-table projection');
    assert.equal(resolved[STABILITY_PARTS_CHART_KIND], true, 'worker resolves stability-parts-chart projection');
  });
});
