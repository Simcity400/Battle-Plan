import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { ChartBucket } from '../../charts/chartSummary.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import type { Canvas2D, MeasureText } from '../../charts/textWidth.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import { readyArt } from '../art/types.ts';
import { boundTiledSeries, createBoundedReadView, displayedHour } from '../boundedView.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import { knownValue, notRecorded } from '../value.ts';
import { formatDateOnly } from '../valueFormat.ts';
import { commandPowerProjection } from '../../widgets/command-power/commandPowerProjection.ts';
import { stabilityProjection } from '../../widgets/stability/stabilityProjection.ts';
import { experienceProjection } from '../../widgets/experience/experienceProjection.ts';
import { fuelProjection } from '../../widgets/fuel/fuelProjection.ts';
import { type TimeChartBinding } from './declaration.ts';
import { registerFamilyProjection } from '../registry.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from './hitTest.ts';
import { createTimeChartGestures, parseTimeChartHit, timeChartHostAdapter } from './hostAdapter.ts';
import { timeChartLane } from './laneView.ts';
import { assertFreshPaint, assertQuietFrames, LANE_THEME, LaneTransport } from '../laneRuntime.fixture.ts';
import { SceneTiles } from '../sceneTiles.ts';
import { formatChartAxisValue, hoverTimeChartLayout, layoutTimeChart } from './layout.ts';
import type { ChartMonthSlot, TimeChartModel } from './model.ts';
import { buildLineSubpaths } from './painter.ts';
import { parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { buildBoundedStack, headReading, headValueOf } from './seriesHelpers.ts';
import { sampleSeriesValueAt } from './hitTest.ts';
import { politicalPowerProjection } from '../../widgets/political-power/politicalPowerProjection.ts';
import { navyLinesProjection } from '../../widgets/navy-size/navyLinesProjection.ts';
import { armyLinesProjection } from '../../widgets/army-size/armyLinesProjection.ts';
import { supplyHealthProjection } from '../../widgets/supply-health/supplyHealthProjection.ts';
import { rulingPartyProjection } from '../../widgets/ruling-party/rulingPartyProjection.ts';
import { createLaneView } from '../laneRuntime.ts';
import { drawTimeChart } from './painter.ts';

const measure: MeasureText = (text: string) => text.length * 6.5;
const FONT = 'Segoe UI, sans-serif';
const H1936 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;

function makeBucket(firstHour: number, lastHour: number, v: number, minV = v, maxV = v, lastV = v, lastValueHour = lastHour): ChartBucket {
  return {
    firstHour,
    lastHour,
    firstValue: v,
    lastValue: lastV,
    lastValueHour,
    minHour: firstHour,
    minValue: minV,
    maxHour: lastValueHour,
    maxValue: maxV,
    firstBreak: false,
    hasBreak: false,
  };
}

function makeTiledLine(tag: string, buckets: readonly ChartBucket[]): TiledLine {
  const from = buckets[0]?.firstHour ?? 0;
  const to = buckets[buckets.length - 1]?.lastHour ?? from;
  return {
    tag,
    head: 'h',
    decimals: 0,
    summary: { from, to, width: buckets.length, buckets: [...buckets], tiles: { binHours: 96, valueAt: () => null } },
    missing: [],
    parts: [],
    marks: null,
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

function laneOn(created: { readonly ctx: Canvas2D }) {
  (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas = class {
    width: number;
    height: number;
    constructor(width: number, height: number) { this.width = width; this.height = height; }
    getContext() { return created.ctx; }
  };
  return createLaneView(timeChartLane, () => {});
}

describe('time-chart BoundedSeries & Foundation C integration', () => {
  it('consumes already-cut BoundedSeries without reading future buckets and joins ramps without staircase treads', () => {
    const base = Math.floor(H1936 / 96) * 96;
    const line: TiledLine = {
      tag: 'GER',
      head: 'h',
      decimals: 0,
      summary: {
        from: base,
        to: base + 287,
        width: 300,
        buckets: [
          makeBucket(base, base + 95, 10, 10, 20, 20, base + 72),
          makeBucket(base + 96, base + 191, 999, 10, 999, 999, base + 168),
          makeBucket(base + 192, base + 287, 5000, 5000, 5000, 5000, base + 264),
        ],
        tiles: { binHours: 96, valueAt: () => null },
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
      extent: [base, base + 287],
      yAxis: { format: 'power' },
      series: [{
        id: 'GER',
        key: 'GER',
        name: 'German Reich',
        color: CHART_MEASURE_COLOURS.politicalPower,
        role: 'primary',
        step: false,
        bounded,
        headValue: 35,
        headHour: until,
      }],
    };
    const layout = layoutTimeChart(model, 600, 220, FONT, measure);
    assert.ok(layout.yDomain[1] < 100);

    const joined = buildLineSubpaths(model.series[0]!, layout, until);
    const pts = joined.subpaths[0]!;
    for (let i = 1; i < pts.length; i++) {
      assert.notEqual(pts[i]![1], pts[i - 1]![1], 'joined monotonic ramp should have no flat staircase treads');
    }

    const stepped = buildLineSubpaths({ ...model.series[0]!, step: true }, layout, until);
    const sPts = stepped.subpaths[0]!;
    assert.ok(sPts.some((p, i) => i > 0 && p[1] === sPts[i - 1]![1]), 'step mode keeps horizontal holds');
  });

  it('resolves dated country identity through Foundation C IdentityIndex snapshot provider', () => {
    const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [H1936, H1936 + 5000]);
    index.setFacts({
      branchId: 'b',
      known: [[H1936, H1936 + 5000]],
      names: ['German Reich'],
      reasons: [],
      flags: [{ keys: ['GER_fascism', null, null], reasons: [null, null, null] }],
      countries: [{ tag: 'GER', runs: [[H1936, 0, 0x454545, -1, 0]] }],
      warnings: [],
    });
    const provider = createSnapshotIdentityProvider(index);
    const res = provider.resolve('country', 'GER', displayedHour(H1936));
    assert.deepEqual(res.name, { status: 'ready', value: 'German Reich' });
    assert.deepEqual(res.colour, { status: 'ready', value: '#454545' });
    assert.equal(res.flagChoice?.status, 'ready');
    if (res.flagChoice?.status === 'ready') assert.ok(res.flagChoice.value.includes('GER_fascism'));
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
        { id: 'm1', hour: h0 + 50 * 24, title: 'Early', icon: { art: readyArt('icon://1') } },
        { id: 'm2', hour: h0 + 52 * 24, title: 'Crowded', icon: { art: readyArt('icon://2') } },
        { id: 'm3', hour: hPlay + 10 * 24, title: 'Future marker', icon: { art: readyArt('icon://3') } },
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
    assert.equal(foldHit, `fold:0@${months[0]!.firstHour}`);
    assert.equal(parseTimeChartHit(foldHit)?.hour, months[0]!.firstHour);
    const foldTip = deriveTimeChartTooltip(base, folded, foldHit);
    assert.equal(foldTip?.payload?.title, 'No loss rows recorded');

    const monthHit = hitTestTimeChart(base, folded, [folded.columns[1]!.x + 10, folded.top + 20]);
    assert.equal(monthHit, `month:1@${months[7]!.firstHour}`);
    assert.equal(parseTimeChartHit(monthHit)?.hour, months[7]!.firstHour);

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
    const bopTicks = [-1, -0.5, 0, 0.5, 1];
    assert.deepEqual(
      bopTicks.map(v => formatChartAxisValue(v, 'signed1', bopTicks)),
      ['\u22121.0', '\u22120.5', '0', '+0.5', '+1.0'],
      'K4: all non-zero signed axis labels carry the same decimal count',
    );
  });
});

describe('time-chart laneView integration', () => {
  it('paints a registered chart over BoundedReadView with LaneArtCache demands, hit-tests lines, emits tooltip and audit rows', () => {
    registerFamilyProjection({
      kind: 'test-pp',
      family: 'time-chart',
      variant: 'line',
      joinMode: 'join',
      measureTitle: 'Political power',
      project: readView => {
        const icon = readView.art({ kind: 'sprite', key: 'GFX_pol_power', frame: 0, recorded: 'recorded-sprite', text: true });
        return {
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
            endLabel: { icon: { art: icon, label: 'Political power' }, name: null, value: '120' },
            absentReason: null,
          }],
          auditRows: [{ row: 'GER', key: 'GER', value: '120' }],
        };
      },
    });

    const created = mockCanvas(600, 240);
    const view = laneOn(created);
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
      scrollTop: 0,
      theme: LANE_THEME,
      audit: true,
      auditId: 7,
    };

    const transport = new LaneTransport();
    const painted = view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: transport.carry(binding),
      hour: H1936 + 100 * 24,
      seek: 0, speed: 0,
      binding: { generation: 1, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 0,
    }, state, () => `c${seq++}`);

    assert.equal(painted.surfaces.length, 1);
    assert.ok(painted.fields.audit);
    assert.deepEqual(painted.fields.artDemands?.value, [{ kind: 'sprite', key: 'GFX_pol_power', frame: 0, recorded: 'recorded-sprite', text: true }],
      'the full sprite handle goes to the art request');
    assert.ok(created.ops.some(op => op.startsWith('text:120')));
    assert.equal((painted.fields.audit?.value as { id?: number }).id, 7, 'the audit field names its id where the host reads it');
    const stops = painted.fields.nav?.value as readonly (readonly [string, string, number])[];
    assert.ok(stops.length >= 2, 'the chart publishes navigation stops');
    assert.equal(stops[stops.length - 1]![0], `plot:${H1936 + 100 * 24}`, 'the last stop is the playhead');
    assert.ok(stops.every((stop, index) => stop[2] === index));
    assert.deepEqual(timeChartHostAdapter.deriveAuditRows?.({ auditRows: [{ row: 'GER', key: 'GER', value: '120' }] } as unknown as TimeChartModel, 0, { viewport: {} as never, settings: undefined, expanded: {}, theme: undefined, fields: {} }),
      [{ row: 'GER', key: 'GER', value: '120' }]);
    const focused = view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: transport.carry({ ...binding, focusedKey: `plot:${H1936 + 100 * 24}` }),
      hour: H1936 + 100 * 24,
      seek: 0, speed: 0,
      binding: { generation: 1, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 0,
    }, state, () => `c${seq++}`);
    assert.equal((focused.fields.focusTooltip?.value as { key: string }).key, `focus:plot:${H1936 + 100 * 24}`, 'a focused stop has its tooltip');
    const hit = painted.hit?.([300, 100]);
    assert.ok(hit && (hit.startsWith('plot:') || hit.startsWith('line:')));

    const hovered = view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: transport.carry(binding),
      hour: H1936 + 100 * 24,
      seek: 0, speed: 0,
      binding: { generation: 1, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
      inspect: hit ?? null,
      now: 1,
    }, state, () => `c${seq++}`);
    assert.ok(hovered.fields.tooltip);
    const fullTransform = (hovered.fields.meta?.value as { transform: { xFrom: number; xTo: number } }).transform;
    assert.deepEqual([fullTransform.xFrom, fullTransform.xTo], [H1936, H1936 + 100 * 24]);

    const zoomWindow: readonly [number, number] = [H1936 + 20 * 24, H1936 + 50 * 24];
    const zoomed = view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: transport.carry({ ...binding, zoom: { x: zoomWindow } }),
      hour: H1936 + 100 * 24,
      seek: 0, speed: 0,
      binding: { generation: 2, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 2,
    }, state, () => `c${seq++}`);
    const zoomedTransform = (zoomed.fields.meta?.value as { transform: { xFrom: number; xTo: number } }).transform;
    assert.deepEqual([zoomedTransform.xFrom, zoomedTransform.xTo], zoomWindow, 'the bound zoom window is the drawn x domain');

    view.unmount?.(1, state);
    assert.equal(state.size, 0, 'unmount leaves nothing held');
  });

  it('projects command-power with joined balance, stepped dashed ceiling, muted Maximum label, and spend ticks', () => {
    const until = H1936 + 200;
    const mainLine = makeTiledLine('GER', [makeBucket(H1936, H1936 + 96, 10, 10, 23, 23, H1936 + 72), makeBucket(H1936 + 96, until, 23, 23, 23, 23, until)]);
    const capLine = makeTiledLine('GER', [makeBucket(H1936, until, 150, 150, 150, 150, until)]);
    const readView = createBoundedReadView({
      hour: until,
      series: new Map([
        ['main', new Map([['GER', boundTiledSeries(mainLine, displayedHour(until), { headValue: knownValue(23) })]])],
        ['ceiling', new Map([['GER', boundTiledSeries(capLine, displayedHour(until), { headValue: knownValue(150) })]])],
      ]),
    });
    const model = commandPowerProjection.project(readView, {
      tags: ['GER'],
      extent: [H1936, until],
      gaps: [],
      surface: '#111111',
      showCeiling: true,
      showSpendTicks: true,
    });
    assert.equal(model.series.length, 2);
    assert.equal(model.series[0]!.step, false);
    assert.equal(model.series[0]!.area, true);
    assert.equal(model.series[0]!.endLabel?.name, 'Command power');
    assert.equal(model.series[0]!.endLabel?.value, '23');
    assert.equal(model.series[1]!.role, 'ceiling');
    assert.equal(model.series[1]!.step, true);
    assert.equal(model.series[1]!.endLabel?.name, 'Maximum');
    assert.equal(model.series[1]!.endLabel?.valueMuted, true);

    const minLayout = layoutTimeChart(model, 476, 186, FONT, measure);
    const cpLabel = minLayout.endLabels.find(l => l.seriesId === 'GER');
    const maxLabel = minLayout.endLabels.find(l => l.seriesId === 'GER:ceiling');
    assert.equal(cpLabel?.name, null, 'at min size icon-bearing line hides its name');
    assert.equal(maxLabel?.name, 'Maximum', 'at min size ceiling keeps Maximum word');
  });

  it('projects stability with valueScale 100 step lines, 0-100% axis, and future-clamped markers', () => {
    const until = H1936 + 240;
    const stabLine = makeTiledLine('GER', [makeBucket(H1936, until, 0.8, 0.75, 1.0, 1.0, until)]);
    const warLine = makeTiledLine('GER', [makeBucket(H1936, until, 0.35, 0.35, 0.9, 0.9, until)]);
    const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [H1936, until + 500]);
    index.setFacts({
      branchId: 'b',
      known: [[H1936, until + 500]],
      names: ['Poland'],
      reasons: [],
      flags: [],
      countries: [{ tag: 'POL', runs: [[H1936, 0, 0xaa4455, -1, -1]] }],
      warnings: [],
    });
    const readView = createBoundedReadView({
      hour: until,
      series: new Map([
        ['stability', new Map([['GER', boundTiledSeries(stabLine, displayedHour(until), { headValue: knownValue(1.0) })]])],
        ['warSupport', new Map([['GER', boundTiledSeries(warLine, displayedHour(until), { headValue: knownValue(0.9) })]])],
      ]),
      panels: new Map([['markers', { hour: until, nextChange: until + 48, warnings: [], markers: [
        { id: 'i1', hour: H1936 + 105, kind: 'idea', title: 'Partial mobilisation adopted', verb: null, other: null, faction: null, sprite: null, alert: false },
        { id: 'w1', hour: H1936 + 113, kind: 'war', title: 'War with POL began', verb: 'began', other: 'POL', faction: null, sprite: null, alert: true },
      ] }]]),
      identity: createSnapshotIdentityProvider(index),
    });
    const model = stabilityProjection.project(readView, {
      tags: ['GER'],
      extent: [H1936, until],
      gaps: [],
      surface: '#111111',
      showStability: true,
      showWarSupport: true,
      zoom: false,
    });
    assert.equal(model.series.length, 2);
    assert.equal(model.series[0]!.step, true);
    assert.equal(model.series[0]!.valueScale, 100);
    assert.equal(model.series[0]!.endLabel?.value, '100%');
    assert.equal(model.series[1]!.endLabel?.value, '90%');
    assert.deepEqual(model.markers?.map(marker => [marker.hour, marker.title]), [
      [H1936 + 105, 'Partial mobilisation adopted'],
      [H1936 + 113, 'War with Poland began'],
    ], 'each event keeps its own hour: a later alert on the same day neither moves nor hides the morning one');

    const fullLayout = layoutTimeChart(model, 960, 276, FONT, measure);
    assert.deepEqual(fullLayout.yTicks, [0, 25, 50, 75, 100]);
    const [first, second] = fullLayout.endLabels;
    assert.equal(first!.y, first!.anchorY, 'a label at the top of the plot centres on its line');
    assert.ok(second!.y - first!.y >= 17, 'the next label keeps at least 17 px');
    const minLayout = layoutTimeChart(model, 476, 186, FONT, measure);
    assert.deepEqual(minLayout.yTicks, [0, 50, 100]);

    const formula = (valueRaw: string) => ({ valueRaw, terms: [], clampRaw: null });
    const hoverAt = H1936 + 120;
    const breakdown = {
      tag: 'GER',
      hour: hoverAt,
      breakdown: { hour: hoverAt, readHour: hoverAt, language: 'english', stability: formula('80000'), warSupport: formula('35000'),
        maxCommandPower: formula('15000000'), linesAvailable: false, modifiers: [] },
      missing: null,
      warnings: [],
    };
    const projectHovered = (hovers: Map<string, { hour: number; data: unknown }>) => stabilityProjection.project(createBoundedReadView({
      hour: until,
      series: new Map([['stability', new Map([['GER', boundTiledSeries(stabLine, displayedHour(until), { headValue: knownValue(1.0) })]])]]),
      hovers,
      identity: createSnapshotIdentityProvider(index),
    }), { tags: ['GER'], extent: [H1936, until], gaps: [], surface: '#111111', showStability: true, showWarSupport: false, zoom: false });
    const hovered = projectHovered(new Map([['hover', { hour: hoverAt, data: breakdown }]]));
    assert.equal(hovered.hoverDetail?.hour, hoverAt);
    const tip = deriveTimeChartTooltip(hovered, layoutTimeChart(hovered, 960, 276, FONT, measure), `plot:${hoverAt}`);
    const labels = tip?.payload?.items?.map(item => `${item.label}=${item.value ?? ''}`) ?? [];
    assert.ok(labels.includes('Total=80.0%'), `breakdown total in the hover card: ${labels.join(' | ')}`);
    assert.equal(deriveTimeChartTooltip(hovered, layoutTimeChart(hovered, 960, 276, FONT, measure), `plot:${hoverAt + 1}`)
      ?.payload?.items?.some(item => item.label === 'Total') ?? false, false, 'another hour shows no stale breakdown');

    const missing = projectHovered(new Map([['hover', { hour: hoverAt, data: { ...breakdown, breakdown: null, missing: 'Stability inputs were not read' } }]]));
    assert.equal(missing.hoverDetail?.items[0]?.missingReason, 'Stability inputs were not read');

    const afterSeekBack = projectHovered(new Map([['hover', { hour: until + 24, data: breakdown }]]));
    assert.equal(afterSeekBack.hoverDetail, null, 'a hover read after the displayed hour is never shown');
  });
});

