import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { ChartBucket } from '../../charts/chartSummary.ts';
import type { Canvas2D, MeasureText } from '../../charts/textWidth.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import { boundTiledSeries, displayedHour } from '../boundedView.ts';
import { countryLooksIdentityProvider } from '../looks.ts';
import { knownValue } from '../value.ts';
import { formatDateOnly } from '../valueFormat.ts';
import { registerTimeChart, type TimeChartBinding } from './declaration.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from './hitTest.ts';
import { timeChartLaneView } from './laneView.ts';
import { formatChartAxisValue, layoutTimeChart, niceChartTicks } from './layout.ts';
import { CHART_MEASURE_COLOURS, type ChartMonthSlot, type TimeChartModel } from './model.ts';

const measure: MeasureText = (text: string) => text.length * 6.5;
const FONT = 'Segoe UI, sans-serif';
const H1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;

function makeBucket(firstHour: number, lastHour: number, v: number, minV = v, maxV = v): ChartBucket {
  return {
    firstHour,
    lastHour,
    firstValue: v,
    lastValue: v,
    lastValueHour: lastHour,
    minHour: firstHour,
    minValue: minV,
    maxHour: lastHour,
    maxValue: maxV,
    firstBreak: false,
    hasBreak: false,
  };
}

function mockCanvas(width: number, height: number): { readonly canvas: OffscreenCanvas; readonly ctx: Canvas2D; readonly ops: string[] } {
  const ops: string[] = [];
  const grad = { addColorStop: () => {} };
  const ctx = {
    font: '',
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    lineJoin: 'miter',
    lineCap: 'butt',
    globalAlpha: 1,
    textAlign: 'left',
    textBaseline: 'alphabetic',
    setTransform: () => { ops.push('setTransform'); },
    clearRect: () => { ops.push('clearRect'); },
    save: () => { ops.push('save'); },
    restore: () => { ops.push('restore'); },
    beginPath: () => { ops.push('beginPath'); },
    closePath: () => {},
    moveTo: () => {},
    lineTo: () => {},
    quadraticCurveTo: () => {},
    roundRect: () => { ops.push('roundRect'); },
    rect: () => {},
    arc: () => { ops.push('arc'); },
    clip: () => {},
    fill: () => { ops.push('fill'); },
    stroke: () => { ops.push('stroke'); },
    fillRect: () => { ops.push('fillRect'); },
    strokeRect: () => { ops.push('strokeRect'); },
    setLineDash: () => {},
    fillText: (text: string) => { ops.push(`text:${text}`); },
    measureText: (text: string) => ({ width: text.length * 6.5 }),
    createLinearGradient: () => grad,
    drawImage: () => { ops.push('drawImage'); },
  } as unknown as Canvas2D;
  const canvas = { width, height, getContext: () => ctx } as unknown as OffscreenCanvas;
  return { canvas, ctx, ops };
}

describe('time-chart BoundedSeries integration (Foundation A)', () => {
  it('consumes already-cut BoundedSeries without reading future buckets', () => {
    const base = Math.floor(H1936 / 100) * 100;
    const line: TiledLine = {
      tag: 'GER',
      head: 'h',
      decimals: 0,
      summary: {
        from: base,
        to: base + 299,
        width: 300,
        buckets: [
          makeBucket(base, base + 99, 10, 5, 20),
          makeBucket(base + 100, base + 199, 999, 10, 999),
          makeBucket(base + 200, base + 299, 5000, 5000, 5000),
        ],
        tiles: { binHours: 100, valueAt: () => null },
      },
      missing: [],
      parts: [],
      marks: null,
    };
    const until = displayedHour(base + 140);
    const bounded = boundTiledSeries(line, until, {
      headPoint: { hour: until, value: 35 },
      headValue: knownValue(35),
    });
    assert.equal(bounded.straddling, 'cut');
    const model: TimeChartModel = {
      kind: 'political-power',
      measureTitle: 'Political power',
      mode: 'lines',
      hour: until,
      extent: [base, base + 299],
      yAxis: { format: 'power' },
      series: [{
        id: 'GER',
        key: 'GER',
        name: 'German Reich',
        color: CHART_MEASURE_COLOURS.politicalPower,
        role: 'primary',
        bounded,
        headValue: 35,
        headHour: until,
      }],
    };
    const layout = layoutTimeChart(model, 600, 220, FONT, measure);
    assert.ok(layout.yDomain[1] < 100);
  });

  it('resolves dated country identity through countryLooksIdentityProvider', () => {
    const provider = countryLooksIdentityProvider({
      GER: [
        { from: H1936, name: 'German Reich', color: '#454545', flag: 'flag://ger' },
        { from: H1936 + 1000, name: 'German Empire', color: '#334455', flag: 'flag://imp' },
      ],
    });
    const early = provider.resolve('country', 'GER', displayedHour(H1936 + 50));
    assert.deepEqual(early.name, { status: 'ready', value: 'German Reich' });
    assert.deepEqual(early.colour, { status: 'ready', value: '#454545' });
    assert.equal(early.nextChangeAt, H1936 + 1000);
    const late = provider.resolve('country', 'GER', displayedHour(H1936 + 1200));
    assert.deepEqual(late.name, { status: 'ready', value: 'German Empire' });
  });
});

