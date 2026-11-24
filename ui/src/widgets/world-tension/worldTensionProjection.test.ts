import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { IdentityIndex } from '../../families/identity/index.ts';
import { createSnapshotIdentityProvider } from '../../families/identity/provider.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart } from '../../families/time-chart/layout.ts';
import { buildLineSubpaths, drawTimeChart } from '../../families/time-chart/painter.ts';
import type { Canvas2D, MeasureText } from '../../charts/textWidth.ts';
import type { WorldTensionReply, WorldTensionStackReply } from '../../generated/dashboard';
import { worldTensionEventsDeclaration, worldTensionEventsProjection } from './worldTensionEventsProjection.ts';
import { worldTensionLinesDeclaration, worldTensionLinesProjection } from './worldTensionLinesProjection.ts';
import { worldTensionDeclaration, worldTensionProjection } from './worldTensionProjection.ts';

const H1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const H1941 = engineHour({ year: 1941, month: 9, day: 15, hour: 1 })!;
const measure: MeasureText = text => text.length * 6.5;

function makeIdentity() {
  const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [H1936, H1941 + 1000]);
  index.setFacts({
    branchId: 'b',
    known: [[H1936, H1941 + 1000]],
    names: ['German Reich', 'Japan', 'Italy'],
    reasons: [],
    flags: [
      { keys: ['GER_fascism', null, null], reasons: [null, null, null] },
      { keys: ['JAP_fascism', null, null], reasons: [null, null, null] },
      { keys: ['ITA_fascism', null, null], reasons: [null, null, null] },
    ],
    countries: [
      { tag: 'GER', runs: [[H1936, 0, 0x454545, -1, 0]] },
      { tag: 'JAP', runs: [[H1936, 1, 0xcc4444, -1, 1]] },
      { tag: 'ITA', runs: [[H1936, 2, 0x44aa55, -1, 2]] },
    ],
    warnings: [],
  });
  return createSnapshotIdentityProvider(index);
}

function sampleStack(): WorldTensionStackReply {
  return {
    branchId: '1',
    tensionScale: 1000,
    contributionScale: 100000,
    ceilingRaw: '10000000',
    endHour: H1941,
    cappedEndTension: 100,
    cappedEndTensionRaw: '100000',
    endTotalRaw: '100000',
    endSumRaw: '10000000',
    downsampled: false,
    refused: null,
    hours: [H1936, H1936 + 5000, H1936 + 20000],
    total: [0, 35, 100],
    totalRaw: ['0', '35000', '100000'],
    sumRaw: ['0', '3500000', '10000000'],
    bands: [
      { slot: 1, tag: 'GER', remembered: false, values: [0, 20, 60], valuesRaw: ['0', '2000000', '6000000'], minValues: [], maxValues: [], endValue: 60, endRaw: '6000000' },
      { slot: 2, tag: 'JAP', remembered: false, values: [0, 10, 25], valuesRaw: ['0', '1000000', '2500000'], minValues: [], maxValues: [], endValue: 25, endRaw: '2500000' },
      { slot: 3, tag: 'ITA', remembered: false, values: [0, 2, 3], valuesRaw: ['0', '200000', '300000'], minValues: [], maxValues: [], endValue: 3, endRaw: '300000' },
    ],
    other: { count: 31, values: [0, 3, 12], valuesRaw: ['0', '300000', '1200000'], endValue: 12, endRaw: '1200000' },
    events: [
      { hour: H1936 + 5000, slot: 1, secondCountry: 0, place: 0, tag: 'GER', remembered: false, reason: 'Remilitarised the Rhineland', addedRaw: '500000', firstRead: H1936 + 5000, lastRead: null },
      { hour: H1941 + 200, slot: 2, secondCountry: 0, place: 0, tag: 'JAP', remembered: false, reason: 'Future event', addedRaw: '1000000', firstRead: H1941 + 200, lastRead: null },
    ],
    missing: [],
    nextChangeHour: H1941 + 24,
    warnings: [],
  };
}