describe('time-chart picture authority', () => {
  const plotRect = { left: 0, top: 0, width: 100, height: 100 };
  const gestureContext = (shown: readonly [number, number], axis: readonly [number, number], zoomWindow: readonly [number, number] | null) => {
    const asked: (readonly [number, number] | null)[] = [];
    const setZoomWindow = (next: readonly [number, number] | null) => { asked.push(next); };
    return {
      asked,
      ctx: {
        fields: { meta: { plotRect, transform: { xFrom: shown[0], xTo: shown[1], axisFrom: axis[0], axisTo: axis[1] } } },
        viewport: { zoomWindow } as never,
        extent: [0, 1000] as const,
        setZoomWindow,
      },
    };
  };

  it('zooms within the drawn axis, not the branch extent', () => {
    const gestures = createTimeChartGestures();
    const { ctx, asked } = gestureContext([0, 100], [0, 100], null);
    assert.equal(gestures.onWheel({ deltaY: -1 } as WheelEvent, [50, 50], ctx), true);
    assert.deepEqual(asked, [[10, 90]]);
  });

  it('keeps drag state per mounted chart', () => {
    const gestures = createTimeChartGestures();
    const a = gestureContext([10, 40], [0, 100], [10, 40]);
    const b = gestureContext([0, 100], [0, 100], null);
    gestures.onDragStart([50, 50], a.ctx);
    gestures.onDragStart([50, 50], b.ctx);
    gestures.onDragMove([60, 50], [-10, 0], a.ctx);
    assert.deepEqual(a.asked, [[13, 43]], 'a second chart starting a drag does not clear the first');
    gestures.onDragMove([60, 50], [-10, 0], b.ctx);
    assert.deepEqual(b.asked, [], 'an unzoomed chart does not pan');
  });

  const pictureModel = (hour: number, extra: Partial<TimeChartModel> = {}): TimeChartModel => ({
    kind: 'command-power',
    measureTitle: 'Command power',
    mode: 'lines',
    hour,
    extent: [H1936, H1936 + 1000],
    yAxis: { format: 'number' },
    series: [{ id: 'cp', key: 'cp', name: 'Command power', color: '#c76a90', role: 'primary', points: [{ hour: H1936, value: 5 }, { hour, value: 5 }] }],
    ...extra,
  });

  it('gives a marker hit its recorded hour', () => {
    const hour = H1936 + 400;
    const model = pictureModel(hour, { markers: [{ id: 'spend', hour: H1936 + 200, title: 'Command power spent' }] });
    const layout = layoutTimeChart(model, 600, 260, FONT, measure);
    const placed = layout.markers[0]!;
    const key = hitTestTimeChart(model, layout, [placed.x, placed.y]);
    assert.equal(key, `marker:0@${H1936 + 200}`);
    assert.equal(parseTimeChartHit(key)?.hour, H1936 + 200);
  });

  it('names a gap by its own identity and never by its future end', () => {
    const hour = H1936 + 400;
    const model = pictureModel(hour, {
      view: { x: [H1936 + 100, hour] },
      gaps: [
        { firstHour: H1936 + 10, lastHour: H1936 + 20, reason: 'Earlier gap' },
        { firstHour: H1936 + 300, lastHour: H1936 + 900, reason: 'Ongoing gap' },
      ],
    });
    const layout = layoutTimeChart(model, 600, 260, FONT, measure);
    assert.equal(layout.gaps.length, 1, 'the earlier gap is outside the window');
    const placed = layout.gaps[0]!;
    const key = hitTestTimeChart(model, layout, [(placed.x0 + placed.x1) / 2, 100]);
    assert.equal(key, 'gap:1');
    const tip = deriveTimeChartTooltip(model, layout, key);
    assert.equal(tip?.payload?.note, 'Ongoing gap');
    assert.equal(tip?.payload?.subtitle, `From ${formatDateOnly(H1936 + 300)}`, 'no end after the displayed hour');
  });
  it('a deselected line leaves the tile cursors, so cycling countries keeps no history behind', () => {
    const hour = H1936 + 95;
    const line = (tag: string) => ({ source: 'main', tag, head: 'h', decimals: 0, binHours: 96, from: H1936, to: hour, width: 1,
      buckets: [makeBucket(H1936, hour, 1)], missing: [], parts: [], marks: null });
    const tiles = new SceneTiles();
    const scene = { widget: 1, subs: [], pictures: [], model: null };
    const held = () => (tiles as unknown as { readonly cursors: ReadonlyMap<string, unknown> }).cursors.size;
    for (let at = 0; at < 50; at++) tiles.series(scene, { sources: {} }, { tiledBySource: { main: { [`C${at}`]: line(`C${at}`) } } }, hour);
    assert.equal(held(), 1, 'only the line still selected keeps a cursor');
    tiles.series(scene, { sources: {} }, {}, hour);
    assert.equal(held(), 0);
  });
  it('a hover reuses the laid-out chart: only end-label fading and the axis labels under the hover tab change', () => {
    const base = Math.floor(H1936 / 96) * 96;
    const hour = base + 900;
    const line = makeTiledLine('GER', [makeBucket(base, base + 899, 10)]);
    const series = (id: string, value: number) => ({ id, key: id, name: id, color: '#e3a83b', role: 'primary' as const,
      bounded: boundTiledSeries(line, displayedHour(hour), { headValue: knownValue(value) }), headValue: value, headHour: hour, endLabel: { value: String(value) } });
    const model = pictureModel(hour, { series: [series('a', 10), series('b', 20)] });
    const laid = layoutTimeChart(model, 600, 260, FONT, measure);
    let measured = 0;
    const counting = (text: string, font: string) => { measured++; return measure(text, font); };
    const tick = laid.axisTicks.find(item => item.label !== null && item.hour < hour - 200)!;
    const hovered = hoverTimeChartLayout(laid, model.hour, FONT, counting, tick.hour, 'a');
    assert.equal(measured, 1, 'only the hover tab is measured; no series, bucket or label is laid out again');
    for (const field of ['yTicks', 'markers', 'gaps', 'columns', 'axisCandidates', 'toX'] as const) assert.equal(hovered[field], laid[field], `${field} is the base layout's`);
    assert.equal(hovered.axisTicks.find(item => item.hour === tick.hour)!.label, null, 'the axis label under the hover tab is hidden');
    assert.deepEqual(Object.fromEntries(hovered.endLabels.map(label => [label.seriesId, label.faded])), { a: false, b: true }, 'the other series fades');
    assert.equal(hoverTimeChartLayout(laid, model.hour, FONT, counting, null, null), laid, 'no hover is the base layout itself');
  });
  it('scales and draws a historical zoom from its own window, never from the current head', () => {
    const base = Math.floor(H1936 / 96) * 96;
    const hour = base + 40 * 24;
    const line = makeTiledLine('GER', [makeBucket(base, base + 95, 10)]);
    const bounded = boundTiledSeries(line, displayedHour(hour), { headPoint: { hour, value: 999 }, headValue: knownValue(999) });
    const model = pictureModel(hour, {
      view: { x: [base, base + 8] },
      series: [{ id: 'pp', key: 'pp', name: 'Political power', color: '#e3a83b', role: 'primary', bounded, headValue: 999, headHour: hour,
        endLabel: { value: '999' } }],
    });
    const layout = layoutTimeChart(model, 600, 260, FONT, measure);
    assert.ok(layout.yDomain[1] < 100, `y axis from the window: ${layout.yDomain[1]}`);
    assert.equal(layout.endLabels.length, 0, 'no end label when the playhead is outside the window');
    const { subpaths } = buildLineSubpaths(model.series[0]!, layout, hour);
    assert.ok(subpaths.flat().every(([, y]) => y >= layout.top - 1), 'no point drawn at the head value');
  });
  it('never turns a missing head or an unwitnessed hour into an exact reading', () => {
    const base = Math.floor(H1936 / 96) * 96;
    const hour = base + 40;
    const line = makeTiledLine('GER', [makeBucket(base, base + 30, 10, 10, 30, 10, base + 30)]);
    const missingHead = boundTiledSeries(line, displayedHour(hour), { headValue: notRecorded('Countries not read at this hour') });
    const headOf = (bounded: ReturnType<typeof boundTiledSeries>) =>
      headReading(createBoundedReadView({ hour, series: new Map([['main', new Map([['GER', bounded]])]]) }), 'main', { tag: 'GER' }, 0);
    assert.deepEqual(headOf(missingHead), { missing: 'Countries not read at this hour' }, 'an explicitly missing head stays missing');
    const model = politicalPowerProjection.project(createBoundedReadView({
      hour,
      series: new Map([['main', new Map([['GER', missingHead]])]]),
    }), { tags: ['GER'], extent: [base, base + 95], gaps: [], surface: '#111111' });
    assert.equal(model.series[0]!.endLabel, null);
    assert.deepEqual(model.auditRows?.map(row => `${row.row}=${row.value}`), ['GER=Countries not read at this hour'], 'the audit states the missing head');

    const gapAfter = boundTiledSeries({ ...line, missing: [{ firstHour: base + 31, lastHour: hour, reason: 'Recording gap' }] }, displayedHour(hour), {});
    assert.equal(headOf(gapAfter), null, 'a gap after the last reading leaves the head unknown, never the last bucket');

    const withHead = { id: 'pp', key: 'pp', name: 'PP', color: '#e3a83b', role: 'primary' as const,
      bounded: boundTiledSeries(line, displayedHour(base + 30), { headPoint: { hour: base + 30, value: 35 }, headValue: knownValue(35) }), headValue: 35, headHour: base + 30 };
    const inside = sampleSeriesValueAt(withHead, base + 25, base + 30);
    assert.notEqual(inside?.value, 35, 'an hour inside the bin is not the head');
  });

  it('shows the declared exact hover reading, or its missing reason', () => {
    const base = Math.floor(H1936 / 96) * 96;
    const hour = base + 90;
    const hoverAt = base + 50;
    const line = { ...makeTiledLine('GER', [makeBucket(base, base + 95, 10, 10, 30, 20, base + 89)]), decimals: 5 };
    const project = (state: unknown) => {
      const model = politicalPowerProjection.project(createBoundedReadView({
        hour,
        series: new Map([['main', new Map([['GER', boundTiledSeries(line, displayedHour(hour), { headValue: knownValue(20) })]])]]),
        hovers: new Map([['hover', { hour: hoverAt, data: [state] }]]),
      }), { tags: ['GER'], extent: [base, base + 95], gaps: [], surface: '#111111' });
      return deriveTimeChartTooltip(model, layoutTimeChart(model, 600, 260, FONT, measure), `plot:${hoverAt}`)?.payload?.items?.[0];
    };
    assert.equal(project({ hour: hoverAt, run: { valueRaw: '1234500000' }, missing: null })?.value, '12,345');
    assert.equal(project({ hour: hoverAt, run: null, missing: 'Countries not read at this hour' })?.missingReason, 'Countries not read at this hour');
  });
  it('shows a group member only from the hour it joins, and keeps colours by recorded order', () => {
    const base = Math.floor(H1936 / 96) * 96;
    const hour = base + 50;
    const line = (tag: string, value: number) => boundTiledSeries(makeTiledLine(tag, [makeBucket(base, base + 50, value)]), displayedHour(hour), { headValue: knownValue(value) });
    const project = (ger: number, ita: number, membership: Readonly<Record<string, readonly (readonly [number, number])[]>> | null) =>
      politicalPowerProjection.project(createBoundedReadView({
        hour,
        series: new Map([['main', new Map([['GER', line('GER', ger)], ['ITA', line('ITA', ita)]])]]),
      }), { tags: ['GER', 'ITA'], membership, extent: [base, base + 95], gaps: [], surface: '#111111' });

    const joinsLater = project(50, 100, { GER: [[base, base + 95]], ITA: [[hour + 10, base + 95]] });
    const ita = joinsLater.series.find(s => s.id === 'ITA')!;
    assert.equal(ita.headValue, null);
    assert.equal(ita.endLabel, null);
    assert.deepEqual(joinsLater.auditRows?.map(row => row.key), ['GER']);

    const gerAhead = project(80, 20, null).series.find(s => s.id === 'GER')!.color;
    const itaAhead = project(20, 80, null).series.find(s => s.id === 'GER')!.color;
    assert.equal(itaAhead, gerAhead, 'a country keeps its colour when another overtakes it');
  });
  it('repaints a same-hour correction of history', () => {
    registerFamilyProjection({
      kind: 'test-correction',
      family: 'time-chart',
      variant: 'line',
      joinMode: 'join',
      measureTitle: 'Correction',
      project: readView => ({
        kind: 'test-correction',
        measureTitle: 'Correction',
        mode: 'lines',
        hour: readView.hour,
        extent: [H1936, H1936 + 95],
        yAxis: { format: 'number' },
        series: [{ id: 'ger', key: 'GER', name: 'German Reich', color: '#e3a83b', role: 'primary', bounded: readView.series('main')?.get('GER') ?? null }],
      }),
    });
    const created = mockCanvas(600, 240);
    const view = laneOn(created);
    const state = new Map<string, unknown>();
    let seq = 1;
    const hour = H1936 + 95;
    const transport = new LaneTransport();
    const tiled = (first: number, last: number) => ({
      source: 'main', tag: 'GER', head: 'h', decimals: 0, binHours: 96, from: H1936, to: H1936 + 95, width: 1,
      buckets: [makeBucket(H1936, H1936 + 95, first, 10, 30, last, H1936 + 95)], missing: [], parts: [], marks: null,
    });
    const paint = (first: number, last: number) => view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: transport.carry({
        family: 'time-chart', kind: 'test-correction', options: {}, sources: {}, extent: [H1936, H1936 + 95], width: 600, height: 240, scrollTop: 0, theme: LANE_THEME,
        tiledBySource: { main: { GER: tiled(first, last) } },
      } satisfies TimeChartBinding),
      hour,
      seek: 0, speed: 0,
      binding: { generation: 1, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 0,
    }, state, () => `c${seq++}`);
    paint(10, 30);
    const before = created.ops.filter(op => op === 'clearRect').length;
    paint(30, 10);
    assert.equal(created.ops.filter(op => op === 'clearRect').length, before + 1, 'the corrected history is drawn');

    const bound = (first: number, last: number) => ({
      family: 'time-chart', kind: 'test-correction', options: {}, sources: {}, extent: [H1936, H1936 + 95], width: 600, height: 240, scrollTop: 0, theme: LANE_THEME,
      tiledBySource: { main: { GER: tiled(first, last) } },
    } satisfies TimeChartBinding);
    const surface = { width: 600, height: 240, dpr: 1 };
    assert.equal(assertQuietFrames(() => createLaneView(timeChartLane, () => {}), { label: 'chart', binding: bound(10, 30), hour, speed: 24 }, 24, surface), hour + 1, 'a playing chart changes every hour');
    const held = bound(10, 30);
    assertFreshPaint(() => createLaneView(timeChartLane, () => {}), [
      { label: 'held', binding: held, hour, check: true },
      { label: 'earlier', binding: held, hour: hour - 20, speed: 24, check: true },
      { label: 'back to the head', binding: held, hour, check: true },
      { label: 'corrected history', binding: bound(30, 10), hour, check: true },
    ], surface);
  });
  it('reads head rows by an index, not a scan per tag, and a missing head cell travels as missing with its reason', () => {
    registerFamilyProjection({
      kind: 'test-head-index',
      family: 'time-chart',
      variant: 'line',
      joinMode: 'join',
      measureTitle: 'Index',
      project: readView => {
        const bounded = readView.series('main')?.get('ITA') ?? null;
        const head = bounded?.headValue;
        return {
          kind: 'test-head-index', measureTitle: 'Index', mode: 'lines', hour: readView.hour, extent: [H1936, H1936 + 95], yAxis: { format: 'number' },
          series: [], auditRows: [{ row: 'ITA', key: 'ITA', value: head?.kind === 'known' ? String(head.value) : head?.kind === 'notRecorded' ? head.reason : `head ${headValueOf(headReading(readView, 'main', { tag: 'ITA' }, 0))}` }],
        };
      },
    });
    const tags = ['GER', 'ITA'];
    const answer = parseFollowed(message(10, [{ id: 1, answer: { keys: [1n, 2n], columns: 1, values: [5n, 7n] } }])).answers[0]!.answer;
    const created = mockCanvas(600, 240);
    const view = laneOn(created);
    const hour = H1936 + 95;
    const transport = new LaneTransport();
    const line = (tag: string) => ({ source: 'main', tag, head: 'h', decimals: 0, binHours: 96, from: H1936, to: hour, width: 1,
      buckets: [makeBucket(H1936, hour, 1)], missing: [], parts: [], marks: null });
    const lines = { main: { GER: line('GER'), ITA: line('ITA') } };
    const heads = { main: { answerName: 'main', tags } };
    const paint = (followed: typeof answer) => {
      const model = transport.carry({
        family: 'time-chart', kind: 'test-head-index', options: {}, sources: {}, extent: [H1936, hour], width: 600, height: 240, scrollTop: 0, theme: LANE_THEME, audit: true, auditId: 1,
        tiledBySource: lines,
        headSourceBySeries: heads,
        staticAnswers: { main: followed },
      } satisfies TimeChartBinding);
      Object.defineProperty(model.headSourceBySeries!.main!.tags, 'indexOf', { configurable: true, value: () => { throw new Error('scanned the tag list'); } });
      return (view.paint({
        widget: 1, subs: [], pictures: [], model, hour, seek: 0, speed: 0,
        binding: { generation: 1, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
        inspect: null, now: 0,
      }, new Map(), () => 'c').fields.audit?.value as { rows: readonly { value?: string }[] }).rows.map(row => row.value);
    };
    assert.deepEqual(paint(answer), ['7']);
    const unread = parseFollowed(message(10, [{ id: 1, answer: { keys: [1n, 2n], columns: 1, values: [5n, 0n], states: [0, 1], reasons: [null, 'Popularity not read at this hour'] } }])).answers[0]!.answer;
    assert.deepEqual(paint(unread), ['Popularity not read at this hour'], 'an unread followed cell after a known earlier bucket is missing, never the bucket');
  });
});