describe('time-chart layout & rules C1-C12', () => {
  it('shows start year when recording starts at 1 Jan 12:00 and picks 4-5 y-ticks at default size', () => {
    const hStart = H1936 + 12;
    const hPlay = engineHour({ year: 1941, month: 9, day: 15, hour: 1 })!;
    const model: TimeChartModel = {
      kind: 'political-power',
      measureTitle: 'Political power',
      mode: 'lines',
      hour: hPlay,
      extent: [hStart, hPlay],
      yAxis: { format: 'power' },
      series: [{
        id: 'GER',
        key: 'GER',
        name: 'German Reich',
        color: CHART_MEASURE_COLOURS.politicalPower,
        role: 'primary',
        area: true,
        negRed: true,
        points: [
          { hour: hStart, value: 0 },
          { hour: hStart + 100 * 24, value: -10 },
          { hour: hStart + 500 * 24, value: 220 },
          { hour: hPlay, value: 22 },
        ],
        headValue: 22,
        headHour: hPlay,
        endLabel: { name: null, value: '22' },
      }],
    };
    const wide = layoutTimeChart(model, 958, 314, FONT, measure);
    assert.equal(wide.axisTicks[0]?.label, '1936');
    assert.deepEqual(wide.yTicks, [0, 50, 100, 150, 200]);

    const min = layoutTimeChart(model, 476, 186, FONT, measure);
    assert.equal(min.axisTicks[0]?.label, '1936');
    assert.deepEqual(min.yTicks, [0, 100, 200]);
  });

  it('clamps future points, gives room below zero, and relaxes colliding end labels via shared fitter', () => {
    const h0 = H1936;
    const hPlay = H1936 + 400 * 24;
    const hFuture = H1936 + 600 * 24;
    const model: TimeChartModel = {
      kind: 'political-power',
      measureTitle: 'Political power',
      mode: 'lines',
      hour: hPlay,
      extent: [h0, hFuture],
      yAxis: { format: 'power' },
      series: [
        {
          id: 'a',
          key: 'GER',
          name: 'German Reich',
          color: CHART_MEASURE_COLOURS.politicalPower,
          role: 'primary',
          area: true,
          negRed: true,
          points: [
            { hour: h0, value: 10 },
            { hour: h0 + 100 * 24, value: -25 },
            { hour: hPlay, value: 50 },
            { hour: hFuture, value: 9999 },
          ],
          headValue: 50,
          headHour: hPlay,
          endLabel: { name: 'German Reich', value: '50' },
        },
        {
          id: 'b',
          key: 'ENG',
          name: 'United Kingdom',
          color: '#d04468',
          role: 'primary',
          points: [
            { hour: h0, value: 20 },
            { hour: hPlay, value: 51 },
          ],
          headValue: 51,
          headHour: hPlay,
          endLabel: { name: 'United Kingdom', value: '51' },
        },
      ],
      markers: [
        { id: 'm1', hour: h0 + 50 * 24, title: 'Early', icon: { state: 'ready', url: 'icon://1' } },
        { id: 'm2', hour: h0 + 52 * 24, title: 'Crowded', icon: { state: 'ready', url: 'icon://2' } },
        { id: 'm3', hour: hPlay + 10 * 24, title: 'Future marker', icon: { state: 'ready', url: 'icon://3' } },
      ],
      gaps: [
        { firstHour: h0 + 200 * 24, lastHour: h0 + 210 * 24, reason: 'Game not running' },
        { firstHour: hPlay + 5 * 24, lastHour: hPlay + 20 * 24, reason: 'Future gap' },
      ],
    };
    const layout = layoutTimeChart(model, 800, 260, FONT, measure);
    assert.ok(layout.yDomain[0] < 0);
    assert.ok(layout.yDomain[1] < 200);
    assert.equal(layout.xDomain[1], hPlay);
    assert.equal(layout.markers.length, 2);
    assert.equal(layout.markers[0]!.kind, 'icon');
    assert.equal(layout.markers[1]!.kind, 'tick');
    assert.equal(layout.gaps.length, 1);
    assert.equal(layout.endLabels.length, 2);
    assert.ok(Math.abs(layout.endLabels[1]!.y - layout.endLabels[0]!.y) >= 17);
  });

  it('folds 6+ consecutive unrecorded months and restores true time when disabled', () => {
    const months: ChartMonthSlot[] = [];
    let y = 1940;
    let m = 1;
    for (let i = 0; i < 10; i++) {
      const firstHour = H1936 + (4 * 365 + i * 30) * 24;
      const lastHour = firstHour + 29 * 24;
      const recorded = i >= 7;
      months.push({
        year: y,
        month: m,
        firstHour,
        lastHour,
        status: recorded ? 'recorded' : 'no-rows',
        ...(recorded
          ? {
              segments: [{ id: 'ground', label: 'Ground', color: CHART_MEASURE_COLOURS.armyExperience, value: i === 7 ? 0 : 1000, exactText: i === 7 ? '0' : '1,000' }],
              totalValue: i === 7 ? 0 : 1000,
              totalText: i === 7 ? '0' : '1,000',
            }
          : { segments: [], totalValue: 0, totalText: '0' }),
        partial: i === 9,
        partialToHour: i === 9 ? firstHour + 14 * 24 : undefined,
      });
      m++;
      if (m > 12) { m = 1; y++; }
    }
    const base: TimeChartModel = {
      kind: 'army-losses-by-month',
      measureTitle: 'Army losses',
      mode: 'columns',
      hour: months[9]!.firstHour + 14 * 24,
      extent: [months[0]!.firstHour, months[9]!.lastHour],
      yAxis: { format: 'compact' },
      series: [],
      columns: { foldUnrecorded: true, foldThreshold: 6, months },
    };
    const folded = layoutTimeChart(base, 720, 240, FONT, measure);
    assert.equal(folded.columns.length, 4);
    assert.equal(folded.columns[0]!.kind, 'fold');
    assert.equal(folded.columns[1]!.kind, 'month');
    assert.equal(folded.columns[1]!.month?.totalValue, 0);
    assert.ok(folded.columns[1]!.label?.includes('1940'));

    const foldHit = hitTestTimeChart(base, folded, [folded.columns[0]!.x + 10, folded.top + 20]);
    assert.equal(foldHit, 'fold:0');
    const foldTip = deriveTimeChartTooltip(base, folded, foldHit);
    assert.equal(foldTip?.payload?.title, 'No loss rows recorded');

    const trueTime = layoutTimeChart({ ...base, columns: { ...base.columns!, foldUnrecorded: false } }, 720, 240, FONT, measure);
    assert.equal(trueTime.columns.length, 10);
    assert.equal(trueTime.columns.every(c => c.kind === 'month'), true);
  });

  it('formats dates without hours and compact/signed axis values via shared formatters', () => {
    assert.equal(formatDateOnly(H1936), '1 Jan 1936');
    assert.equal(formatChartAxisValue(250000, 'compact'), '250K');
    assert.equal(formatChartAxisValue(1500000, 'compact'), '1.5M');
    assert.equal(formatChartAxisValue(-0.5, 'signed1'), '\u22120.5');
    assert.equal(formatChartAxisValue(0.5, 'signed1'), '+0.5');
    assert.equal(formatChartAxisValue(0, 'signed1'), '0');
    assert.deepEqual(niceChartTicks(-14, 236, 4), [0, 50, 100, 150, 200]);
  });
});

