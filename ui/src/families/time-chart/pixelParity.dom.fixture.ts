import { canvasMeasureText, clearTextWidthCache } from '../../charts/textWidth.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import { resolveLineColors, tailColour } from '../../charts/palette.ts';
import type { ChartBucket } from '../../charts/chartSummary.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import type { MissingSpan } from '../../generated/dashboard';
import { readyArt } from '../art/types.ts';
import { boundTiledSeries, BoundedSeriesCursor, displayedHour } from '../boundedView.ts';
import type { ArtHandle, FamilyTheme, IdentityResult } from '../contract.ts';
import { projectScene } from '../scene.ts';
import { knownValue } from '../value.ts';
import { drawTimeAxis, resolveKitTokens } from '../../draw/kit.ts';
import { DEFAULT_SEEDS, deriveColors, type ThemeMode } from '../../shell/appearance/themes.ts';
import { timeChartCacheState } from './cacheControl.ts';
import { chartNavStops } from './laneView.ts';
import { layoutTimeChart } from './layout.ts';
import type { ChartMonthSlot, TimeChartModel } from './model.ts';
import { drawTimeChart, fitTimeChartLegend } from './painter.ts';
import { buildBoundedStack, countrySeriesLooks, missingSpansOf, partialSpansOf } from './seriesHelpers.ts';

function makeTheme(mode: ThemeMode): FamilyTheme {
  const c = deriveColors(DEFAULT_SEEDS[mode]);
  return {
    surface: c.surface,
    raised: c.raised,
    dark: mode === 'dark',
    text: c.text,
    muted: c.muted,
    line: c.border,
    lineSoft: c.border,
    accent: c.accent,
    blue: c.accent,
    red: c.error,
    shortage: c.error,
    amber: c.warning,
    green: c.positive,
    grey: c.muted,
    hatch: c.muted,
    hatchEdge: c.border,
    font: 'Segoe UI, sans-serif',
    fontSize: { small: 11, body: 12 },
    lineWidth: 1,
    hatchSpacing: 6,
  };
}

interface ParityStep {
  readonly label: string;
  readonly mode: 'lines' | 'stack' | 'columns';
  readonly hour: number;
  readonly width: number;
  readonly height: number;
  readonly dpr: number;
  readonly theme: FamilyTheme;
  readonly tags: readonly string[];
  readonly zoom?: readonly [number, number];
  readonly inspect: string | null;
}

export interface ParityFailure {
  readonly label: string;
  readonly diffBytes: number;
  readonly firstDiff: { readonly x: number; readonly y: number; readonly channel: number; readonly cached: number; readonly ref: number } | null;
  readonly navEqual: boolean;
  readonly looksEqual: boolean;
}