describe('experience and fuel', () => {
  const base = Math.floor(H1936 / 96) * 96;
  const hour = base + 90;
  const raw = (value: number) => value * 32_768;

  it('draws experience in points from the raw fixed-point series, one line per branch with its own end label', () => {
    const line = makeTiledLine('GER', [makeBucket(base, base + 95, raw(10), raw(10), raw(32.57), raw(32.57), base + 89)]);
    const model = experienceProjection.project(createBoundedReadView({
      hour,
      series: new Map([
        ['army', new Map([['GER', boundTiledSeries(line, displayedHour(hour), { headValue: knownValue(raw(32.57)) })]])],
        ['navy', new Map([['GER', boundTiledSeries(line, displayedHour(hour), { headValue: notRecorded('Navy experience was not read') })]])],
      ]),
    }), { tag: 'GER', extent: [base, base + 95], gaps: [] });
    assert.deepEqual(model.series.map(s => s.id), ['army', 'navy', 'air']);
    assert.equal(model.series[0]!.endLabel?.value, '32.57');
    assert.equal(model.series[0]!.endLabel?.name, 'Army');
    assert.equal(model.series[1]!.endLabel, null, 'a branch not recorded has no end label');
    assert.equal(model.series[2]!.headValue, null, 'a branch with no series is missing, not zero');
    const layout = layoutTimeChart(model, 960, 300, FONT, measure);
    assert.ok(layout.yDomain[1] > 32 && layout.yDomain[1] < 50, `axis in points: ${layout.yDomain[1]}`);
  });

  it('draws fuel against a dashed stepped capacity, and its hover card is the recorded day', () => {
    const fuelLine = makeTiledLine('GER', [makeBucket(base, base + 95, raw(598_000))]);
    const capLine = makeTiledLine('GER', [makeBucket(base, base + 95, raw(665_000))]);
    const reply = {
      tag: 'GER', hour: base + 50, quantityScale: 32_768, gainScale: 100_000, dayHours: 24, warnings: [], missing: null,
      read: null,
    };
    const model = fuelProjection.project(createBoundedReadView({
      hour,
      series: new Map([
        ['fuel', new Map([['GER', boundTiledSeries(fuelLine, displayedHour(hour), { headValue: knownValue(raw(598_000)) })]])],
        ['capacity', new Map([['GER', boundTiledSeries(capLine, displayedHour(hour), { headValue: knownValue(raw(665_000)) })]])],
      ]),
      hovers: new Map([['fuelHover', { hour: base + 50, data: { ...reply, missing: 'Fuel was not read on this day' } }]]),
    }), { tag: 'GER', capacity: true, area: true, digits: 0, extent: [base, base + 95], gaps: [] });
    assert.equal(model.series[0]!.endLabel?.value, '598K');
    assert.equal(model.series[1]!.role, 'ceiling');
    assert.equal(model.series[1]!.step, true);
    assert.ok(model.series[1]!.dash?.length);
    assert.equal(model.series[1]!.endLabel?.value, '665K');
    assert.equal(model.series[1]!.endLabel?.valueMuted, true);
    const tip = deriveTimeChartTooltip(model, layoutTimeChart(model, 960, 300, FONT, measure), `plot:${base + 50}`);
    const labels = tip?.payload?.items?.map(item => item.label) ?? [];
    assert.deepEqual(labels, ['Fuel was not read on this day'], 'the card is the recorded day and its missing reason, not the sampled line');
  });
});

