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
import { knownValue } from '../value.ts';
import { formatDateOnly } from '../valueFormat.ts';
import { commandPowerProjection } from '../../widgets/command-power/commandPowerProjection.ts';
import { stabilityProjection } from '../../widgets/stability/stabilityProjection.ts';
import { registerTimeChart, type TimeChartBinding } from './declaration.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from './hitTest.ts';
import { timeChartLaneView } from './laneView.ts';
import { formatChartAxisValue, layoutTimeChart, niceChartTicks } from './layout.ts';
import type { ChartMonthSlot, TimeChartModel } from './model.ts';
import { buildLineSubpaths } from './painter.ts';

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
    assert.deepEqual(res.colour, { status: 'ready', value: 'rgb(69, 69, 69)' });
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
  it('paints a registered chart over BoundedReadView with LaneArtCache demands, hit-tests lines, emits tooltip and audit rows', () => {
    registerTimeChart({
      kind: 'test-pp',
      family: 'series',
      variant: 'line',
      joinMode: 'join',
      measureTitle: 'Political power',
      project: readView => {
        const icon = readView.art({ kind: 'sprite', key: 'GFX_pol_power', frame: 0 });
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
    assert.ok(painted.fields.artDemands);
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
    const fullTransform = (hovered.fields.meta?.value as { transform: { xFrom: number; xTo: number } }).transform;
    assert.deepEqual([fullTransform.xFrom, fullTransform.xTo], [H1936, H1936 + 100 * 24]);

    const zoomWindow: readonly [number, number] = [H1936 + 20 * 24, H1936 + 50 * 24];
    const zoomed = view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: { ...binding, view: { x: zoomWindow } },
      hour: H1936 + 100 * 24,
      speed: 0,
      binding: { generation: 2, surfaces: [{ width: 600, height: 240, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 2,
    }, state, () => `c${seq++}`);
    const zoomedTransform = (zoomed.fields.meta?.value as { transform: { xFrom: number; xTo: number } }).transform;
    assert.deepEqual([zoomedTransform.xFrom, zoomedTransform.xTo], zoomWindow, 'the bound zoom window is the drawn x domain');

    view.unmount?.(1, state);
    assert.equal(state.has('time-chart'), false);
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
      rawMarkers: [
        { id: 'w1', hour: H1936 + 100, kind: 'war', title: 'War', otherTag: 'POL', warVerb: 'began', warAlert: true },
        { id: 'future', hour: until + 48, kind: 'focus', title: 'Future focus' },
      ],
    });
    assert.equal(model.series.length, 2);
    assert.equal(model.series[0]!.step, true);
    assert.equal(model.series[0]!.valueScale, 100);
    assert.equal(model.series[0]!.endLabel?.value, '100%');
    assert.equal(model.series[1]!.endLabel?.value, '90%');
    assert.equal(model.markers?.length, 1, 'future marker must be clamped');
    assert.equal(model.markers![0]!.title, 'War with Poland began');

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
    }), { tags: ['GER'], extent: [H1936, until], gaps: [], surface: '#111111', showStability: true, showWarSupport: false, zoom: false, rawMarkers: [] });
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