function runParitySuite(): { readonly checked: number; readonly failures: ParityFailure[] } {
  const H0 = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
  const BIN = 24;
  const TOTAL_DAYS = 900;
  const EXTENT: readonly [number, number] = [H0, H0 + TOTAL_DAYS * BIN - 1];

  function makeTiled(tag: string, fn: (day: number) => number, missing: MissingSpan[] = [], partial: MissingSpan[] = []): TiledLine {
    const buckets: ChartBucket[] = [];
    for (let d = 0; d < TOTAL_DAYS; d++) {
      const firstHour = H0 + d * BIN;
      const lastHour = firstHour + BIN - 1;
      const v0 = fn(d);
      const v1 = fn(d + 0.5);
      buckets.push({
        firstHour,
        lastHour,
        firstValue: v0,
        lastValue: v1,
        lastValueHour: lastHour,
        minValue: Math.min(v0, v1),
        maxValue: Math.max(v0, v1),
        minHour: v0 <= v1 ? firstHour : lastHour,
        maxHour: v0 >= v1 ? firstHour : lastHour,
        firstBreak: false,
        hasBreak: false,
      });
    }
    return {
      tag,
      head: 'h',
      decimals: 2,
      summary: {
        from: H0,
        to: EXTENT[1],
        width: TOTAL_DAYS,
        buckets,
        tiles: { binHours: BIN, valueAt: () => null },
      },
      missing,
      partial,
      parts: [],
      marks: null,
    };
  }

  const gap1 = { firstHour: H0 + 180 * BIN, lastHour: H0 + 210 * BIN, reason: 'Unrecorded stretch' };
  const partial1: MissingSpan = { firstHour: H0 + 380 * BIN, lastHour: H0 + 440 * BIN, reason: 'Partial day only' };
  const seriesMissing: MissingSpan[] = [{ firstHour: H0 + 260 * BIN, lastHour: H0 + 290 * BIN, reason: 'No telemetry' }];

  const tiledByTag: Record<string, TiledLine> = {
    GER: makeTiled('GER', d => 120 + d * 0.65 + Math.sin(d * 0.08) * 18, [], [partial1]),
    SOV: makeTiled('SOV', d => 95 + d * 0.52 + Math.cos(d * 0.05) * 22, seriesMissing, []),
    USA: makeTiled('USA', d => 60 + d * 0.72 + Math.sin(d * 0.11) * 12, [], []),
    ENG: makeTiled('ENG', d => 110 + d * 0.35 + Math.cos(d * 0.09) * 10, [], []),
    FRA: makeTiled('FRA', d => 85 + d * 0.2 - (d > 450 ? (d - 450) * 0.25 : 0), [], []),
  };

  const identityChangeAt = H0 + 420 * BIN;
  const identityProvider = {
    resolve(_kind: string, key: string, atHour: number): IdentityResult {
      if (key === 'FRA' && atHour >= identityChangeAt) {
        return {
          kind: 'country',
          key,
          holder: { status: 'ready', value: key },
          tag: { status: 'ready', value: key },
          name: { status: 'ready', value: 'Free France' },
          colour: { status: 'ready', value: darkTheme.accent },
          flagChoice: { status: 'ready', value: 'FRA_free' },
          validInterval: [identityChangeAt, Infinity],
          nextChangeAt: Infinity,
        };
      }
      const names: Record<string, string> = {
        GER: 'German Reich',
        SOV: 'Soviet Union',
        USA: 'United States',
        ENG: 'United Kingdom',
        FRA: 'French Republic',
      };
      const until = key === 'FRA' ? identityChangeAt - 1 : Infinity;
      return {
        kind: 'country',
        key,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: { status: 'ready', value: names[key] ?? key },
        colour: { status: 'absent', reason: 'default' },
        flagChoice: { status: 'ready', value: `${key}_base` },
        validInterval: [H0, until],
        nextChangeAt: until === Infinity ? Infinity : identityChangeAt,
      };
    },
  };
  const artProvider = {
    resolve(handle: ArtHandle) {
      const label = handle.kind === 'flag' ? handle.choice : handle.kind;
      return readyArt(`art://${label}`, { assetId: label, file: `${label}.png` });
    },
  };
  const emptyScene = {
    widget: 1, subs: [], pictures: [], model: null, hour: H0, seek: 0, speed: 0,
    binding: { generation: 1, surfaces: [], offset: 0, pin: null }, inspect: null, now: 0,
  };

  const cursors = new Map<string, BoundedSeriesCursor>();
  function getBounded(tag: string, hour: number, useCache: boolean) {
    const line = tiledByTag[tag]!;
    const day = Math.max(0, Math.min(TOTAL_DAYS - 1, Math.floor((hour - H0) / BIN)));
    const headVal = line.summary.buckets[day]!.lastValue;
    const headPoint = { hour: Math.floor(hour), value: headVal };
    const headValue = knownValue(headVal);
    if (!useCache) {
      return boundTiledSeries(line, displayedHour(hour), { headPoint, headValue });
    }
    let c = cursors.get(tag);
    if (!c) cursors.set(tag, c = new BoundedSeriesCursor());
    return c.advance(line, hour, { headPoint, headValue });
  }

  const darkTheme = makeTheme('dark');
  const lightTheme = makeTheme('light');

  function buildLineModel(step: ParityStep, useCache: boolean): { readonly model: TimeChartModel; readonly artUrls: readonly string[]; readonly looksJson: string } {
    const projected = projectScene(
      emptyScene,
      { sources: {} },
      step.hour,
      { identity: identityProvider, art: artProvider, artRevision: 1 },
      view => countrySeriesLooks(step.tags, view, {
        holders: null,
        from: EXTENT[0],
        surface: step.theme.surface,
        withFlag: true,
        colouredCount: 3,
        tailColor: tailColour(step.theme.surface),
      }),
    );
    const looks = projected.model;
    const series = step.tags.map((tag, i) => {
      const bounded = getBounded(tag, step.hour, useCache);
      const headVal = bounded.headValue?.kind === 'known' ? Number(bounded.headValue.value) : null;
      const look = looks.get(tag)!;
      return {
        id: tag,
        key: tag,
        name: look.name,
        color: look.color,
        role: (i < 3 ? 'primary' : 'tail') as 'primary' | 'tail',
        step: tag === 'ENG',
        bounded,
        headValue: headVal,
        headHour: step.hour,
        identities: look.identities,
        missingSpans: missingSpansOf(bounded),
        partialSpans: partialSpansOf(bounded),
        endLabel: i < 3 && headVal !== null ? { name: look.name, fullName: look.name, value: headVal.toFixed(1) } : null,
      };
    });
    const model: TimeChartModel = {
      kind: 'parity-lines',
      measureTitle: 'Parity lines',
      mode: 'lines',
      hour: step.hour,
      extent: EXTENT,
      ...(step.zoom ? { view: { x: step.zoom } } : {}),
      yAxis: { format: 'compact' },
      series,
      gaps: [gap1],
      legend: [{ id: 'main', label: 'Primary' }, { id: 'second', label: 'Comparison', dash: [4, 3] }],
      auditRows: [],
    };
    return {
      model,
      artUrls: projected.artUrls.slice().sort(),
      looksJson: JSON.stringify([...looks.entries()].map(([k, v]) => [k, v.name, v.color, v.identities.map(r => [r.firstHour, r.lastHour, r.name, r.color])])),
    };
  }

  function buildStackModel(step: ParityStep, useCache: boolean): TimeChartModel {
    const stackTags = ['GER', 'SOV', 'USA'] as const;
    const colors = resolveLineColors(
      stackTags.map(key => ({ key, color: null })),
      step.theme.surface,
    );
    const inputs = stackTags.map(tag => {
      const bounded = getBounded(tag, step.hour, useCache);
      const headVal = bounded.headValue?.kind === 'known' ? Number(bounded.headValue.value) : 0;
      return {
        id: tag,
        key: tag,
        name: tag,
        color: colors.get(tag)!,
        parts: [{ series: bounded }],
        head: { value: headVal },
        endLabel: { name: tag, fullName: tag, value: headVal.toFixed(0) },
      };
    });
    return {
      kind: 'parity-stack',
      measureTitle: 'Parity stack',
      mode: 'stack',
      hour: step.hour,
      extent: EXTENT,
      ...(step.zoom ? { view: { x: step.zoom } } : {}),
      yAxis: { format: 'number', decimals: 0 },
      series: [],
      stack: buildBoundedStack(inputs, step.hour),
      gaps: [gap1],
      auditRows: [],
    };
  }

  function buildColumnsModel(step: ParityStep): TimeChartModel {
    const months: ChartMonthSlot[] = [];
    for (let i = 0; i < 14; i++) {
      const year = 1936 + Math.floor(i / 12);
      const month = (i % 12) + 1;
      const firstHour = engineHour({ year, month, day: 1, hour: 1 })!;
      const lastHour = firstHour + 27 * 24;
      if (i >= 3 && i <= 9) {
        months.push({ year, month, firstHour, lastHour, status: 'not-recorded', segments: [], totalValue: 0, totalText: '0' });
      } else {
        const val = (i + 1) * 150;
        months.push({
          year,
          month,
          firstHour,
          lastHour,
          status: 'recorded',
          partial: i === 12,
          segments: [{ id: 'combat', label: 'Combat', color: step.theme.red, value: val, exactText: String(val) }],
          totalValue: val,
          totalText: String(val),
        });
      }
    }
    return {
      kind: 'parity-columns',
      measureTitle: 'Parity columns',
      mode: 'columns',
      hour: step.hour,
      extent: [months[0]!.firstHour, months[months.length - 1]!.lastHour],
      yAxis: { format: 'compact' },
      series: [],
      columns: { months, foldUnrecorded: true, foldThreshold: 6 },
      auditRows: [],
    };
  }

  const steps: ParityStep[] = [
    { label: 'day-ticks-1', mode: 'lines', hour: H0 + 18 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['GER', 'SOV', 'ENG'], inspect: null },
    { label: 'day-ticks-2', mode: 'lines', hour: H0 + 45 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['GER', 'SOV', 'ENG'], inspect: null },
    { label: 'month-ticks-1', mode: 'lines', hour: H0 + 120 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['GER', 'SOV', 'ENG'], inspect: null },
    { label: 'month-ticks-cached', mode: 'lines', hour: H0 + 132 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['GER', 'SOV', 'ENG'], inspect: null },
    { label: 'gap-ongoing', mode: 'lines', hour: H0 + 195 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['GER', 'SOV', 'ENG'], inspect: null },
    { label: 'gap-passed-line-enter', mode: 'lines', hour: H0 + 275 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['GER', 'SOV', 'USA', 'ENG', 'FRA'], inspect: null },
    { label: 'partial-span-active', mode: 'lines', hour: H0 + 410 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['GER', 'USA', 'SOV', 'FRA'], inspect: null },
    { label: 'identity-change-and-partial-settled', mode: 'lines', hour: H0 + 500 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['USA', 'GER', 'FRA', 'SOV'], inspect: null },
    { label: 'line-leave-year-ticks-1', mode: 'lines', hour: H0 + 780 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['USA', 'GER', 'SOV', 'ENG'], inspect: null },
    { label: 'year-ticks-cached-hover', mode: 'lines', hour: H0 + 810 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['USA', 'GER', 'SOV', 'ENG'], inspect: `plot:${H0 + 400 * BIN}` },
    { label: 'seek-backward', mode: 'lines', hour: H0 + 240 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['GER', 'SOV', 'ENG'], inspect: null },
    { label: 'seek-forward-label-hover', mode: 'lines', hour: H0 + 850 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['USA', 'GER', 'SOV', 'FRA'], inspect: 'label:GER' },
    { label: 'zoom-in', mode: 'lines', hour: H0 + 850 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['USA', 'GER', 'SOV', 'FRA'], zoom: [H0 + 150 * BIN, H0 + 450 * BIN], inspect: null },
    { label: 'zoom-pan', mode: 'lines', hour: H0 + 860 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: ['USA', 'GER', 'SOV', 'FRA'], zoom: [H0 + 220 * BIN, H0 + 520 * BIN], inspect: null },
    { label: 'resize-small', mode: 'lines', hour: H0 + 860 * BIN, width: 380, height: 190, dpr: 1, theme: darkTheme, tags: ['USA', 'GER', 'SOV'], inspect: null },
    { label: 'fractional-dpr-125', mode: 'lines', hour: H0 + 865 * BIN, width: 720, height: 320, dpr: 1.25, theme: darkTheme, tags: ['USA', 'GER', 'SOV', 'ENG'], inspect: null },
    { label: 'fractional-dpr-125-warm', mode: 'lines', hour: H0 + 868 * BIN, width: 720, height: 320, dpr: 1.25, theme: darkTheme, tags: ['USA', 'GER', 'SOV', 'ENG'], inspect: null },
    { label: 'resize-hidpi-light', mode: 'lines', hour: H0 + 870 * BIN, width: 640, height: 300, dpr: 1.5, theme: lightTheme, tags: ['USA', 'GER', 'SOV', 'ENG'], inspect: null },
    { label: 'stack-forward', mode: 'stack', hour: H0 + 520 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: [], inspect: null },
    { label: 'stack-dpr-125', mode: 'stack', hour: H0 + 620 * BIN, width: 720, height: 320, dpr: 1.25, theme: darkTheme, tags: [], inspect: null },
    { label: 'stack-zoom-hover', mode: 'stack', hour: H0 + 820 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: [], zoom: [H0 + 100 * BIN, H0 + 600 * BIN], inspect: `plot:${H0 + 300 * BIN}` },
    { label: 'columns-folded-partial', mode: 'columns', hour: H0 + 400 * BIN, width: 720, height: 320, dpr: 1, theme: darkTheme, tags: [], inspect: null },
  ];

  const cachedCanvas = new OffscreenCanvas(720, 320);
  const cachedCtx = cachedCanvas.getContext('2d', { willReadFrequently: true })!;
  const cachedMeasure = canvasMeasureText(cachedCtx);

  const cachedResults: {
    readonly bw: number;
    readonly pixels: Uint8ClampedArray;
    readonly navJson: string;
    readonly looksJson: string | null;
    readonly artUrlsJson: string | null;
  }[] = [];

  for (const step of steps) {
    const bw = Math.round(step.width * step.dpr);
    const bh = Math.round(step.height * step.dpr);
    if (cachedCanvas.width !== bw || cachedCanvas.height !== bh) {
      cachedCanvas.width = bw;
      cachedCanvas.height = bh;
    }
    const cachedLine = step.mode === 'lines' ? buildLineModel(step, true) : null;
    const cachedModel = cachedLine ? cachedLine.model : step.mode === 'stack' ? buildStackModel(step, true) : buildColumnsModel(step);
    const cachedLayout = layoutTimeChart(cachedModel, step.width, step.height, step.theme.font, cachedMeasure);
    const cachedLegend = fitTimeChartLegend(cachedModel, cachedLayout, step.theme, cachedMeasure);
    drawTimeChart(cachedCtx, cachedModel, cachedLayout, step.theme, step.inspect, step.dpr, cachedLegend);
    cachedResults.push({
      bw,
      pixels: new Uint8ClampedArray(cachedCtx.getImageData(0, 0, bw, bh).data),
      navJson: JSON.stringify(chartNavStops(cachedModel, cachedLayout)),
      looksJson: cachedLine ? cachedLine.looksJson : null,
      artUrlsJson: cachedLine ? JSON.stringify(cachedLine.artUrls) : null,
    });
  }

  const failures: ParityFailure[] = [];
  const prevEnabled = timeChartCacheState.enabled;
  timeChartCacheState.enabled = false;
  try {
    for (let s = 0; s < steps.length; s++) {
      const step = steps[s]!;
      const cached = cachedResults[s]!;
      const bw = cached.bw;
      const bh = Math.round(step.height * step.dpr);
      const refCanvas = new OffscreenCanvas(bw, bh);
      const refCtx = refCanvas.getContext('2d', { willReadFrequently: true })!;
      clearTextWidthCache();
      const refMeasure = (text: string, font: string) => {
        const prev = refCtx.font;
        if (prev !== font) refCtx.font = font;
        const w = refCtx.measureText(text).width;
        if (prev !== font) refCtx.font = prev;
        return w;
      };
      const refLine = step.mode === 'lines' ? buildLineModel(step, false) : null;
      const refModel = refLine ? refLine.model : step.mode === 'stack' ? buildStackModel(step, false) : buildColumnsModel(step);
      const refLayout = layoutTimeChart(refModel, step.width, step.height, step.theme.font, refMeasure);
      const refLegend = fitTimeChartLegend(refModel, refLayout, step.theme, refMeasure);
      drawTimeChart(refCtx, refModel, refLayout, step.theme, step.inspect, step.dpr, refLegend);
      const refPixels = refCtx.getImageData(0, 0, bw, bh).data;
      const refNavJson = JSON.stringify(chartNavStops(refModel, refLayout));

      let diffBytes = 0;
      let firstDiff: ParityFailure['firstDiff'] = null;
      for (let i = 0; i < cached.pixels.length; i++) {
        if (cached.pixels[i] !== refPixels[i]) {
          diffBytes++;
          if (firstDiff === null) {
            const px = Math.floor(i / 4);
            firstDiff = { x: px % bw, y: Math.floor(px / bw), channel: i % 4, cached: cached.pixels[i]!, ref: refPixels[i]! };
          }
        }
      }
      const navEqual = cached.navJson === refNavJson;
      const looksEqual = !refLine || (
        cached.looksJson === refLine.looksJson
        && cached.artUrlsJson === JSON.stringify(refLine.artUrls)
      );
      if (diffBytes > 0 || !navEqual || !looksEqual) {
        failures.push({ label: step.label, diffBytes, firstDiff, navEqual, looksEqual });
      }
    }
  } finally {
    timeChartCacheState.enabled = prevEnabled;
  }

  const dpr = 1.25;
  const sw = 320;
  const sh = 180;
  const sbw = Math.round(sw * dpr);
  const sbh = Math.round(sh * dpr);
  const strokeTicks = Array.from({ length: 16 }, (_, i) => ({ hour: H0 + i * 24, label: '' }));
  const prodAxisCanvas = new OffscreenCanvas(sbw, sbh);
  const prodAxisCtx = prodAxisCanvas.getContext('2d', { willReadFrequently: true })!;
  prodAxisCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
  drawTimeAxis(prodAxisCtx, strokeTicks, {
    plotLeft: 30,
    plotRight: 62,
    startHour: H0,
    endHour: H0 + 15 * 24,
    axisTop: 80,
    playhead: null,
    playheadLabel: '',
    hairline: 'none',
    tickColor: resolveKitTokens(darkTheme).zeroLine,
    theme: darkTheme,
  });
  const refAxisCanvas = new OffscreenCanvas(sbw, sbh);
  const refAxisCtx = refAxisCanvas.getContext('2d', { willReadFrequently: true })!;
  refAxisCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
  refAxisCtx.strokeStyle = resolveKitTokens(darkTheme).zeroLine;
  refAxisCtx.lineWidth = 1;
  for (const tick of strokeTicks) {
    const x = Math.floor(30 + ((tick.hour - H0) / (15 * 24)) * 32) + 0.5;
    refAxisCtx.beginPath();
    refAxisCtx.moveTo(x, 80);
    refAxisCtx.lineTo(x, 85);
    refAxisCtx.stroke();
  }
  const prodAxisPx = prodAxisCtx.getImageData(0, 0, sbw, sbh).data;
  const refAxisPx = refAxisCtx.getImageData(0, 0, sbw, sbh).data;
  let axisDiff = 0;
  for (let i = 0; i < prodAxisPx.length; i++) if (prodAxisPx[i] !== refAxisPx[i]) axisDiff++;
  if (axisDiff > 0) failures.push({ label: 'per-stroke-x-ticks-dpr-125', diffBytes: axisDiff, firstDiff: null, navEqual: true, looksEqual: true });

  const denseGridModel: TimeChartModel = {
    kind: 'dense-grid',
    measureTitle: 'Grid',
    mode: 'lines',
    hour: H0 + 24,
    extent: [H0, H0 + 24],
    yAxis: { format: 'number', fixed: [0, 10], ticks: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10] },
    series: [],
  };
  const denseLay = layoutTimeChart(denseGridModel, 200, 60, darkTheme.font, cachedMeasure);
  const prodGridCanvas = new OffscreenCanvas(Math.round(200 * dpr), Math.round(60 * dpr));
  const prodGridCtx = prodGridCanvas.getContext('2d', { willReadFrequently: true })!;
  drawTimeChart(prodGridCtx, denseGridModel, denseLay, darkTheme, null, dpr, null);
  const refGridCanvas = new OffscreenCanvas(Math.round(200 * dpr), Math.round(60 * dpr));
  const refGridCtx = refGridCanvas.getContext('2d', { willReadFrequently: true })!;
  refGridCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const tokens = resolveKitTokens(darkTheme);
  for (const v of denseLay.yTicks) {
    const y = Math.round(denseLay.toY(v)) + 0.5;
    if (y < denseLay.top - 2 || y > denseLay.baseY + 2) continue;
    refGridCtx.save();
    refGridCtx.strokeStyle = v === 0 && denseLay.yDomain[0] < 0 ? tokens.zeroLine : tokens.gridLine;
    refGridCtx.lineWidth = 1;
    refGridCtx.beginPath();
    refGridCtx.moveTo(denseLay.left, y);
    refGridCtx.lineTo(denseLay.left + denseLay.plotWidth, y);
    refGridCtx.stroke();
    refGridCtx.restore();
  }
  const sampleX = Math.round((denseLay.left + 4) * dpr);
  const sampleY = Math.round((denseLay.top + 1) * dpr);
  const sampleW = Math.round(20 * dpr);
  const sampleH = Math.round((denseLay.plotHeight - 3) * dpr);
  const prodGridPx = prodGridCtx.getImageData(sampleX, sampleY, sampleW, sampleH).data;
  const refGridPx = refGridCtx.getImageData(sampleX, sampleY, sampleW, sampleH).data;
  let gridDiff = 0;
  for (let i = 0; i < prodGridPx.length; i++) if (prodGridPx[i] !== refGridPx[i]) gridDiff++;
  if (gridDiff > 0) failures.push({ label: 'per-stroke-y-grid-dpr-125', diffBytes: gridDiff, firstDiff: null, navEqual: true, looksEqual: true });

  return { checked: steps.length, failures };
}

(window as unknown as { timeChartPixelParity: typeof runParitySuite }).timeChartPixelParity = runParitySuite;