describe('multi-country order from the product', () => {
  const base = Math.floor(H1936 / 96) * 96;
  const hour = base + 50;
  const line = (tag: string, value: number) => boundTiledSeries({ ...makeTiledLine(tag, [makeBucket(base, base + 50, value)]), decimals: 5 },
    displayedHour(hour), { headValue: knownValue(value) });
  const project = (extra: { answers?: Map<string, never>; panels?: Map<string, unknown> }) => politicalPowerProjection.project(createBoundedReadView({
    hour,
    series: new Map([['main', new Map([['GER', line('GER', 90)], ['ITA', line('ITA', 10)]])]]),
    ...(extra.answers ? { answers: extra.answers } : {}),
    ...(extra.panels ? { panels: extra.panels } : {}),
  }), { tags: ['GER', 'ITA'], extent: [base, base + 95], gaps: [], surface: '#111111' });

  it('orders lines by the followed ranking, not by the values it holds', () => {
    const rank = parseFollowed(message(10, [{ id: 1, answer: { keys: [2n, 1n], columns: 1, values: [10n, 90n], names: ['ITA', 'GER'] } }])).answers[0]!.answer;
    assert.deepEqual(project({ answers: new Map([['rank', rank as never]]) }).series.map(s => s.id), ['ITA', 'GER']);
  });

  it('orders lines by the paused ranking reply, and keeps the recorded order before any ranking', () => {
    const panels = new Map<string, unknown>([['rank', { states: [{ country: 'ITA', run: { valueRaw: '1000000' }, missing: null }, { country: 'GER', run: { valueRaw: '9000000' }, missing: null }] }]]);
    assert.deepEqual(project({ panels }).series.map(s => s.id), ['ITA', 'GER']);
    assert.deepEqual(project({}).series.map(s => s.id), ['GER', 'ITA'], 'no ranking yet: the recorded order, never a client sort');
  });

  it('takes a head from the exact read when there is no followed answer and no tiles', () => {
    const model = politicalPowerProjection.project(createBoundedReadView({
      hour,
      panels: new Map([['main', { states: [{ country: 'GER', run: { valueRaw: '2200000' }, missing: null }] }]]),
    }), { tags: ['GER'], extent: [base, base + 95], gaps: [], surface: '#111111' });
    assert.equal(model.series[0]!.headValue, 22);
    assert.deepEqual(model.auditRows?.map(row => row.value), ['22']);
  });
});