describe('time-chart laneView integration', () => {
  it('paints a registered chart over BoundedReadView, hit-tests lines, emits payload tooltip and audit rows', () => {
    registerTimeChart({
      kind: 'test-pp',
      family: 'series',
      variant: 'line',
      measureTitle: 'Political power',
      project: readView => ({
        kind: 'test-pp',
        measureTitle: 'Political power',
        mode: 'lines',
        hour: readView.hour,
        extent: [H1936, H1936 + 100 * 24],
        yAxis: { format: 'power' },
        series: [{
          id: 'ger',
          key: 'GER',
          name: 'German Reich',
          color: CHART_MEASURE_COLOURS.politicalPower,
          role: 'primary',
          area: true,
          negRed: true,
          points: [
            { hour: H1936, value: 0 },
            { hour: H1936 + 100 * 24, value: 120 },
          ],
          headValue: 120,
          headHour: H1936 + 100 * 24,
          endLabel: { name: null, value: '120' },
          absentReason: null,
        }],
        auditRows: [{ row: 'GER', key: 'GER', value: '120' }],
      }),
    });

    const created = mockCanvas(600, 240);
    const view = timeChartLaneView(undefined, { createCanvas: () => ({ canvas: created.canvas, ctx: created.ctx }) });
    const state = new Map<string, unknown>();
    let seq = 1;
    const binding: TimeChartBinding = {
      family: 'time-chart',
      kind: 'test-pp',
      options: {},
      sources: {},
      extent: [H1936, H1936 + 100 * 24],
      width: 600,
      height: 240,
      audit: true,
      auditId: 7,
    };

    const painted = view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: binding,
      hour: H1936 + 100 * 24,
      speed: 0,
      binding: { generation: 1, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 0,
    }, state, () => `c${seq++}`);

    assert.equal(painted.surfaces.length, 1);
    assert.ok(painted.fields.audit);
    assert.ok(created.ops.some(op => op.startsWith('text:120')));
    const hit = painted.hit?.([300, 100]);
    assert.ok(hit && (hit.startsWith('plot:') || hit.startsWith('line:')));

    const hovered = view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: binding,
      hour: H1936 + 100 * 24,
      speed: 0,
      binding: { generation: 1, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
      inspect: hit ?? null,
      now: 1,
    }, state, () => `c${seq++}`);
    assert.ok(hovered.fields.tooltip);

    view.unmount?.(1, state);
    assert.equal(state.has('time-chart'), false);
  });
});