describe('world-tension projections', () => {
  it('projects stepped stack with Other at bottom, leader on top, shortName at small, and clamps future events', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const model = worldTensionProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: true,
      gaps: [],
    });
    assert.equal(model.mode, 'stack');
    assert.equal(model.stack?.step, true);
    assert.equal(model.stack?.bands[0]?.key, 'slot:1');
    assert.equal(model.stack?.bands.at(-1)?.id, 'other');
    assert.equal(model.stack?.bands.at(-1)?.endLabel?.name, 'Other (31)');
    assert.equal(model.stack?.bands.at(-1)?.endLabel?.shortName, 'Other');
    assert.equal(model.stack?.total?.endLabel?.value, '100%');
    assert.equal(model.markers?.length, 1, 'future event after displayed hour is excluded');

    const fullLayout = layoutTimeChart(model, 960, 280, 'Segoe UI', measure);
    assert.ok(fullLayout.endLabels.some(l => l.name === 'Other (31)'));
    const totalLabel = fullLayout.endLabels.find(l => l.seriesId === '__total__')!;
    const labelHit = hitTestTimeChart(model, fullLayout, [totalLabel.x + 10, totalLabel.y]);
    assert.equal(labelHit, 'label:__total__');
    const tip = deriveTimeChartTooltip(model, fullLayout, labelHit);
    assert.equal(tip?.payload?.title, 'World tension');

    const minLayout = layoutTimeChart(model, 476, 186, 'Segoe UI', measure);
    assert.ok(minLayout.endLabels.some(l => l.name === 'Other'), 'min layout uses shortName Other');
    assert.equal(minLayout.endLabels.some(l => l.seriesId === 'slot:3'), false, 'thin <10px band label hidden at min size');

    const ops: string[] = [];
    const ctx = new Proxy({}, {
      get: (_t, prop) => {
        if (prop === 'measureText') return (t: string) => ({ width: t.length * 6.5 });
        return (...args: unknown[]) => { ops.push(String(prop)); return args; };
      },
    }) as unknown as Canvas2D;
    drawTimeChart(ctx, model, fullLayout, undefined, `plot:${H1936 + 5000}`, 1);
    assert.ok(ops.includes('fill') && ops.includes('arc'), 'stepped stack paints bands and hover dot');
  });

  it('projects world-tension events table view with dated events clipped to displayed hour', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const table = worldTensionEventsProjection.project(readView, { branchFirst: H1936 });
    assert.equal(table.rows.length, 1);
    assert.equal(table.rows[0]?.name, 'Remilitarised the Rhineland');
  });

  it('projects world-tension lines view with top-8 coloured lines and stepped inline points', () => {
    const reply = sampleStack();
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      identity: makeIdentity(),
    });
    const model = worldTensionLinesProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    assert.equal(model.mode, 'lines');
    assert.equal(model.series.length, 3);
    assert.equal(model.series[0]?.key, 'slot:1');
    assert.equal(model.series[0]?.step, true);
    assert.equal(model.series[0]?.endLabel?.value, '60%');
    assert.equal(model.series[0]?.points?.length, 3);
  });

  it('K1: world-tension preserves small shares as nonzero and never turns non-finite inputs into 0%', () => {
    const reply = sampleStack();
    const withEdge = {
      ...reply,
      bands: [
        { slot: 1, tag: 'GER', remembered: false, values: [0, 0.4, 0.4], valuesRaw: ['0', '40000', '40000'], minValues: [], maxValues: [], endValue: 0.4, endRaw: '40000' },
        { slot: 2, tag: 'JAP', remembered: false, values: [0, 0.04, 0.04], valuesRaw: ['0', '4000', '4000'], minValues: [], maxValues: [], endValue: 0.04, endRaw: '4000' },
        { slot: 3, tag: 'ITA', remembered: false, values: [0, 1, 1], valuesRaw: ['0', '100000', '100000'], minValues: [], maxValues: [], endValue: Number.NaN, endRaw: '0' },
      ],
      other: null,
    };
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', withEdge]]),
      identity: makeIdentity(),
    });
    const model = worldTensionProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: false,
      gaps: [],
    });
    assert.equal(model.stack?.bands[0]?.endLabel?.value, '0.4%');
    assert.equal(model.stack?.bands[1]?.endLabel?.value, '<0.1%');
    assert.equal(model.stack?.bands[2]?.endLabel, null);
  });

  it('S2: stacks negative contributions below zero, keeps recorded world total in label and audit, preserves witnessed bands without world scalar, and omits end labels during a gap', () => {
    const negReply: WorldTensionStackReply = {
      branchId: '1',
      tensionScale: 1000,
      contributionScale: 100000,
      ceilingRaw: '10000000',
      endHour: H1936 + 5000,
      cappedEndTension: 6.5,
      cappedEndTensionRaw: '6500',
      endTotalRaw: '650000',
      endSumRaw: '650000',
      downsampled: false,
      refused: null,
      hours: [H1936, H1936 + 5000],
      total: [0, 6.5],
      totalRaw: ['0', '650000'],
      sumRaw: ['0', '650000'],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, values: [0, 4], valuesRaw: ['0', '400000'], minValues: [], maxValues: [], endValue: 4, endRaw: '400000' },
        { slot: 2, tag: 'JAP', remembered: false, values: [0, 3], valuesRaw: ['0', '300000'], minValues: [], maxValues: [], endValue: 3, endRaw: '300000' },
        { slot: 3, tag: 'ITA', remembered: false, values: [0, -0.5], valuesRaw: ['0', '-50000'], minValues: [], maxValues: [], endValue: -0.5, endRaw: '-50000' },
      ],
      other: null,
      events: [],
      missing: [],
      nextChangeHour: null,
      warnings: [],
    };
    const negModel = worldTensionProjection.project(
      createBoundedReadView({ hour: H1936 + 5000, panels: new Map([['main', negReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5000], surface: '#111111', layers: 5, values: 'recorded', showMarkers: false, gaps: [] },
    );
    assert.equal(negModel.yAxis.min, undefined, 'negative contributions must not be clipped at min: 0');
    const negBand = negModel.stack?.bands.find(b => b.id === 'slot:3');
    assert.ok(negBand?.endLabel, 'negative contribution band keeps its end label');
    assert.equal(negModel.stack?.total?.endLabel?.value, '6.5%', 'recorded world total label is 6.5%');
    assert.ok(negModel.auditRows?.some(r => r.row === '__total__' && r.value === '6.5%'));
    assert.ok(negModel.auditRows?.some(r => r.row === 'slot:3' && r.value === '\u22120.5%'), 'negative contribution appears in audit');

    const negLayout = layoutTimeChart(negModel, 720, 260, 'Segoe UI', measure);
    assert.ok(negLayout.yDomain[0] < 0, 'stack yDomain includes room below zero for negative bands');
    const itaLabel = negLayout.endLabels.find(l => l.seriesId === 'slot:3');
    const gerLabel = negLayout.endLabels.find(l => l.seriesId === 'slot:1');
    assert.ok(itaLabel && gerLabel, 'both positive and negative band labels are placed');
    assert.ok(itaLabel!.anchorY > negLayout.toY(0), 'negative band anchors below the zero axis');
    assert.ok(gerLabel!.anchorY < negLayout.toY(0), 'positive band anchors above the zero axis');

    const negLines = worldTensionLinesProjection.project(
      createBoundedReadView({ hour: H1936 + 5000, panels: new Map([['main', negReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5000], surface: '#111111', gaps: [] },
    );
    assert.equal(negLines.yAxis.min, undefined, 'lines view must not clip negative contributions at 0');

    const noWorldScalar: WorldTensionStackReply = {
      ...negReply,
      cappedEndTension: null,
      cappedEndTensionRaw: null,
      endTotalRaw: null,
      total: [null, null],
      totalRaw: [null, null],
    };
    const witnessedModel = worldTensionProjection.project(
      createBoundedReadView({ hour: H1936 + 5000, panels: new Map([['main', noWorldScalar]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5000], surface: '#111111', layers: 5, values: 'recorded', showMarkers: false, gaps: [] },
    );
    assert.equal(witnessedModel.empty, undefined, 'missing world scalar must not blank independently witnessed contributions');
    assert.equal(witnessedModel.stack?.bands.length, 3);

    const gapReply: WorldTensionStackReply = {
      ...negReply,
      endHour: null,
      cappedEndTension: null,
      cappedEndTensionRaw: null,
      endTotalRaw: null,
      endSumRaw: null,
      hours: [H1936, H1936 + 5000, H1936 + 5010],
      total: [0, 6.5, null],
      totalRaw: ['0', '650000', null],
      sumRaw: ['0', '650000', null],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, values: [0, 4, null], valuesRaw: ['0', '400000', null], minValues: [], maxValues: [], endValue: null, endRaw: null },
        { slot: 2, tag: 'JAP', remembered: false, values: [0, 3, null], valuesRaw: ['0', '300000', null], minValues: [], maxValues: [], endValue: null, endRaw: null },
        { slot: 3, tag: 'ITA', remembered: false, values: [0, -0.5, null], valuesRaw: ['0', '-50000', null], minValues: [], maxValues: [], endValue: null, endRaw: null },
      ],
      missing: [{ firstHour: H1936 + 5010, lastHour: H1936 + 5020, reason: 'Recording gap' }],
    };
    const gapStack = worldTensionProjection.project(
      createBoundedReadView({ hour: H1936 + 5015, panels: new Map([['main', gapReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5015], surface: '#111111', layers: 5, values: 'recorded', showMarkers: false, gaps: [] },
    );
    assert.equal(gapStack.stack?.total?.endLabel, null, 'total end label is absent during a gap');
    assert.equal(gapStack.auditRows?.length, 0, 'no stale audit rows during a gap');
    const gapLines = worldTensionLinesProjection.project(
      createBoundedReadView({ hour: H1936 + 5015, panels: new Map([['main', gapReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1936 + 5015], surface: '#111111', gaps: [] },
    );
    assert.ok(gapLines.series.every(s => s.endLabel === null), 'lines omit end labels during a gap');
  });

  it('R2-K1: hovering a stack prints each figure by its kind: a recorded -2% contribution and an uncapped 125% total, in both value modes', () => {
    const at = H1936 + 5000;
    const reply: WorldTensionStackReply = {
      ...sampleStack(),
      endHour: at,
      cappedEndTension: 100,
      hours: [H1936, at],
      total: [0, 125],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, values: [0, 127], minValues: [], maxValues: [], endValue: 127, endRaw: '12700000' },
        { slot: 2, tag: 'JAP', remembered: false, values: [0, -2], minValues: [], maxValues: [], endValue: -2, endRaw: '-200000' },
      ],
      other: null,
      events: [],
      nextChangeHour: null,
    };
    for (const values of ['recorded', 'share'] as const) {
      const model = worldTensionProjection.project(
        createBoundedReadView({ hour: at, panels: new Map([['main', reply]]), identity: makeIdentity() }),
        { extent: [H1936, at], surface: '#111111', layers: 5, values, showMarkers: false, gaps: [] },
      );
      const layout = layoutTimeChart(model, 960, 280, 'Segoe UI', measure);
      const ger = layout.endLabels.find(l => l.seriesId === 'slot:1')!;
      const labelTip = deriveTimeChartTooltip(model, layout, hitTestTimeChart(model, layout, [ger.x + 10, ger.y]));
      assert.deepEqual((labelTip?.payload?.items ?? []).map(item => [item.label, item.value]), [['German Reich', '+127%']], values);
      const plotTip = deriveTimeChartTooltip(model, layout, `plot:${at}`);
      assert.deepEqual((plotTip?.payload?.items ?? []).map(item => [item.label, item.value]), [
        ['World tension', '125%'],
        ['German Reich', '+127%'],
        ['Japan', '−2%'],
      ], values);
    }
  });

  it('S10: stack hover reads same-hour events from the hovered-hour world-tension reply, not later playhead revisions', () => {
    const hoverHour = H1936 + 5000;
    const reply: WorldTensionStackReply = {
      ...sampleStack(),
      events: [
        { hour: hoverHour, slot: 1, secondCountry: 0, place: 0, tag: 'GER', remembered: false, reason: 'Remilitarised the Rhineland', addedRaw: '900000', firstRead: hoverHour, lastRead: null },
      ],
    };
    const hoverReply: WorldTensionReply = {
      tensionScale: 1000,
      contributionScale: 100000,
      ceilingRaw: '10000000',
      unnamed: false,
      contributions: [{ slot: 1, tag: 'GER', remembered: false }],
      sources: [
        { ordinal: 0, tag: 'GER', remembered: false, reason: 'Remilitarised the Rhineland', finalRaw: '500000', dailyRaw: '0', date: hoverHour },
      ],
      missing: null,
      warnings: [],
    };
    const livePayload = {
      live: {
        groups: [
          { table: 1, columns: [0], keys: ['0'], values: [5000] },
          { table: 2, columns: [0], keys: ['0'], values: [500000] },
          { table: 3, columns: [0], keys: ['1'], values: [500000] },
        ],
        rows: ['world', '1'],
        kinds: [
          { id: 'tension', op: 'value', scale: 1 },
          { id: 'sum', op: 'value', scale: 1 },
          { id: 'value', op: 'value', scale: 1 },
        ],
        cells: [
          0, 0, 1, 0, 0, 0,
          0, 1, 1, 1, 0, 0,
          1, 2, 1, 2, 0, 0,
        ],
      },
    };
    const readView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', reply]]),
      hovers: new Map([['hover', { hour: hoverHour, data: { ...hoverReply, ...livePayload } }]]),
      identity: makeIdentity(),
    });
    const model = worldTensionProjection.project(readView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: true,
      gaps: [],
    });
    const evItem = model.hoverDetail?.items.find(item => item.label === 'Remilitarised the Rhineland');
    assert.equal(evItem?.value, '+5%', 'hover shows +5% recorded at hoverHour, not +9% revised later at playhead');
  });

  it('S11: downsampled world-tension lines paint interval bounds, hover as ranges before exact hover, and show exact hovered-hour contributions once loaded', () => {
    const hoverHour = H1936 + 5000;
    const downsampledReply = {
      ...sampleStack(),
      downsampled: true,
      hours: [H1936, hoverHour - 10, H1941],
      total: [0, 20, 60],
      bands: [
        {
          slot: 1,
          tag: 'GER',
          remembered: false,
          values: [0, 20, 60],
          minValues: [0, 10, 60],
          maxValues: [0, 40, 60],
          endValue: 60,
          endRaw: '6000000',
        },
      ],
      other: null,
    } as WorldTensionStackReply;

    const noHoverView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', downsampledReply]]),
      identity: makeIdentity(),
    });
    const lineModel = worldTensionLinesProjection.project(noHoverView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    const layout = layoutTimeChart(lineModel, 720, 260, 'Segoe UI', measure);
    const tipWithoutExact = deriveTimeChartTooltip(lineModel, layout, `plot:${hoverHour}`);
    const rangeItem = tipWithoutExact?.payload?.items?.find(i => i.label === 'German Reich');
    assert.ok(rangeItem?.value?.includes('\u2013'), `expected interval bounds range on downsampled hover, got ${rangeItem?.value}`);

    const { subpaths } = buildLineSubpaths(lineModel.series[0]!, layout, H1941);
    const ys = subpaths.flat().map(p => p[1]);
    assert.ok(ys.some(y => Math.abs(y - layout.toY(40)) < 0.5), 'painter includes max excursion 40%');
    assert.ok(ys.some(y => Math.abs(y - layout.toY(10)) < 0.5), 'painter includes min excursion 10%');

    const hoverReply = {
      tensionScale: 1000,
      contributionScale: 100000,
      ceilingRaw: '10000000',
      unnamed: false,
      contributions: [{ slot: 1, tag: 'GER', remembered: false }],
      sources: [],
      missing: null,
      warnings: [],
      live: {
        groups: [{ table: 3, columns: [0], keys: ['1'], values: [3500000] }],
        rows: ['1'],
        kinds: [{ id: 'value', op: 'value', scale: 1 }],
        cells: [0, 0, 1, 0, 0, 0],
      },
    };
    const exactView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', downsampledReply]]),
      hovers: new Map([['hover', { hour: hoverHour, data: hoverReply }]]),
      identity: makeIdentity(),
    });
    const exactModel = worldTensionLinesProjection.project(exactView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    const exactLayout = layoutTimeChart(exactModel, 720, 260, 'Segoe UI', measure);
    const exactTip = deriveTimeChartTooltip(exactModel, exactLayout, `plot:${hoverHour}`);
    const exactItem = exactTip?.payload?.items?.find(i => i.label === 'German Reich');
    assert.equal(exactItem?.value, '+35%', 'exact hover overrides downsampled range with the signed slot contribution');

    const plan = worldTensionLinesDeclaration.sources({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1', firstHour: H1936, lastHour: H1941, gaps: [] },
      countries: { all: true, tags: [], top: null },
      options: {},
      widget: { countries: { kind: 'all' } },
      gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b }),
    } as never, { width: 800, height: 300, following: false, staticSpan: null });
    assert.ok(plan.entries.some(e => e.name === 'hover' && e.read.shape === 'exact-hover'), 'lines view declares exact-hover entry');
  });

  it('S14 + R3-S11: keys world-tension bands and audit rows by slot (even when two slots share a tag), and includes secondary country and place in events table rowKey', () => {
    const sharedTagReply = {
      ...sampleStack(),
      bands: [
        { slot: 1, tag: 'GER', remembered: true, values: [0, 20, 30], minValues: [], maxValues: [], endValue: 30, endRaw: '3000000' },
        { slot: 2, tag: 'GER', remembered: false, values: [0, 10, 40], minValues: [], maxValues: [], endValue: 40, endRaw: '4000000' },
      ],
      other: null,
      events: [
        { hour: H1936 + 5000, slot: 1, secondCountry: 2, place: 0, tag: 'GER', remembered: false, reason: 'Declared war', addedRaw: '500000', firstRead: H1936 + 5000, lastRead: null },
        { hour: H1936 + 5000, slot: 1, secondCountry: 3, place: 0, tag: 'GER', remembered: false, reason: 'Declared war', addedRaw: '400000', firstRead: H1936 + 5000, lastRead: null },
        { hour: H1936 + 5000, slot: 1, secondCountry: 3, place: 1, tag: 'GER', remembered: false, reason: 'Declared war', addedRaw: '300000', firstRead: H1936 + 5000, lastRead: null },
      ],
    } as unknown as WorldTensionStackReply;

    const view = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', sharedTagReply]]),
      identity: makeIdentity(),
    });
    const stackModel = worldTensionProjection.project(view, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'recorded',
      showMarkers: false,
      gaps: [],
    });
    assert.deepEqual(stackModel.stack?.bands.map(b => b.key), ['slot:1', 'slot:2'], 'stack bands key by slot, not tag');
    const stackAuditKeys = (stackModel.auditRows ?? []).map(r => r.row);
    assert.equal(new Set(stackAuditKeys).size, stackAuditKeys.length, 'R3-S11: audit keys must be unique when two slots share a tag');
    assert.ok(stackAuditKeys.includes('slot:1') && stackAuditKeys.includes('slot:2'), 'R3-S11: stack audit keys use slot ids');

    const linesModel = worldTensionLinesProjection.project(view, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    assert.deepEqual(linesModel.series.map(s => s.key), ['slot:1', 'slot:2'], 'lines key by slot, not tag');
    assert.deepEqual(linesModel.series.map(s => s.id), ['slot:1', 'slot:2'], 'lines id by slot, not tag');
    const linesAuditKeys = (linesModel.auditRows ?? []).map(r => r.row);
    assert.deepEqual(linesAuditKeys, ['slot:1', 'slot:2'], 'R3-S11: lines audit keys use slot ids');

    const table = worldTensionEventsProjection.project(view, { branchFirst: H1936 });
    const rowKeys = table.rows.map(r => r.key);
    assert.equal(new Set(rowKeys).size, 3, 'events table rowKey distinguishes secondary country and occurrence ordinal');
  });

  it('S15 + S18: passes top and dated GroupSelection to Rust across Stack/Lines/Events, renders refused replies, and skips unused stack/event work', () => {
    const viewReq = { width: 800, height: 300, following: false, staticSpan: null };
    const mkCtx = (countries: unknown, widgetCountries: unknown, options: Record<string, unknown> = {}) => ({
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
      branch: { id: '1', firstHour: H1936, lastHour: H1941, gaps: [] },
      countries,
      options,
      widget: { countries: widgetCountries },
      gateSpan: (a: number, b: number) => ({ firstHour: a, lastHour: b }),
    } as never);

    const topCtx = mkCtx({ all: false, tags: [], top: 3 }, { kind: 'top', count: 3 });
    const stackTopParams = (worldTensionDeclaration.sources(topCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    const linesTopParams = (worldTensionLinesDeclaration.sources(topCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    const eventsTopParams = (worldTensionEventsDeclaration.sources(topCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    assert.equal(stackTopParams.top, 3, 'Stack passes Top-N count to product');
    assert.equal(stackTopParams.countries, null, 'Stack does not send empty countries array for Top-N');
    assert.equal(linesTopParams.top, 3, 'Lines passes Top-N count to product');
    assert.equal(eventsTopParams.top, 3, 'Events passes Top-N count to product');
    assert.equal(eventsTopParams.includeStack, false, 'S18: Events skips building stack history');
    assert.equal(linesTopParams.includeEvents, false, 'S18: Lines skips scanning events');

    const noMarkersCtx = mkCtx({ all: true, tags: [], top: null }, { kind: 'all' }, { markers: false });
    const stackNoMarkers = (worldTensionDeclaration.sources(noMarkersCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    assert.equal(stackNoMarkers.includeEvents, false, 'S18: Stack with markers off skips scanning events');

    const groupCtx = mkCtx(
      { all: false, tags: ['GER', 'ITA'], top: null },
      { kind: 'group', group: { type: 'faction', id: '42' }, membership: 'hour' },
    );
    const expectedGroup = { type: 'faction', id: '42', membership: 'hour' };
    const stackGroupParams = (worldTensionDeclaration.sources(groupCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    const linesGroupParams = (worldTensionLinesDeclaration.sources(groupCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    const eventsGroupParams = (worldTensionEventsDeclaration.sources(groupCtx, viewReq).entries[0]!.read as { params: Record<string, unknown> }).params;
    assert.deepEqual(stackGroupParams.group, expectedGroup, 'Stack sends GroupSelection directly');
    assert.equal(stackGroupParams.countries, null, 'Stack does not flatten faction group to countries union');
    assert.deepEqual(linesGroupParams.group, expectedGroup, 'Lines sends GroupSelection directly');
    assert.deepEqual(eventsGroupParams.group, expectedGroup, 'Events sends GroupSelection directly');

    const refusedReply = {
      ...sampleStack(),
      refused: 'Membership of this group is not recorded at this hour',
      hours: [],
      total: [],
      bands: [],
      other: null,
      events: [],
    } as unknown as WorldTensionStackReply;
    const refusedView = createBoundedReadView({
      hour: H1941,
      panels: new Map([['main', refusedReply]]),
      identity: makeIdentity(),
    });
    const refusedStack = worldTensionProjection.project(refusedView, {
      extent: [H1936, H1941],
      surface: '#111111',
      layers: 5,
      values: 'share',
      showMarkers: true,
      gaps: [],
    });
    assert.equal(refusedStack.empty?.kind, 'unavailable');
    assert.equal(refusedStack.empty?.message, 'Membership of this group is not recorded at this hour');
    const refusedLines = worldTensionLinesProjection.project(refusedView, {
      extent: [H1936, H1941],
      surface: '#111111',
      gaps: [],
    });
    assert.equal(refusedLines.empty?.kind, 'unavailable');
    assert.equal(refusedLines.empty?.message, 'Membership of this group is not recorded at this hour');
    const refusedEvents = worldTensionEventsProjection.project(refusedView, { branchFirst: H1936 });
    assert.equal(refusedEvents.emptyKind, 'unavailable');
    assert.equal(refusedEvents.empty, 'Membership of this group is not recorded at this hour');
  });

  it('formats total, band shares, Other, lines, tooltips, and audit from raw integers without float drift', () => {
    const driftFloat = (145 / 700) * 70;
    const shareReply = {
      ...sampleStack(),
      cappedEndTension: 70,
      cappedEndTensionRaw: '70000',
      endTotalRaw: '70000',
      endSumRaw: '700',
      hours: [H1936, H1941],
      total: [0, 70],
      totalRaw: ['0', '70000'],
      sumRaw: ['0', '700'],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, values: [0, driftFloat], valuesRaw: ['0', '145'], minValues: [], maxValues: [], endValue: driftFloat, endRaw: '145' },
        { slot: 2, tag: 'JAP', remembered: false, values: [0, 41], valuesRaw: ['0', '410'], minValues: [], maxValues: [], endValue: 41, endRaw: '410' },
      ],
      other: { count: 4, values: [0, driftFloat], valuesRaw: ['0', '145'], endValue: driftFloat, endRaw: '145' },
      events: [],
    } as unknown as WorldTensionStackReply;
    const shareModel = worldTensionProjection.project(
      createBoundedReadView({ hour: H1941, panels: new Map([['main', shareReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1941], surface: '#111111', layers: 5, values: 'share', showMarkers: false, gaps: [] },
    );
    assert.equal(shareModel.stack?.bands[0]?.endLabel?.value, '15%', '145/700 of 70% is exact 14.5% -> 15%, not float 14.499999999999998 -> 14%');
    assert.equal(shareModel.stack?.bands.at(-1)?.endLabel?.value, '15%', 'Other 145/700 of 70% prints 15%, not float 14%');
    assert.equal(shareModel.auditRows?.find(r => r.row === 'slot:1')?.value, '15%');
    assert.equal(shareModel.auditRows?.find(r => r.row === 'other')?.value, '15%');
    const shareLayout = layoutTimeChart(shareModel, 960, 280, 'Segoe UI', measure);
    const shareTip = deriveTimeChartTooltip(shareModel, shareLayout, 'label:__total__');
    assert.equal(shareTip?.payload?.items?.find(i => i.label === 'German Reich')?.value, '+14.5%');
    assert.equal(shareTip?.payload?.items?.find(i => i.label === 'Other (4)')?.value, '+14.5%');

    const bigRaw = '999999999999999999';
    const bigFloat = Number(bigRaw) / 100_000;
    const recReply = {
      ...sampleStack(),
      cappedEndTension: 100,
      cappedEndTensionRaw: '100000',
      endTotalRaw: bigRaw,
      endSumRaw: bigRaw,
      hours: [H1941],
      total: [bigFloat],
      totalRaw: [bigRaw],
      sumRaw: [bigRaw],
      bands: [
        { slot: 1, tag: 'GER', remembered: false, values: [bigFloat], valuesRaw: [bigRaw], minValues: [], maxValues: [], endValue: bigFloat, endRaw: bigRaw },
      ],
      other: { count: 1, values: [bigFloat], valuesRaw: [bigRaw], endValue: bigFloat, endRaw: bigRaw },
      events: [],
    } as unknown as WorldTensionStackReply;
    const recModel = worldTensionProjection.project(
      createBoundedReadView({ hour: H1941, panels: new Map([['main', recReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1941], surface: '#111111', layers: 5, values: 'recorded', showMarkers: false, gaps: [] },
    );
    assert.equal(recModel.stack?.total?.endLabel?.value, '9,999,999,999,999.99999%');
    assert.equal(recModel.stack?.bands[0]?.endLabel?.value, '9,999,999,999,999.99999%');
    assert.equal(recModel.stack?.bands[1]?.endLabel?.value, '9,999,999,999,999.99999%');
    assert.equal(recModel.auditRows?.find(r => r.row === '__total__')?.value, '9,999,999,999,999.99999%');
    assert.equal(recModel.auditRows?.find(r => r.row === 'other')?.value, '9,999,999,999,999.99999%');
    const recLayout = layoutTimeChart(recModel, 960, 280, 'Segoe UI', measure);
    const recTip = deriveTimeChartTooltip(recModel, recLayout, 'label:__total__');
    assert.equal(recTip?.payload?.items?.[0]?.value, '9,999,999,999,999.99999%');
    assert.equal(recTip?.payload?.items?.find(i => i.label === 'German Reich')?.value, '+9,999,999,999,999.99999%');

    const linesModel = worldTensionLinesProjection.project(
      createBoundedReadView({ hour: H1941, panels: new Map([['main', recReply]]), identity: makeIdentity() }),
      { extent: [H1936, H1941], surface: '#111111', gaps: [] },
    );
    assert.equal(linesModel.series[0]?.endLabel?.value, '9,999,999,999,999.99999%');
    assert.equal(linesModel.auditRows?.find(r => r.row === 'slot:1')?.value, '9,999,999,999,999.99999%');
    const linesLayout = layoutTimeChart(linesModel, 960, 280, 'Segoe UI', measure);
    const linesTip = deriveTimeChartTooltip(linesModel, linesLayout, `plot:${H1941}`);
    assert.equal(linesTip?.payload?.items?.find(i => i.label === 'German Reich')?.value, '+9,999,999,999,999.99999%');
  });
});