describe('several countries, one measure', () => {
  const base = Math.floor(H1936 / 96) * 96;
  const hour = base + 50;
  const tags = ['A01', 'A02', 'A03', 'A04', 'A05', 'A06', 'A07', 'A08', 'A09', 'A10'];
  const lines = new Map(tags.map((tag, at) => [tag, boundTiledSeries(makeTiledLine(tag, [makeBucket(base, base + 50, (at + 1) * 1000)]),
    displayedHour(hour), { headValue: knownValue((at + 1) * 1000) })]));
  const ranked = [...tags].reverse();
  const rank = parseFollowed(message(10, [{ id: 1, answer: { keys: ranked.map((_, at) => BigInt(at + 1)), columns: 1, values: ranked.map(() => 1n), names: ranked } }])).answers[0]!.answer;
  const project = (projection: typeof navyLinesProjection, membership: Readonly<Record<string, readonly (readonly [number, number])[]>> | null) =>
    projection.project(createBoundedReadView({ hour, series: new Map([['main', lines]]), answers: new Map([['rank', rank]]) }),
      { tags, top: null, membership, decimals: 0, digits: null, endLabels: true, second: null, options: {}, extent: [base, base + 95], gaps: [], surface: '#111111' });

  it('orders by the product ranking, colours eight and keeps the rest a grey tail without end labels', () => {
    const model = project(navyLinesProjection, null);
    assert.deepEqual(model.series.map(s => s.id), ranked);
    assert.equal(model.series.filter(s => s.role === 'primary').length, 8);
    assert.ok(model.series.slice(8).every(s => s.role === 'tail' && s.endLabel === null));
    assert.equal(model.series[0]!.endLabel?.value, '10,000');
  });

  it('gives a country that has left the selection no head and no end label, and army size reads in K and M', () => {
    const membership: Record<string, readonly (readonly [number, number])[]> = Object.fromEntries(tags.map(tag => [tag, tag === 'A10' ? [[base, base + 20] as const] : [[base, base + 95] as const]]));
    const model = project(navyLinesProjection, membership);
    const gone = model.series.find(s => s.id === 'A10')!;
    assert.equal(gone.headValue, null);
    assert.equal(gone.endLabel, null);
    assert.equal(model.series.at(-1)!.id, 'A10', 'not present at the hour: last, never ranked');
    assert.equal(project(armyLinesProjection, null).series[0]!.endLabel?.value, '10K');
  });

  it('draws a top of countries as the lines its tiles name, with exact hover readings matched by country', () => {
    const named = new Map(['A07', 'A01', 'A03'].map(tag => [tag, lines.get(tag)!]));
    const hovers = new Map([['hover', { hour: base + 10, data: { states: [
      { country: 'A01', hour: base + 10, run: { valueRaw: '1234' }, missing: null },
      { country: 'A09', hour: base + 10, run: { valueRaw: '99' }, missing: null },
      { country: 'A03', hour: base + 10, run: null, missing: 'Country did not exist' },
    ] } }]]);
    const model = navyLinesProjection.project(createBoundedReadView({ hour, series: new Map([['main', named]]), hovers }),
      { tags: [], top: 3, membership: null, decimals: 0, digits: null, endLabels: true, second: null, options: {}, extent: [base, base + 95], gaps: [], surface: '#111111' });
    assert.deepEqual(model.series.map(s => s.id), ['A07', 'A01', 'A03']);
    assert.equal(model.series.find(s => s.id === 'A07')!.exact, null);
    assert.deepEqual(model.series.find(s => s.id === 'A01')!.exact, { hour: base + 10, value: 1234 });
    assert.deepEqual(model.series.find(s => s.id === 'A03')!.exact, { hour: base + 10, value: null, reason: 'Country did not exist' });
    assert.equal(model.series[0]!.endLabel?.value, '7,000');
  });

  it('draws a second measure dashed beside each country in the same colour, keyed apart, with a legend naming both', () => {
    const named = new Map(['A07', 'A01'].map(tag => [tag, lines.get(tag)!]));
    const model = navyLinesProjection.project(createBoundedReadView({ hour, series: new Map([['main', named], ['second', named]]) }),
      { tags: ['A07', 'A01'], top: null, membership: null, decimals: 0, digits: null, endLabels: false, second: { measure: 'other', legend: ['Resistance', 'Compliance'] }, options: {},
        extent: [base, base + 95], gaps: [], surface: '#111111' });
    assert.deepEqual(model.series.map(s => [s.key.replace('\u0000', '|'), s.dash ? 'dashed' : 'solid']), [['A07', 'solid'], ['A07|second', 'dashed'], ['A01', 'solid'], ['A01|second', 'dashed']]);
    assert.equal(model.series[1]!.color, model.series[0]!.color);
    assert.ok(model.series.every(s => s.endLabel === null), 'end labels off by option');
    assert.deepEqual(model.legend?.map(entry => [entry.label, !!entry.dash]), [['Resistance', false], ['Compliance', true]]);
  });

  it('audits the plotted heads whatever the end labels show, so a wrong drawn value differs from the exact read', () => {
    const options = { tags: ['A07', 'A01'], top: null, membership: null, decimals: 0, digits: null, endLabels: false, second: null, options: {}, extent: [base, base + 95] as const, gaps: [], surface: '#111111' };
    const drawn = navyLinesProjection.project(createBoundedReadView({ hour, series: new Map([['main', lines]]) }), options);
    const state = (country: string, raw: string) => ({ country, hour, run: { valueRaw: raw }, missing: null });
    const exact = navyLinesProjection.project(createBoundedReadView({ hour, panels: new Map([['main', { decimals: 0, states: [state('A07', '7000'), state('A01', '1500')] }]]) }), options);
    const rows = (model: typeof drawn) => model.auditRows?.map(row => `${row.row}=${row.value}`);
    assert.deepEqual(rows(drawn), ['A07=7,000', 'A01=1,000'], 'labels off still audits every plotted head');
    assert.deepEqual(rows(exact), ['A07=7,000', 'A01=1,500']);
    assert.notDeepEqual(rows(drawn), rows(exact), 'the wrong drawn value is caught');
  });

  it('audits a ranked chart from the product’s exact ranked reply and hovers only members at the hovered hour', () => {
    const reading = (raw: string) => ({ hour, run: { valueRaw: raw }, missing: null });
    const ranked = { hour, measures: [{ name: 'm', decimals: 0 }], rows: [{ country: 'A09', readings: [reading('9000')] }, { country: 'A02', readings: [reading('2000')] }], refused: null, from: hour, until: hour };
    const exact = navyLinesProjection.project(createBoundedReadView({ hour, panels: new Map([['main', ranked]]) }),
      { tags: [], top: 2, membership: null, decimals: 0, digits: null, endLabels: true, second: null, options: {}, extent: [base, base + 95], gaps: [], surface: '#111111' });
    assert.deepEqual(exact.auditRows?.map(row => `${row.row}=${row.value}`), ['A09=9,000', 'A02=2,000']);

    const hovers = new Map([['hover', { hour: base + 30, data: { states: [{ country: 'A10', hour: base + 30, run: { valueRaw: '5' }, missing: null }] } }]]);
    const membership = Object.fromEntries(tags.map(tag => [tag, tag === 'A10' ? [[base, base + 20] as const, [base + 40, base + 95] as const] : [[base, base + 95] as const]]));
    const model = navyLinesProjection.project(createBoundedReadView({ hour, series: new Map([['main', lines]]), hovers }),
      { tags, top: null, membership, decimals: 0, digits: null, endLabels: true, second: null, options: {}, extent: [base, base + 95], gaps: [], surface: '#111111' });
    assert.equal(model.series.find(s => s.id === 'A10')!.exact, null, 'not a member at the hovered hour: no reading shown');
  });

  it('names, flags and colours a historical hit by the identity at the hit hour, never the playhead’s', () => {
    const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: 'b', content: 0 }, [base, base + 95]);
    index.setFacts({
      branchId: 'b',
      known: [[base, base + 95]],
      names: ['Germany', 'German Reich', 'Italy'],
      reasons: [],
      flags: [{ keys: ['GER', null, null], reasons: [null, null, null] }, { keys: ['GER_fascism', null, null], reasons: [null, null, null] }, { keys: ['ITA', null, null], reasons: [null, null, null] }],
      countries: [
        { tag: 'A07', runs: [[base, 0, 0x3366cc, -1, 0], [base + 30, 1, 0xcc3333, -1, 1]] },
        { tag: 'A01', runs: [[base, 2, 0x33aa55, -1, 2]] },
      ],
      warnings: [],
    });
    const named = new Map(['A07', 'A01'].map(tag => [tag, lines.get(tag)!]));
    const model = navyLinesProjection.project(createBoundedReadView({ hour, series: new Map([['main', named]]), identity: createSnapshotIdentityProvider(index) }),
      { tags: ['A07', 'A01'], top: null, membership: null, decimals: 0, digits: null, endLabels: true, second: null, options: {}, extent: [base, base + 95], gaps: [], surface: '#111111' });
    const layout = layoutTimeChart(model, 600, 220, FONT, measure);
    const item = (at: number) => deriveTimeChartTooltip(model, layout, `plot:${at}`)?.payload?.items?.find(entry => entry.value === '7,000');
    const early = item(base + 10)!;
    const now = item(hour)!;
    assert.equal(early.label, 'Germany', 'an earlier hit is named as the country was named then');
    assert.equal(now.label, 'German Reich');
    assert.notEqual(early.swatch, now.swatch, 'and coloured as it was coloured then');
    assert.equal(model.series[0]!.endLabel?.name, 'German Reich', 'the end label stays at the displayed hour');

    const { ctx } = mockCanvas(600, 220);
    const strokes: string[] = [];
    (ctx as unknown as { stroke: () => void }).stroke = function (this: { strokeStyle: string }) { strokes.push(this.strokeStyle); };
    drawTimeChart(ctx, model, layout, LANE_THEME, null, 1);
    assert.ok(strokes.includes(early.swatch!) && strokes.includes(now.swatch!), 'the line is drawn in each stretch’s own colour');
  });
});

describe('stacked bounded series, supply health, and ruling party', () => {
  const base = Math.floor(H1936 / 96) * 96;
  const hour = base + 240;

  it('builds stacked bands from BoundedSeries, breaks cleanly across missing spans, and never reads past hour', () => {
    const lineA = makeTiledLine('GER', [
      makeBucket(base, base + 95, 0.2),
      makeBucket(base + 96, base + 191, 0.3),
      makeBucket(base + 192, base + 287, 0.4),
    ]);
    const lineB = {
      ...makeTiledLine('GER', [
        makeBucket(base, base + 95, 0.8),
        makeBucket(base + 96, base + 191, 0.7),
        makeBucket(base + 192, base + 287, 0.6),
      ]),
      missing: [{ firstHour: base + 96, lastHour: base + 191, reason: 'Recording gap' }],
    };
    const boundedA = boundTiledSeries(lineA, displayedHour(hour), { headPoint: { hour, value: 0.4 }, headValue: knownValue(0.4) });
    const boundedB = boundTiledSeries(lineB, displayedHour(hour), { headPoint: { hour, value: 0.6 }, headValue: knownValue(0.6) });
    const stack = buildBoundedStack([
      { id: 'a', key: 'a', name: 'A', color: '#e5484d', parts: [{ series: boundedA }], valueScale: 100, head: { value: 40 } },
      { id: 'b', key: 'b', name: 'B', color: '#3fae63', parts: [{ series: boundedB }], valueScale: 100, head: { value: 60 } },
    ], hour);
    assert.ok(stack.hours.every(h => h <= hour), 'stack hours stay at or before playhead');
    const gapIdx = stack.hours.indexOf(base + 96);
    assert.ok(gapIdx >= 0);
    assert.equal(stack.bands[0]!.values[gapIdx], null, 'missing span produces a null break');
    assert.equal(stack.bands[1]!.values[gapIdx], null);
    const headIdx = stack.hours.indexOf(hour);
    assert.equal(stack.bands[0]!.values[headIdx], 40);
    assert.equal(stack.bands[1]!.values[headIdx], 60);
  });

  it('projects supply health from one reply: Rust shares, counts and total, a recorded zero, and an audit from the exact read', () => {
    const subject = '\u00007';
    const mk = (v: number) => boundTiledSeries(makeTiledLine(subject, [makeBucket(base, base + 95, v), makeBucket(base + 192, base + 287, v)]), displayedHour(hour), { headPoint: { hour, value: v }, headValue: knownValue(v) });
    const hoverAt = base + 50;
    const reading = (raw: string | null, missing: string | null = null) => ({ country: 7, hour: hoverAt, drop: null, missing,
      run: raw === null ? null : { firstHour: hoverAt, lastHour: hoverAt, valueRaw: raw, slot: 7, countryId: 1, startsSegment: false, segmentFirstHour: hoverAt, firstExact: true, lastExact: true } });
    const measures = [{ name: 'divisions', decimals: 0 }, ...['noSupply', 'strained', 'inSupply'].map(name => ({ name, decimals: 0 })),
      ...['noSupplyShare', 'strainedShare', 'inSupplyShare'].map(name => ({ name, decimals: 5 }))];
    const rankReply = (readings: ReturnType<typeof reading>[]) => ({ hour: hoverAt, data: { branchId: '1', hour: hoverAt, measures, rows: [{ country: 7, readings }], refused: null, from: hoverAt, until: hoverAt } });
    const options = { subject, extent: [base, base + 287] as const, gaps: [] };
    const model = supplyHealthProjection.project(createBoundedReadView({
      hour,
      series: new Map([
        ['noSupply', new Map([[subject, mk(0.07)]])],
        ['strained', new Map([[subject, mk(0.66)]])],
        ['inSupply', new Map([[subject, mk(0.27)]])],
      ]),
      hovers: new Map([['hover', rankReply([reading('100'), reading('7'), reading('66'), reading('27'), reading('7000'), reading('66000'), reading('27000')])]]),
    }), options);

    assert.equal(model.mode, 'stack');
    assert.deepEqual(model.stack?.bands.map(b => [b.id, b.color, b.endLabel?.value]), [
      ['noSupply', CHART_MEASURE_COLOURS.supplyNone, '7.0%'],
      ['strained', CHART_MEASURE_COLOURS.supplyStrained, '66%'],
      ['inSupply', CHART_MEASURE_COLOURS.supplyOk, '27%'],
    ]);
    const layout = layoutTimeChart(model, 720, 260, FONT, measure);
    assert.equal(layout.endLabels.length, 3);
    assert.deepEqual(layout.yTicks, [0, 50, 100]);

    const { ctx, ops } = mockCanvas(720, 260);
    drawTimeChart(ctx, model, layout, undefined, null, 1);
    assert.ok(ops.some(op => op === 'text:27%'));
    assert.ok(ops.some(op => op === 'text:66%'));
    assert.ok(ops.some(op => op === 'text:7.0%'));

    const hitBand = hitTestTimeChart(model, layout, [layout.toX(base + 20), layout.toY(90)]);
    assert.ok(hitBand?.startsWith('line:inSupply@'));
    const labelTip = deriveTimeChartTooltip(model, layout, 'label:strained');
    assert.equal(labelTip?.payload?.title, 'Strained');
    const hoverTip = deriveTimeChartTooltip(model, layout, `plot:${hoverAt}`);
    assert.deepEqual(hoverTip?.payload?.items?.map(i => `${i.label}:${i.value}`), [
      'In supply:27% · 27 divisions',
      'Strained:66% · 66 divisions',
      'No supply:7.0% · 7 divisions',
    ], 'the shares are the product’s, not a sum of three replies');

    const hoverOf = (readings: ReturnType<typeof reading>[]) => supplyHealthProjection.project(createBoundedReadView({ hour, hovers: new Map([['hover', rankReply(readings)]]) }), options).hoverDetail?.items.map(i => `${i.label}:${i.value ?? i.missingReason}`);
    assert.deepEqual(hoverOf([reading('0'), reading('0'), reading('0'), reading('0'), reading(null, 'No divisions'), reading(null, 'No divisions'), reading(null, 'No divisions')]), ['Divisions:0'], 'none only for a recorded zero');
    assert.deepEqual(hoverOf([reading(null, 'Supply was not read at this hour'), reading(null), reading('1'), reading('1'), reading(null), reading(null), reading(null)]), ['Divisions:Supply was not read at this hour']);

    const share = (raw: string) => ({ decimals: 5, states: [{ country: 7, hour, run: { valueRaw: raw }, missing: null }] });
    const audited = supplyHealthProjection.project(createBoundedReadView({ hour, panels: new Map([['noSupply', share('7000')], ['strained', share('66000')], ['inSupply', share('27000')]]) }), options);
    assert.deepEqual(audited.auditRows?.map(row => `${row.row}=${row.value}`), ['noSupply=7.0%', 'strained=66%', 'inSupply=27%'], 'the audit reads the exact panels, not tiles');
  });

  it('projects ruling party in the product’s order with dated part ownership, witnessed elections, leader note, exact hover and audit', () => {
    const subject = '\u00007';
    const reusedFrom = base + 150;
    const history = {
      branchId: '1',
      tag: 'GER',
      asked: 7,
      decimals: 5,
      warnings: [],
      unobserved: [],
      ideologies: [
        { token: 'democratic', name: 'Democratic', color: [0, 0, 255] as [number, number, number] },
        { token: 'fascism', name: 'Fascist', color: [150, 75, 0] as [number, number, number] },
        { token: 'communism', name: 'Communist', color: [200, 0, 0] as [number, number, number] },
        { token: 'neutrality', name: 'Non-aligned', color: [120, 120, 120] as [number, number, number] },
      ],
      parties: [
        { token: 'democratic', parts: [{ part: 0, firstHour: base, lastHour: reusedFrom - 1 }] },
        { token: 'fascism', parts: [{ part: 1, firstHour: base, lastHour: hour + 200 }] },
        { token: 'communism', parts: [{ part: 0, firstHour: reusedFrom, lastHour: hour + 200 }] },
        { token: 'neutrality', parts: [{ part: 2, firstHour: hour + 100, lastHour: hour + 200 }] },
      ],
      ruling: [{ token: 'fascism', firstHour: base, lastHour: hour + 200, startsSegment: true }],
      elections: [{ hour: base + 48, witnessed: base + 50 }, { hour: hour - 10, witnessed: hour + 5 }, { hour: hour + 96, witnessed: hour + 96 }],
      leaders: [{ ideology: 'nazism', person: 0, firstHour: base, lastHour: hour + 200, startsSegment: true }],
      people: [{ character: { id: 1, token: 'GER_leader', name: 'GER_leader' }, name: 'Test Leader', portrait: null, portraitMissing: null }],
      ideologyTypes: [{ token: 'nazism', name: 'Nazism' }],
      partyNames: ['Test Party'],
      partyNameRuns: [{ name: 0, firstHour: base, lastHour: hour + 200 }],
    };
    const part0 = boundTiledSeries(makeTiledLine(`${subject}\u00000`, [makeBucket(base, base + 287, 30)]), displayedHour(hour), { headPoint: { hour, value: 30 }, headValue: knownValue(30) });
    const part1 = boundTiledSeries(makeTiledLine(`${subject}\u00001`, [makeBucket(base, base + 287, 70)]), displayedHour(hour), { headPoint: { hour, value: 70 }, headValue: knownValue(70) });
    const hoverAt = base + 50;
    const options = { subject, history, extent: [base, base + 287] as const, gaps: [], surface: '#111111', showChanges: true, showLeaders: true };
    const model = rulingPartyProjection.project(createBoundedReadView({
      hour,
      series: new Map([['popularity', new Map([[`${subject}\u00000`, part0], [`${subject}\u00001`, part1]])]]),
      hovers: new Map([['popularityHover', {
        hour: hoverAt,
        data: {
          decimals: 5,
          states: [
            { country: 7, part: '0', hour: hoverAt, run: { valueRaw: '3000000' }, missing: null },
            { country: 7, part: '1', hour: hoverAt, run: { valueRaw: '7000000' }, missing: null },
          ],
        },
      }]]),
    }), options);

    assert.equal(model.mode, 'stack');
    assert.deepEqual(model.stack?.bands.map(b => [b.id, b.endLabel?.value ?? null]), [
      ['democratic', null],
      ['fascism', '70%'],
      ['communism', '30%'],
      ['neutrality', null],
    ], 'bands keep the product order whoever rules; a part read by its new party belongs to it alone');
    const stack = model.stack!;
    const at = (h: number) => stack.hours.indexOf(h);
    assert.equal(stack.bands[0]!.values[at(base)], 30, 'democratic owns part 0 before it was reused');
    assert.equal(stack.bands[2]!.values[at(base)], 0, 'communism owns nothing yet');
    assert.equal(stack.bands[0]!.values[at(hour)], 0);
    assert.equal(stack.bands[3]!.values[at(hour)], 0, 'a party that exists only later does not blank the current stack');
    assert.equal(model.note, 'Test Leader · Nazism · Test Party');
    assert.deepEqual(model.markers?.map(marker => marker.hour), [base + 48], 'an election shows at its recorded date once read, never before');
    const hoverTip = deriveTimeChartTooltip(model, layoutTimeChart(model, 720, 260, FONT, measure), `plot:${hoverAt}`);
    assert.deepEqual(hoverTip?.payload?.items?.map(i => `${i.label}=${i.value}`), [
      'Leader=Test Leader',
      'Ideology · party=Nazism · Test Party',
      'Democratic=30%',
      'Fascist (ruling)=70%',
    ], 'the hover lists the parties that existed at the hovered hour, each from its own part');

    const exact = (raw: string, part: string) => ({ country: 7, part, hour, run: { valueRaw: raw }, missing: null });
    const audited = rulingPartyProjection.project(createBoundedReadView({ hour, panels: new Map([['popularity', { decimals: 5, states: [exact('7000000', '1'), exact('3000000', '0')] }]]) }), options);
    assert.deepEqual(audited.auditRows?.map(row => `${row.row}=${row.value}`), ['fascism=70%', 'communism=30%'], 'the audit reads the exact read of each owned part');
  });

  it('installs a cleared surface when model.empty is set and draws overlays on compact wide plots', () => {
    const emptyModel: TimeChartModel = {
      kind: 'test-empty',
      measureTitle: 'Balance of power',
      mode: 'lines',
      hour: H1936 + 200,
      extent: [H1936, H1936 + 500],
      yAxis: { format: 'signed1', fixed: [-1, 1] },
      series: [],
      empty: { message: 'Germany has no balance of power in this campaign.', kind: 'empty' },
    };
    registerFamilyProjection({
      kind: 'test-empty',
      family: 'time-chart',
      variant: 'line',
      joinMode: 'step',
      measureTitle: 'Balance of power',
      project: () => emptyModel,
    });
    const created = mockCanvas(480, 180);
    const view = laneOn(created);
    const painted = view.paint(
      {
        widget: 1,
        subs: [],
        pictures: [],
        model: {
          family: 'time-chart',
          kind: 'test-empty',
          options: {},
          sources: {},
          extent: [H1936, H1936 + 500],
          width: 480,
          height: 180,
          scrollTop: 0,
          theme: LANE_THEME,
        },
        hour: H1936 + 200,
        seek: 0,
        speed: 0,
        binding: { surfaces: [{ width: 480, height: 180, dpr: 1 }], generation: 1, offset: 0, pin: null },
        inspect: null,
        now: 0,
      },
      new Map(),
      () => 'c1',
    );
    assert.equal(painted.surfaces.length, 1, 'empty model must still install a surface so page.ts leaves reading mode');
    assert.equal((painted.fields.meta?.value as { empty?: string })?.empty, 'Germany has no balance of power in this campaign.');

    const withOverlays: TimeChartModel = {
      ...emptyModel,
      empty: null,
      series: [{
        id: 'bop',
        key: 'bop',
        name: 'Power Balance',
        color: CHART_MEASURE_COLOURS.bopRight,
        role: 'primary',
        step: true,
        points: [{ hour: H1936, value: 0.2 }, { hour: H1936 + 200, value: 0.9 }],
        endLabel: {
          icon: { art: readyArt('bop'), label: 'Balance of power' },
          name: 'Very Long Consolidated Power Range Name',
          shortName: 'Very Long Consolidated Power Range Name',
          value: '0.36',
        },
      }],
      overlays: [
        { position: 'top-right', text: 'Benito Mussolini', arrow: 'right', color: CHART_MEASURE_COLOURS.bopRight },
        { position: 'bottom-right', text: 'Grand Council of Fascism', arrow: 'left', color: CHART_MEASURE_COLOURS.bopLeft },
      ],
    };
    const { ctx, ops } = mockCanvas(960, 160);
    const layout = layoutTimeChart(withOverlays, 960, 160, FONT, measure);
    drawTimeChart(ctx, withOverlays, layout, LANE_THEME, null, 1);
    const overlayTextIdx = ops.indexOf('text:Benito Mussolini');
    const plateIdx = ops.lastIndexOf('roundRect', overlayTextIdx);
    assert.ok(overlayTextIdx > 0, 'top-right overlay renders on wide 160px plot');
    assert.ok(plateIdx > 0 && plateIdx < overlayTextIdx, 'side overlay paints a surface plate behind the label so the line stays clear');
    assert.ok(ops.includes('text:Grand Council of Fascism'), 'bottom-right overlay renders on wide 160px plot');
    assert.ok(layout.endLabels[0]?.name?.endsWith('…'), 'long end-label name truncates to fit the right gutter');
    assert.equal(layout.endLabels[0]?.value, '0.36');

    const narrowLayout = layoutTimeChart(withOverlays, 320, 160, FONT, measure);
    const narrowLabel = narrowLayout.endLabels[0]!;
    assert.equal(narrowLabel.value, '0.36', 'value never truncates before the name');
    assert.ok(narrowLabel.name === null || narrowLabel.name.endsWith('…'), 'narrow end-label truncates or drops name before clipping value');
    assert.ok(narrowLabel.x + narrowLabel.width <= 320, 'fitted end-label stays inside the chart width');
  });
});
