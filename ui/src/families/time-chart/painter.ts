import { measureCanvasText, textWidthRevision, type Canvas2D } from '../../charts/textWidth.ts';
import { timeChartCacheState } from './cacheControl.ts';
import {
  drawArtSlot,
  drawCrosshairLine,
  drawDateTab,
  drawFlagSlot,
  drawGapBand,
  drawLegendStrip,
  drawPlayheadLine,
  drawSelectedHourLine,
  drawSeriesMissingStrip,
  drawTimeAxis,
  fitLegendStrip,
  resolveKitTokens,
  snapCanvasText,
  type LegendFit,
  type LegendMeasure,
  type LegendStripSpec,
} from '../../draw/kit.ts';
import { parseColor, rgbaRgb } from '../../shell/appearance/color.ts';
import { resolveCachedArt, toArtSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import { collectArtUrls, type ArtUrlInput } from '../art/provider.ts';
import type { BoundedSeries, FamilyTheme } from '../contract.ts';
import { formatDateOnly } from '../valueFormat.ts';
import { parseChartInspect, sampleSeriesValueAt } from './hitTest.ts';
import { historyFits, lineHistory, LineTrace, type HistorySubpath, type LineSink, type LineWindow } from './history.ts';
import { formatChartAxisValue, type TimeChartLayout } from './layout.ts';
import { seriesLookAt, type ChartSeriesLook, type ChartStackBand, type TimeChartModel, type TimeChartSeriesModel } from './model.ts';
import { drawChartRows, parseRowsInspect } from './rows.ts';

export function timeChartLegendSpec(model: TimeChartModel, artCache?: LaneArtCache, ink?: string): LegendStripSpec | null {
  const legend = model.legend;
  if (!legend || legend.length === 0) return null;
  const isLineLegend = model.mode === 'lines';
  let ordinaryLen = legend.length;
  while (ordinaryLen > 0 && legend[ordinaryLen - 1]!.hatch) ordinaryLen--;
  const trailing = ordinaryLen < legend.length
    ? legend.slice(ordinaryLen).map(item => ({ label: item.label, color: item.color, hatch: true, muted: false }))
    : undefined;
  return {
    entries: {
      length: ordinaryLen,
      at: i => {
        const item = legend[i]!;
        return {
          label: item.label,
          color: item.color ?? ink ?? '',
          ...(isLineLegend && !item.hatch ? { line: true } : {}),
          dash: item.dash,
          hatch: item.hatch,
          flag: item.flag ? toArtSlotState(resolveCachedArt(item.flag.art, artCache), item.flag.tag ?? item.flag.label) : undefined,
          muted: false,
        };
      },
      ...(trailing ? { trailing } : {}),
    },
    entryGap: 14,
  };
}

interface CachedLegendFit {
  readonly key: string;
  readonly fit: LegendFit;
}

const legendFitCache = new WeakMap<LegendMeasure, { revision: number; map: Map<string, CachedLegendFit> }>();
const LEGEND_FIT_CACHE_MAX = 64;
const LEGEND_FIT_MAX_ENTRIES = 32;

function legendFitKey(model: TimeChartModel, layout: TimeChartLayout, theme: FamilyTheme, ink: string): string | null {
  const legend = model.legend;
  if (!legend || legend.length === 0 || legend.length > LEGEND_FIT_MAX_ENTRIES || layout.legendTop === null) return null;
  let key = `${model.mode}\u0000${layout.left}\u0000${layout.width - layout.left}\u0000${theme.font}\u0000${theme.fontSize.small}\u0000${ink}`;
  for (let i = 0; i < legend.length; i++) {
    const item = legend[i]!;
    const dash = item.dash?.length ? item.dash.join(',') : '';
    const flag = item.flag
      ? (item.flag.art.status === 'noArtExpected' ? '0' : `${item.flag.art.status}:${item.flag.tag ?? item.flag.label ?? ''}`)
      : '';
    key += `\u0001${item.label}\u0002${item.color ?? ink}\u0002${dash}\u0002${item.hatch ? 1 : 0}\u0002${flag}`;
  }
  return key;
}

export function fitTimeChartLegend(model: TimeChartModel, layout: TimeChartLayout, theme: FamilyTheme, measure: LegendMeasure): LegendFit | null {
  if (layout.legendTop === null || !model.legend?.length) return null;
  const ink = resolveKitTokens(theme).textSecondary;
  const key = timeChartCacheState.enabled ? legendFitKey(model, layout, theme, ink) : null;
  if (key !== null) {
    const rev = textWidthRevision();
    let bucket = legendFitCache.get(measure);
    if (!bucket) {
      bucket = { revision: rev, map: new Map() };
      legendFitCache.set(measure, bucket);
    } else if (bucket.revision !== rev) {
      bucket.revision = rev;
      bucket.map.clear();
    }
    const hit = bucket.map.get(key);
    if (hit !== undefined) return hit.fit;
    const spec = timeChartLegendSpec(model, undefined, ink);
    if (!spec) return null;
    const fit = fitLegendStrip(layout.left, layout.width - layout.left, spec, theme, measure);
    if (bucket.map.size >= LEGEND_FIT_CACHE_MAX) bucket.map.clear();
    bucket.map.set(key, { key, fit });
    return fit;
  }
  const spec = timeChartLegendSpec(model, undefined, ink);
  return spec ? fitLegendStrip(layout.left, layout.width - layout.left, spec, theme, measure) : null;
}

export function collectVisibleChartArt(model: TimeChartModel, layout: TimeChartLayout, legend: LegendFit | null): string[] {
  const items: ArtUrlInput[] = [];
  for (const band of model.stack?.bands ?? []) {
    if (band.identities?.length) {
      visibleBandLooks(band, layout.xDomain[0], Math.min(model.hour, layout.xDomain[1]), (_, look) => items.push(look.flag));
    }
  }
  for (const label of layout.endLabels) {
    items.push(label.flag, label.icon);
  }
  for (const m of layout.markers) {
    if (m.kind === 'flag') items.push(m.marker.flag);
    if (m.kind === 'icon') items.push(m.marker.icon);
  }
  if (legend && model.legend) {
    for (let i = 0; i < legend.shown.length; i++) items.push(model.legend[i]!.flag);
  }
  if (model.overlays) {
    for (const ov of model.overlays) items.push(ov.icon);
  }
  for (const placed of layout.rows?.rows.rows ?? []) items.push(placed.row.icon);
  return collectArtUrls(items);
}

interface LinePoints {
  readonly subpaths: readonly (readonly [number, number][])[];
  readonly end: readonly [number, number] | null;
}

function headOf(series: TimeChartSeriesModel, window: LineWindow, until: number): readonly [number, number] | null {
  const headHour = series.headHour ?? until;
  return series.headValue != null && Number.isFinite(series.headValue) && headHour >= window.from && headHour <= window.to ? [headHour, series.headValue] : null;
}

function traceBounded(trace: LineTrace, bounded: BoundedSeries, window: LineWindow, from: number): void {
  if (bounded.runs && bounded.runs.length > 0) {
    for (let i = from; i < bounded.runs.length; i++) trace.run(bounded.runs.at(i)!, window);
  } else {
    const useStraddlingRuns = bounded.straddling === 'cut' && !!bounded.straddlingRuns?.length;
    const limit = useStraddlingRuns ? bounded.buckets.closedLength : bounded.buckets.length;
    for (let i = from; i < limit; i++) trace.bucket(bounded.buckets.at(i), window);
    if (useStraddlingRuns && from <= bounded.buckets.closedLength) {
      const tail = bounded.buckets.at(bounded.buckets.closedLength);
      const runs = bounded.straddlingRuns!;
      let prevEnd = trace.prevLastHour;
      for (let r = 0; r < runs.length; r++) {
        const run = runs[r]!;
        if (trace.open && ((r === 0 && tail?.firstBreak) || run.firstHour > prevEnd + 1)) {
          trace.open = false;
        }
        trace.run(run, window);
        prevEnd = run.lastHour;
      }
      trace.prevLastHour = prevEnd;
    }
  }
}

export function buildLineSubpaths(series: TimeChartSeriesModel, layout: TimeChartLayout, until: number): LinePoints {
  const subpaths: [number, number][][] = [];
  const window = { from: layout.xDomain[0], to: layout.xDomain[1] };
  const bounded = series.bounded;
  if (bounded) {
    const sink = {
      move: (hour: number, value: number) => { subpaths.push([[layout.toX(hour), layout.toY(value)]]); },
      line: (hour: number, value: number) => { subpaths[subpaths.length - 1]!.push([layout.toX(hour), layout.toY(value)]); },
    };
    const trace = new LineTrace(series.step === true, series.valueScale ?? 1, bounded.binHours, sink);
    traceBounded(trace, bounded, window, 0);
    const head = headOf(series, window, until);
    if (head) trace.head(head[0], head[1]);
    return { subpaths, end: trace.ended ? [layout.toX(trace.lastHour), layout.toY(trace.lastValue)] : null };
  }
  let end: readonly [number, number] | null = null;
  if (series.points && series.points.length > 0) {
    const step = series.step === true;
    let cur: [number, number][] = [];
    for (const pt of series.points) {
      if (pt.hour > until) break;
      if (pt.value === null || !Number.isFinite(pt.value)) {
        if (cur.length) { subpaths.push(cur); cur = []; }
        continue;
      }
      const x = layout.toX(pt.hour);
      const y = layout.toY(pt.value);
      const hasRange = pt.min != null && pt.max != null && Number.isFinite(pt.min) && Number.isFinite(pt.max) && pt.min !== pt.max;
      if (!cur.length) {
        if (hasRange) cur.push([x, layout.toY(pt.min!)], [x, layout.toY(pt.max!)], [x, y]);
        else cur.push([x, y]);
      } else if (step) {
        cur.push([x, cur[cur.length - 1]![1]]);
        if (hasRange) cur.push([x, layout.toY(pt.min!)], [x, layout.toY(pt.max!)], [x, y]);
        else cur.push([x, y]);
      } else if (hasRange) {
        cur.push([x, layout.toY(pt.min!)], [x, layout.toY(pt.max!)], [x, y]);
      } else {
        cur.push([x, y]);
      }
      end = [x, y];
    }
    if (cur.length) subpaths.push(cur);
  }
  return { subpaths, end };
}

interface LinePath {
  readonly path: Path2D;
  readonly first: readonly [number, number];
  last: readonly [number, number];
  count: number;
}

function pathOfPoints(pts: readonly (readonly [number, number])[]): LinePath {
  const path = new Path2D();
  path.moveTo(pts[0]![0], pts[0]![1]);
  for (let i = 1; i < pts.length; i++) path.lineTo(pts[i]![0], pts[i]![1]);
  return { path, first: pts[0]!, last: pts[pts.length - 1]!, count: pts.length };
}

export interface LineVertexBuffer {
  coords: Float64Array;
  starts: Int32Array;
  counts: Int32Array;
  subpathCount: number;
  coordLen: number;
  hasEnd: boolean;
  endX: number;
  endY: number;
}

function createLineVertexBuffer(): LineVertexBuffer {
  return {
    coords: new Float64Array(512),
    starts: new Int32Array(16),
    counts: new Int32Array(16),
    subpathCount: 0,
    coordLen: 0,
    hasEnd: false,
    endX: 0,
    endY: 0,
  };
}

function ensureBufferCoords(buf: LineVertexBuffer, needed: number): Float64Array {
  if (needed <= buf.coords.length) return buf.coords;
  let cap = buf.coords.length;
  while (cap < needed) cap *= 2;
  const next = new Float64Array(cap);
  next.set(buf.coords.subarray(0, buf.coordLen));
  buf.coords = next;
  return next;
}

function pushBufferSubpath(buf: LineVertexBuffer, x: number, y: number): void {
  const s = buf.subpathCount;
  if (s >= buf.starts.length) {
    const cap = buf.starts.length * 2;
    const nextStarts = new Int32Array(cap);
    nextStarts.set(buf.starts);
    buf.starts = nextStarts;
    const nextCounts = new Int32Array(cap);
    nextCounts.set(buf.counts);
    buf.counts = nextCounts;
  }
  const off = buf.coordLen;
  const coords = ensureBufferCoords(buf, off + 2);
  coords[off] = x;
  coords[off + 1] = y;
  buf.starts[s] = off;
  buf.counts[s] = 1;
  buf.subpathCount = s + 1;
  buf.coordLen = off + 2;
}

function pushBufferPoint(buf: LineVertexBuffer, x: number, y: number): void {
  const off = buf.coordLen;
  const coords = ensureBufferCoords(buf, off + 2);
  coords[off] = x;
  coords[off + 1] = y;
  buf.counts[buf.subpathCount - 1]!++;
  buf.coordLen = off + 2;
}

const scratchBuffer = createLineVertexBuffer();
const scratchWindow = { from: 0, to: 0 };
const scratchTrace = new LineTrace(false, 1, 1, { move() {}, line() {} });
const CEILING_DASH: readonly number[] = [5, 4];

let stackXsBuf = new Float64Array(512);
let stackPosAccBuf = new Float64Array(512);
let stackNegAccBuf = new Float64Array(512);
let stackPosTopYsBuf = new Float64Array(512);
let stackNegTopYsBuf = new Float64Array(512);
let stackBotYsBuf = new Float64Array(512);
let stackTopYsBuf = new Float64Array(512);
let stackSignBuf = new Int8Array(512);

function ensureStackScratch(len: number): void {
  if (len <= stackXsBuf.length) return;
  let cap = stackXsBuf.length;
  while (cap < len) cap *= 2;
  stackXsBuf = new Float64Array(cap);
  stackPosAccBuf = new Float64Array(cap);
  stackNegAccBuf = new Float64Array(cap);
  stackPosTopYsBuf = new Float64Array(cap);
  stackNegTopYsBuf = new Float64Array(cap);
  stackBotYsBuf = new Float64Array(cap);
  stackTopYsBuf = new Float64Array(cap);
  stackSignBuf = new Int8Array(cap);
}

const linearSinkState = {
  ctx: null as Canvas2D | null,
  buf: null as LineVertexBuffer | null,
  left: 0,
  x0: 0,
  xSpan: 1,
  plotWidth: 1,
  top: 0,
  y0: 0,
  ySpan: 1,
  plotHeight: 1,
  open: false,
  toX: null as ((h: number) => number) | null,
  toY: null as ((v: number) => number) | null,
};

const directLinearSink: LineSink = {
  move(hour, value) {
    const st = linearSinkState;
    const ctx = st.ctx!;
    if (st.open) ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(
      st.left + ((hour - st.x0) / st.xSpan) * st.plotWidth,
      st.top + (1 - (value - st.y0) / st.ySpan) * st.plotHeight,
    );
    st.open = true;
  },
  line(hour, value) {
    const st = linearSinkState;
    st.ctx!.lineTo(
      st.left + ((hour - st.x0) / st.xSpan) * st.plotWidth,
      st.top + (1 - (value - st.y0) / st.ySpan) * st.plotHeight,
    );
  },
};

const directGenericSink: LineSink = {
  move(hour, value) {
    const st = linearSinkState;
    const ctx = st.ctx!;
    if (st.open) ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(st.toX!(hour), st.toY!(value));
    st.open = true;
  },
  line(hour, value) {
    const st = linearSinkState;
    st.ctx!.lineTo(st.toX!(hour), st.toY!(value));
  },
};

const bufferLinearSink: LineSink = {
  move(hour, value) {
    const st = linearSinkState;
    pushBufferSubpath(
      st.buf!,
      st.left + ((hour - st.x0) / st.xSpan) * st.plotWidth,
      st.top + (1 - (value - st.y0) / st.ySpan) * st.plotHeight,
    );
  },
  line(hour, value) {
    const st = linearSinkState;
    pushBufferPoint(
      st.buf!,
      st.left + ((hour - st.x0) / st.xSpan) * st.plotWidth,
      st.top + (1 - (value - st.y0) / st.ySpan) * st.plotHeight,
    );
  },
};

const bufferGenericSink: LineSink = {
  move(hour, value) {
    const st = linearSinkState;
    pushBufferSubpath(st.buf!, st.toX!(hour), st.toY!(value));
  },
  line(hour, value) {
    const st = linearSinkState;
    pushBufferPoint(st.buf!, st.toX!(hour), st.toY!(value));
  },
};

function emitHistorySubpathLinear(
  ctx: Canvas2D,
  sub: HistorySubpath,
  left: number,
  x0: number,
  xSpan: number,
  plotWidth: number,
  top: number,
  y0: number,
  ySpan: number,
  plotHeight: number,
): void {
  const coords = sub.coords;
  const n = sub.count * 2;
  ctx.beginPath();
  ctx.moveTo(
    left + ((coords[0]! - x0) / xSpan) * plotWidth,
    top + (1 - (coords[1]! - y0) / ySpan) * plotHeight,
  );
  for (let i = 2; i < n; i += 2) {
    ctx.lineTo(
      left + ((coords[i]! - x0) / xSpan) * plotWidth,
      top + (1 - (coords[i + 1]! - y0) / ySpan) * plotHeight,
    );
  }
}

function appendHistorySubpathLinear(
  buf: LineVertexBuffer,
  sub: HistorySubpath,
  left: number,
  x0: number,
  xSpan: number,
  plotWidth: number,
  top: number,
  y0: number,
  ySpan: number,
  plotHeight: number,
): void {
  const count = sub.count;
  const s = buf.subpathCount;
  if (s >= buf.starts.length) {
    const cap = buf.starts.length * 2;
    const nextStarts = new Int32Array(cap);
    nextStarts.set(buf.starts);
    buf.starts = nextStarts;
    const nextCounts = new Int32Array(cap);
    nextCounts.set(buf.counts);
    buf.counts = nextCounts;
  }
  const base = buf.coordLen;
  const n = count * 2;
  const out = ensureBufferCoords(buf, base + n);
  const src = sub.coords;
  for (let i = 0; i < n; i += 2) {
    out[base + i] = left + ((src[i]! - x0) / xSpan) * plotWidth;
    out[base + i + 1] = top + (1 - (src[i + 1]! - y0) / ySpan) * plotHeight;
  }
  buf.starts[s] = base;
  buf.counts[s] = count;
  buf.subpathCount = s + 1;
  buf.coordLen = base + n;
}

function emitBufferSubpath(ctx: Canvas2D, buf: LineVertexBuffer, subIndex: number): void {
  const coords = buf.coords;
  const start = buf.starts[subIndex]!;
  const end = start + buf.counts[subIndex]! * 2;
  ctx.beginPath();
  ctx.moveTo(coords[start]!, coords[start + 1]!);
  for (let i = start + 2; i < end; i += 2) {
    ctx.lineTo(coords[i]!, coords[i + 1]!);
  }
}

function fillPointsIntoBuffer(series: TimeChartSeriesModel, layout: TimeChartLayout, until: number, out: LineVertexBuffer): void {
  const pts = series.points;
  if (!pts || pts.length === 0) return;
  const step = series.step === true;
  let open = false;
  let prevY = 0;
  for (let i = 0; i < pts.length; i++) {
    const pt = pts[i]!;
    if (pt.hour > until) break;
    if (pt.value === null || !Number.isFinite(pt.value)) {
      open = false;
      continue;
    }
    const x = layout.toX(pt.hour);
    const y = layout.toY(pt.value);
    const hasRange = pt.min != null && pt.max != null && Number.isFinite(pt.min) && Number.isFinite(pt.max) && pt.min !== pt.max;
    if (!open) {
      if (hasRange) {
        pushBufferSubpath(out, x, layout.toY(pt.min!));
        pushBufferPoint(out, x, layout.toY(pt.max!));
        pushBufferPoint(out, x, y);
      } else {
        pushBufferSubpath(out, x, y);
      }
      open = true;
    } else if (step) {
      pushBufferPoint(out, x, prevY);
      if (hasRange) {
        pushBufferPoint(out, x, layout.toY(pt.min!));
        pushBufferPoint(out, x, layout.toY(pt.max!));
        pushBufferPoint(out, x, y);
      } else {
        pushBufferPoint(out, x, y);
      }
    } else if (hasRange) {
      pushBufferPoint(out, x, layout.toY(pt.min!));
      pushBufferPoint(out, x, layout.toY(pt.max!));
      pushBufferPoint(out, x, y);
    } else {
      pushBufferPoint(out, x, y);
    }
    prevY = y;
    out.hasEnd = true;
    out.endX = x;
    out.endY = y;
  }
}

export function fillLineBuffer(
  series: TimeChartSeriesModel,
  layout: TimeChartLayout,
  until: number,
  out: LineVertexBuffer = scratchBuffer,
): LineVertexBuffer {
  out.subpathCount = 0;
  out.coordLen = 0;
  out.hasEnd = false;
  const bounded = series.bounded;
  scratchWindow.from = layout.xDomain[0];
  scratchWindow.to = layout.xDomain[1];
  const history = bounded && !layout.logShown ? lineHistory(bounded, series.step === true, series.valueScale ?? 1) : null;
  if (bounded && history && historyFits(history, scratchWindow)) {
    const x0 = layout.xDomain[0];
    const xSpan = layout.xDomain[1] - x0;
    const y0 = layout.yDomain[0];
    const ySpan = layout.yDomain[1] - y0;
    const left = layout.left;
    const plotWidth = layout.plotWidth;
    const top = layout.top;
    const plotHeight = layout.plotHeight;
    const subs = history.subpaths;
    for (let i = 0; i < subs.length; i++) {
      appendHistorySubpathLinear(out, subs[i]!, left, x0, xSpan, plotWidth, top, y0, ySpan, plotHeight);
    }
    const tailLen = bounded.runs && bounded.runs.length > 0 ? bounded.runs.length : bounded.buckets.length;
    const headHour = series.headHour ?? until;
    const hasHead = series.headValue != null && Number.isFinite(series.headValue) && headHour >= scratchWindow.from && headHour <= scratchWindow.to;
    if (history.items < tailLen || hasHead) {
      const st = linearSinkState;
      st.buf = out;
      st.left = left;
      st.x0 = x0;
      st.xSpan = xSpan;
      st.plotWidth = plotWidth;
      st.top = top;
      st.y0 = y0;
      st.ySpan = ySpan;
      st.plotHeight = plotHeight;
      scratchTrace.copyFrom(history.trace, bufferLinearSink);
      if (history.items < tailLen) traceBounded(scratchTrace, bounded, scratchWindow, history.items);
      if (hasHead) scratchTrace.head(headHour, series.headValue!);
      if (scratchTrace.ended) {
        out.hasEnd = true;
        out.endX = left + ((scratchTrace.lastHour - x0) / xSpan) * plotWidth;
        out.endY = top + (1 - (scratchTrace.lastValue - y0) / ySpan) * plotHeight;
      }
    } else if (history.trace.ended) {
      out.hasEnd = true;
      out.endX = left + ((history.trace.lastHour - x0) / xSpan) * plotWidth;
      out.endY = top + (1 - (history.trace.lastValue - y0) / ySpan) * plotHeight;
    }
    return out;
  }
  if (bounded) {
    const st = linearSinkState;
    st.buf = out;
    st.toX = layout.toX;
    st.toY = layout.toY;
    scratchTrace.configure(series.step === true, series.valueScale ?? 1, bounded.binHours, bufferGenericSink);
    traceBounded(scratchTrace, bounded, scratchWindow, 0);
    const headHour = series.headHour ?? until;
    if (series.headValue != null && Number.isFinite(series.headValue) && headHour >= scratchWindow.from && headHour <= scratchWindow.to) {
      scratchTrace.head(headHour, series.headValue);
    }
    if (scratchTrace.ended) {
      out.hasEnd = true;
      out.endX = layout.toX(scratchTrace.lastHour);
      out.endY = layout.toY(scratchTrace.lastValue);
    }
    return out;
  }
  fillPointsIntoBuffer(series, layout, until, out);
  return out;
}

function strokeLineDirect(
  ctx: Canvas2D,
  series: TimeChartSeriesModel,
  layout: TimeChartLayout,
  until: number,
  outEnd: { hasEnd: boolean; x: number; y: number },
): void {
  outEnd.hasEnd = false;
  const bounded = series.bounded;
  scratchWindow.from = layout.xDomain[0];
  scratchWindow.to = layout.xDomain[1];
  const history = bounded && !layout.logShown ? lineHistory(bounded, series.step === true, series.valueScale ?? 1) : null;
  if (bounded && history && historyFits(history, scratchWindow)) {
    const x0 = layout.xDomain[0];
    const xSpan = layout.xDomain[1] - x0;
    const y0 = layout.yDomain[0];
    const ySpan = layout.yDomain[1] - y0;
    const left = layout.left;
    const plotWidth = layout.plotWidth;
    const top = layout.top;
    const plotHeight = layout.plotHeight;
    const subs = history.subpaths;
    const lastSub = subs.length - 1;
    for (let i = 0; i < lastSub; i++) {
      emitHistorySubpathLinear(ctx, subs[i]!, left, x0, xSpan, plotWidth, top, y0, ySpan, plotHeight);
      ctx.stroke();
    }
    const tailLen = bounded.runs && bounded.runs.length > 0 ? bounded.runs.length : bounded.buckets.length;
    const headHour = series.headHour ?? until;
    const hasHead = series.headValue != null && Number.isFinite(series.headValue) && headHour >= scratchWindow.from && headHour <= scratchWindow.to;
    if (history.items < tailLen || hasHead) {
      const st = linearSinkState;
      st.ctx = ctx;
      st.left = left;
      st.x0 = x0;
      st.xSpan = xSpan;
      st.plotWidth = plotWidth;
      st.top = top;
      st.y0 = y0;
      st.ySpan = ySpan;
      st.plotHeight = plotHeight;
      if (lastSub >= 0) {
        emitHistorySubpathLinear(ctx, subs[lastSub]!, left, x0, xSpan, plotWidth, top, y0, ySpan, plotHeight);
        st.open = true;
      } else {
        st.open = false;
      }
      scratchTrace.copyFrom(history.trace, directLinearSink);
      if (history.items < tailLen) traceBounded(scratchTrace, bounded, scratchWindow, history.items);
      if (hasHead) scratchTrace.head(headHour, series.headValue!);
      if (st.open) ctx.stroke();
      st.ctx = null;
      if (scratchTrace.ended) {
        outEnd.hasEnd = true;
        outEnd.x = left + ((scratchTrace.lastHour - x0) / xSpan) * plotWidth;
        outEnd.y = top + (1 - (scratchTrace.lastValue - y0) / ySpan) * plotHeight;
      }
    } else {
      if (lastSub >= 0) {
        emitHistorySubpathLinear(ctx, subs[lastSub]!, left, x0, xSpan, plotWidth, top, y0, ySpan, plotHeight);
        ctx.stroke();
      }
      if (history.trace.ended) {
        outEnd.hasEnd = true;
        outEnd.x = left + ((history.trace.lastHour - x0) / xSpan) * plotWidth;
        outEnd.y = top + (1 - (history.trace.lastValue - y0) / ySpan) * plotHeight;
      }
    }
    return;
  }
  if (bounded) {
    const st = linearSinkState;
    st.ctx = ctx;
    st.toX = layout.toX;
    st.toY = layout.toY;
    st.open = false;
    scratchTrace.configure(series.step === true, series.valueScale ?? 1, bounded.binHours, directGenericSink);
    traceBounded(scratchTrace, bounded, scratchWindow, 0);
    const headHour = series.headHour ?? until;
    if (series.headValue != null && Number.isFinite(series.headValue) && headHour >= scratchWindow.from && headHour <= scratchWindow.to) {
      scratchTrace.head(headHour, series.headValue);
    }
    if (st.open) ctx.stroke();
    st.ctx = null;
    if (scratchTrace.ended) {
      outEnd.hasEnd = true;
      outEnd.x = layout.toX(scratchTrace.lastHour);
      outEnd.y = layout.toY(scratchTrace.lastValue);
    }
    return;
  }
  const buf = fillLineBuffer(series, layout, until, scratchBuffer);
  for (let i = 0; i < buf.subpathCount; i++) {
    emitBufferSubpath(ctx, buf, i);
    ctx.stroke();
  }
  if (buf.hasEnd) {
    outEnd.hasEnd = true;
    outEnd.x = buf.endX;
    outEnd.y = buf.endY;
  }
}

function sameDash(a: readonly number[] | null, b: readonly number[] | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function visibleBandLooks(band: ChartStackBand, first: number, last: number, visit: (hour: number, look: ChartSeriesLook) => void): void {
  if (first > last) return;
  const runs = band.identities!;
  let lo = 0, hi = runs.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (runs[mid]!.lastHour < first) lo = mid + 1;
    else hi = mid;
  }
  let at = first;
  for (let i = lo; i < runs.length && runs[i]!.firstHour <= last; i++) {
    const run = runs[i]!;
    if (at < run.firstHour) visit(at, band);
    visit(Math.max(at, run.firstHour), run);
    at = run.lastHour + 1;
  }
  if (at <= last) visit(at, band);
}

function bandFill(ctx: Canvas2D, band: ChartStackBand, layout: TimeChartLayout, hour: number): string | CanvasGradient {
  const [first, end] = layout.xDomain;
  const last = Math.min(end, hour);
  let color: string | undefined;
  let gradient: CanvasGradient | undefined;
  visibleBandLooks(band, first, last, (at, look) => {
    if (color !== undefined && color !== look.color && end > first) {
      gradient ??= ctx.createLinearGradient(layout.toX(first), 0, layout.toX(end), 0);
      const offset = (at - first) / (end - first);
      gradient.addColorStop(offset, color);
      gradient.addColorStop(offset, look.color);
    }
    color = look.color;
  });
  return gradient ?? color ?? band.color;
}

function colourStretches(s: TimeChartSeriesModel, layout: TimeChartLayout, hour: number): readonly { readonly color: string; readonly x0: number; readonly x1: number }[] | null {
  const runs = s.identities;
  if (!runs || runs.length < 2) return null;
  const out: { color: string; x0: number; x1: number }[] = [];
  const pad = 4;
  for (const run of runs) {
    if (run.lastHour < layout.xDomain[0] || run.firstHour > Math.min(hour, layout.xDomain[1])) continue;
    const x0 = layout.toX(Math.max(run.firstHour, layout.xDomain[0]));
    const x1 = layout.toX(Math.min(run.lastHour + 1, layout.xDomain[1]));
    const prev = out[out.length - 1];
    if (prev && prev.color === run.color) prev.x1 = x1;
    else out.push({ color: run.color, x0, x1 });
  }
  if (out.length === 0 || (out.length === 1 && out[0]!.color === s.color)) return null;
  out[0]!.x0 -= pad;
  out[out.length - 1]!.x1 += pad;
  return out;
}

interface BackdropArtSnapshot {
  readonly status: string;
  readonly bitmap: unknown;
  readonly reason: string | undefined;
}

interface TimeChartBackdropEntry {
  canvas: OffscreenCanvas | null;
  ctx: OffscreenCanvasRenderingContext2D | null;
  bw: number;
  bh: number;
  dpr: number;
  colorSpace: PredefinedColorSpace;
  model: TimeChartModel | null;
  layout: TimeChartLayout | null;
  hoveredSeriesId: string | null;
  legendFit: LegendFit | null;
  themeKey: string;
  textRev: number;
  artSnapshot: BackdropArtSnapshot[];
}

const backdropCache = new WeakMap<object, TimeChartBackdropEntry>();
const tcCacheKey = (ctx: Canvas2D): object =>
  (ctx as unknown as { __bpWidgetKey?: object }).__bpWidgetKey ?? ctx;

export function clearTimeChartBackdrop(ctx: Canvas2D): void {
  backdropCache.delete(tcCacheKey(ctx));
}

function backdropThemeKey(theme: FamilyTheme): string {
  return [
    theme.surface,
    theme.raised ?? '',
    theme.dark ? '1' : '0',
    theme.text,
    theme.muted,
    theme.line,
    theme.lineSoft,
    theme.hairline ?? '',
    theme.hairlineStrong ?? '',
    theme.bandDivider ?? '',
    theme.headerBand ?? '',
    theme.rowHover ?? '',
    theme.rowSelected ?? '',
    theme.childTint ?? '',
    theme.accent,
    theme.blue,
    theme.red,
    theme.shortage ?? '',
    theme.amber,
    theme.green,
    theme.grey,
    theme.barMuted ?? '',
    theme.barTrack ?? '',
    theme.gridLine ?? '',
    theme.zeroLine ?? '',
    theme.tailLine ?? '',
    theme.playhead ?? '',
    theme.crosshair ?? '',
    theme.dateTabBg ?? '',
    theme.dateTabText ?? '',
    theme.hoverTabBg ?? '',
    theme.hoverTabText ?? '',
    theme.hatch,
    theme.hatchEdge ?? '',
    theme.font,
    String(theme.fontSize.small),
    String(theme.fontSize.body),
    String(theme.lineWidth),
    String(theme.hatchSpacing),
  ].join('\u0000');
}

function artResultEntry(art: Parameters<typeof resolveCachedArt>[0], artCache: LaneArtCache | undefined): BackdropArtSnapshot {
  const res = resolveCachedArt(art, artCache);
  return {
    status: res.status,
    bitmap: res.status === 'ready' ? res.bitmap : undefined,
    reason: 'reason' in res ? res.reason : undefined,
  };
}

function sameArtEntry(
  snap: BackdropArtSnapshot,
  art: Parameters<typeof resolveCachedArt>[0],
  artCache: LaneArtCache | undefined,
): boolean {
  const res = resolveCachedArt(art, artCache);
  if (snap.status !== res.status) return false;
  const bitmap = res.status === 'ready' ? res.bitmap : undefined;
  if (snap.bitmap !== bitmap) return false;
  const reason = 'reason' in res ? res.reason : undefined;
  return snap.reason === reason;
}

function collectBackdropArtSnapshot(
  model: TimeChartModel,
  layout: TimeChartLayout,
  legendFit: LegendFit | null,
  artCache: LaneArtCache | undefined,
): BackdropArtSnapshot[] {
  const out: BackdropArtSnapshot[] = [];
  if (layout.legendTop !== null && model.legend) {
    const legendLimit = legendFit ? Math.min(legendFit.shown.length, model.legend.length) : model.legend.length;
    for (let i = 0; i < legendLimit; i++) {
      const flag = model.legend[i]!.flag;
      if (flag) out.push(artResultEntry(flag.art, artCache));
    }
  }
  for (let i = 0; i < layout.endLabels.length; i++) {
    const label = layout.endLabels[i]!;
    if (label.flag && label.flag.art.status !== 'noArtExpected') {
      out.push(artResultEntry(label.flag.art, artCache));
    } else if (label.icon && label.icon.art.status !== 'noArtExpected') {
      out.push(artResultEntry(label.icon.art, artCache));
    }
  }
  for (let i = 0; i < layout.markers.length; i++) {
    const m = layout.markers[i]!;
    if (m.kind === 'flag' && m.marker.flag) {
      out.push(artResultEntry(m.marker.flag.art, artCache));
    } else if (m.kind === 'icon' && m.marker.icon) {
      out.push(artResultEntry(m.marker.icon.art, artCache));
    }
  }
  return out;
}

function sameBackdropArtSnapshot(
  prev: readonly BackdropArtSnapshot[],
  model: TimeChartModel,
  layout: TimeChartLayout,
  legendFit: LegendFit | null,
  artCache: LaneArtCache | undefined,
): boolean {
  let idx = 0;
  if (layout.legendTop !== null && model.legend) {
    const legendLimit = legendFit ? Math.min(legendFit.shown.length, model.legend.length) : model.legend.length;
    for (let i = 0; i < legendLimit; i++) {
      const flag = model.legend[i]!.flag;
      if (!flag) continue;
      if (idx >= prev.length || !sameArtEntry(prev[idx++]!, flag.art, artCache)) return false;
    }
  }
  for (let i = 0; i < layout.endLabels.length; i++) {
    const label = layout.endLabels[i]!;
    if (label.flag && label.flag.art.status !== 'noArtExpected') {
      if (idx >= prev.length || !sameArtEntry(prev[idx++]!, label.flag.art, artCache)) return false;
    } else if (label.icon && label.icon.art.status !== 'noArtExpected') {
      if (idx >= prev.length || !sameArtEntry(prev[idx++]!, label.icon.art, artCache)) return false;
    }
  }
  for (let i = 0; i < layout.markers.length; i++) {
    const m = layout.markers[i]!;
    if (m.kind === 'flag' && m.marker.flag) {
      if (idx >= prev.length || !sameArtEntry(prev[idx++]!, m.marker.flag.art, artCache)) return false;
    } else if (m.kind === 'icon' && m.marker.icon) {
      if (idx >= prev.length || !sameArtEntry(prev[idx++]!, m.marker.icon.art, artCache)) return false;
    }
  }
  return idx === prev.length;
}

function backdropColorSpaceOf(ctx: Canvas2D): PredefinedColorSpace | null {
  const attrs = typeof (ctx as { readonly getContextAttributes?: () => CanvasRenderingContext2DSettings }).getContextAttributes === 'function'
    ? (ctx as { readonly getContextAttributes: () => CanvasRenderingContext2DSettings }).getContextAttributes()
    : null;
  if (attrs && attrs.alpha === false) return null;
  return attrs?.colorSpace ?? 'srgb';
}

function drawTimeChartBackdrop(
  ctx: Canvas2D,
  model: TimeChartModel,
  layout: TimeChartLayout,
  theme: FamilyTheme,
  tokens: ReturnType<typeof resolveKitTokens>,
  parsedInspect: ReturnType<typeof parseChartInspect>,
  hoveredSeriesId: string | null,
  legendFit: LegendFit | null | undefined,
  artCache: LaneArtCache | undefined,
): void {
  const smallFont = `${theme.fontSize.small}px ${theme.font}`;
  const labelNameFont = `500 ${theme.fontSize.body}px ${theme.font}`;
  const labelValueFont = `600 ${theme.fontSize.body}px ${theme.font}`;
  const overlayFont = `${theme.fontSize.body}px ${theme.font}`;
  const textSecondary = tokens.textSecondary;

  const legendSpec = layout.legendTop === null ? null : timeChartLegendSpec(model, artCache, tokens.textSecondary);
  if (legendSpec && layout.legendTop !== null) {
    const width = layout.width - layout.left;
    drawLegendStrip(ctx, layout.left, layout.legendTop, width, 14, legendSpec, theme,
      legendFit ?? fitLegendStrip(layout.left, width, legendSpec, theme, (text, font) => measureCanvasText(ctx, text, font)));
  }

  const visibleYTicks: { readonly index: number; readonly value: number; readonly y: number }[] = [];
  if (layout.yTicks.length > 0) {
    ctx.save();
    ctx.lineWidth = 1;
    for (let i = 0; i < layout.yTicks.length; i++) {
      const v = layout.yTicks[i]!;
      const y = Math.round(layout.toY(v)) + 0.5;
      if (y < layout.top - 2 || y > layout.baseY + 2) continue;
      visibleYTicks.push({ index: i, value: v, y });
      const isZeroCrossing = v === 0 && layout.yDomain[0] < 0;
      ctx.strokeStyle = isZeroCrossing ? tokens.zeroLine : tokens.gridLine;
      ctx.beginPath();
      ctx.moveTo(layout.left, y);
      ctx.lineTo(layout.left + layout.plotWidth, y);
      ctx.stroke();
    }
    ctx.restore();
  }
  if (visibleYTicks.length > 0) {
    const minGap = theme.fontSize.small + 2;
    const placedY: number[] = [];
    const lastIdx = visibleYTicks.length - 1;
    const order = lastIdx === 0
      ? [0]
      : [0, lastIdx, ...Array.from({ length: lastIdx - 1 }, (_, i) => i + 1)];
    ctx.save();
    ctx.font = smallFont;
    ctx.fillStyle = theme.muted;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    for (const idx of order) {
      const tick = visibleYTicks[idx]!;
      if (placedY.some(prev => Math.abs(prev - tick.y) < minGap)) continue;
      placedY.push(tick.y);
      const label = layout.yTickLabels?.[tick.index] ?? formatChartAxisValue(tick.value, model.yAxis.format, layout.yTicks);
      ctx.fillText(label, layout.left - 8, tick.y);
    }
    ctx.restore();
  }

  for (const g of layout.gaps) {
    drawGapBand(ctx, g.x0, layout.top, g.x1 - g.x0, layout.plotHeight, theme);
  }

  if (model.mode === 'columns') {
    const hoveredColIdx = parsedInspect?.kind === 'month' || parsedInspect?.kind === 'fold' ? parsedInspect.index : -1;
    for (const col of layout.columns) {
      if (col.index === hoveredColIdx) {
        ctx.save();
        ctx.fillStyle = theme.text;
        ctx.globalAlpha = 0.055;
        ctx.fillRect(col.x, layout.top, col.width, layout.plotHeight);
        ctx.restore();
      }
      if (col.kind === 'fold') {
        drawGapBand(ctx, col.barX, layout.top, col.barWidth, layout.plotHeight, theme);
        continue;
      }
      const m = col.month!;
      if (m.status !== 'recorded') {
        drawGapBand(ctx, col.barX, layout.top, col.barWidth, layout.plotHeight, theme);
        continue;
      }
      let yPos = layout.toY(0);
      let yNeg = layout.toY(0);
      for (const seg of m.segments) {
        if (seg.value === 0) continue;
        const h = Math.abs(layout.toY(0) - layout.toY(seg.value));
        ctx.save();
        ctx.globalAlpha = m.partial ? 0.55 : 1;
        ctx.fillStyle = seg.color;
        if (seg.value > 0) {
          ctx.fillRect(col.barX, yPos - h, col.barWidth, h);
          yPos -= h;
        } else {
          ctx.fillRect(col.barX, yNeg, col.barWidth, h);
          yNeg += h;
        }
        ctx.restore();
      }
    }
  } else if (model.mode === 'stack' && model.stack && model.stack.hours.length > 0) {
    const { hours, bands, total, step } = model.stack;
    const fills = bands.some(band => band.identities?.length)
      ? bands.map(band => band.identities?.length ? bandFill(ctx, band, layout, model.hour) : band.color)
      : null;
    let count = 0;
    while (count < hours.length && hours[count]! <= model.hour) count++;
    if (count > 0 && step) {
      const gapBetween = (a: number, b: number) =>
        model.gaps?.some(g => g.firstHour > a && g.firstHour < b) ?? false;
      const isNullAt = (i: number) => (total?.values[i] == null) && bands.every(b => b.values[i] == null);
      const runs: { readonly start: number; readonly end: number; readonly endHour: number }[] = [];
      let runStart = -1;
      for (let i = 0; i < count; i++) {
        if (isNullAt(i)) {
          if (runStart >= 0) {
            runs.push({ start: runStart, end: i - 1, endHour: hours[i]! });
            runStart = -1;
          }
          continue;
        }
        if (runStart >= 0 && gapBetween(hours[i - 1]!, hours[i]!)) {
          const hitGap = model.gaps!.find(g => g.firstHour > hours[i - 1]! && g.firstHour < hours[i]!);
          runs.push({ start: runStart, end: i - 1, endHour: hitGap ? hitGap.firstHour : hours[i - 1]! });
          runStart = i;
        } else if (runStart < 0) {
          runStart = i;
        }
      }
      if (runStart >= 0) {
        const trailingGap = model.gaps?.find(g => g.firstHour > hours[count - 1]! && g.firstHour <= model.hour);
        runs.push({ start: runStart, end: count - 1, endHour: trailingGap ? trailingGap.firstHour : Math.min(layout.xDomain[1], model.hour) });
      }
      if (runs.length > 0) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, layout.plotHeight + 4);
        ctx.clip();
        const stackZeroY = layout.toY(0);
        for (const run of runs) {
          const len = run.end - run.start + 1;
          ensureStackScratch(len + 1);
          const xs = stackXsBuf;
          const posAcc = stackPosAccBuf;
          const negAcc = stackNegAccBuf;
          const posTopYs = stackPosTopYsBuf;
          const negTopYs = stackNegTopYsBuf;
          const botYs = stackBotYsBuf;
          const topYs = stackTopYsBuf;
          const sign = stackSignBuf;
          posAcc.fill(0, 0, len);
          negAcc.fill(0, 0, len);
          posTopYs.fill(stackZeroY, 0, len);
          negTopYs.fill(stackZeroY, 0, len);
          for (let k = 0; k < len; k++) xs[k] = layout.toX(hours[run.start + k]!);
          const endX = layout.toX(Math.min(run.endHour, layout.xDomain[1], model.hour));
          xs[len] = endX;
          for (let bandIdx = 0; bandIdx < bands.length; bandIdx++) {
            const band = bands[bandIdx]!;
            const values = band.values;
            sign.fill(0, 0, len);
            for (let k = 0; k < len; k++) {
              const v = values[run.start + k];
              if (v == null || !Number.isFinite(v) || v === 0) continue;
              if (v > 0) {
                const top = posAcc[k]! + v;
                posAcc[k] = top;
                botYs[k] = posTopYs[k]!;
                const ty = layout.toY(top);
                topYs[k] = ty;
                posTopYs[k] = ty;
                sign[k] = 1;
              } else {
                const top = negAcc[k]! + v;
                negAcc[k] = top;
                botYs[k] = negTopYs[k]!;
                const ty = layout.toY(top);
                topYs[k] = ty;
                negTopYs[k] = ty;
                sign[k] = -1;
              }
            }
            let k = 0;
            while (k < len) {
              const s = sign[k]!;
              if (s === 0) { k++; continue; }
              const subStart = k;
              while (k < len && sign[k] === s) k++;
              const subEnd = k - 1;
              const subEndX = xs[subEnd + 1]!;
              ctx.beginPath();
              for (let j = subStart; j <= subEnd; j++) {
                const x = xs[j]!;
                const y = topYs[j]!;
                if (j === subStart) ctx.moveTo(x, y);
                else {
                  ctx.lineTo(x, topYs[j - 1]!);
                  ctx.lineTo(x, y);
                }
              }
              if (subEndX > xs[subEnd]!) {
                ctx.lineTo(subEndX, topYs[subEnd]!);
                ctx.lineTo(subEndX, botYs[subEnd]!);
              }
              for (let j = subEnd; j >= subStart; j--) {
                const y = botYs[j]!;
                if (j < subEnd) {
                  ctx.lineTo(xs[j + 1]!, y);
                }
                ctx.lineTo(xs[j]!, y);
              }
              ctx.closePath();
              ctx.fillStyle = fills?.[bandIdx] ?? band.color;
              ctx.globalAlpha = (band.opacity ?? 0.78) * (hoveredSeriesId && hoveredSeriesId !== band.id ? 0.35 : 1);
              ctx.fill();
              if (bandIdx < bands.length - 1) {
                ctx.beginPath();
                for (let j = subStart; j <= subEnd; j++) {
                  const x0 = xs[j]!;
                  const x1 = xs[j + 1]!;
                  if (x1 <= x0) continue;
                  const y = topYs[j]!;
                  ctx.moveTo(x0, y);
                  ctx.lineTo(x1, y);
                }
                ctx.globalAlpha = 0.9;
                ctx.strokeStyle = theme.surface;
                ctx.lineWidth = 0.8;
                ctx.stroke();
              }
            }
          }
          if (total) {
            ctx.globalAlpha = 1;
            ctx.strokeStyle = total.color ?? theme.text;
            ctx.lineWidth = 1.6;
            ctx.lineJoin = 'round';
            let started = false;
            let prevY = 0;
            let lastValidK = -1;
            for (let k = 0; k < len; k++) {
              const tv = total.values[run.start + k];
              if (tv == null || !Number.isFinite(tv)) {
                if (started) {
                  const breakX = xs[k]!;
                  if (breakX > xs[lastValidK]!) ctx.lineTo(breakX, prevY);
                  ctx.stroke();
                  started = false;
                }
                continue;
              }
              const x = xs[k]!;
              const y = layout.toY(tv);
              if (!started) { ctx.beginPath(); ctx.moveTo(x, y); started = true; }
              else { ctx.lineTo(x, prevY); ctx.lineTo(x, y); }
              prevY = y;
              lastValidK = k;
            }
            if (started) {
              if (endX > xs[lastValidK]!) {
                ctx.lineTo(endX, prevY);
              }
              ctx.stroke();
            }
          }
        }
        if (layout.yDomain[0] < 0) {
          const zeroY = Math.round(layout.toY(0)) + 0.5;
          ctx.globalAlpha = 1;
          ctx.strokeStyle = tokens.hairlineStrong;
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(layout.left, zeroY);
          ctx.lineTo(layout.left + layout.plotWidth, zeroY);
          ctx.stroke();
        }
        ctx.restore();
      }
    } else if (count > 0 && bands.length > 0) {
      const segments: [number, number][] = [];
      let segStart = -1;
      for (let i = 0; i < count; i++) {
        let valid = true;
        for (let b = 0; b < bands.length; b++) {
          const v = bands[b]!.values[i];
          if (v === null || v === undefined || !Number.isFinite(v)) { valid = false; break; }
        }
        if (valid) {
          if (segStart < 0) segStart = i;
        } else if (segStart >= 0) {
          segments.push([segStart, i - 1]);
          segStart = -1;
        }
      }
      if (segStart >= 0) segments.push([segStart, count - 1]);

      if (segments.length > 0) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, layout.plotHeight + 4);
        ctx.clip();
        ensureStackScratch(count);
        const xs = stackXsBuf;
        for (let i = 0; i < count; i++) xs[i] = layout.toX(hours[i]!);
        const zeroY = layout.toY(0);
        for (const [start, end] of segments) {
          const len = end - start + 1;
          const acc = stackPosAccBuf;
          let botYs = stackBotYsBuf;
          let topYs = stackTopYsBuf;
          acc.fill(0, 0, len);
          botYs.fill(zeroY, 0, len);
          for (let bandIdx = 0; bandIdx < bands.length; bandIdx++) {
            const band = bands[bandIdx]!;
            const values = band.values;
            for (let k = 0; k < len; k++) {
              const nextVal = acc[k]! + (values[start + k] ?? 0);
              acc[k] = nextVal;
              topYs[k] = layout.toY(nextVal);
            }
            ctx.beginPath();
            if (len === 1) {
              const x0 = xs[start]!;
              const x1 = Math.min(layout.playheadX, x0 + 1);
              const yTop = topYs[0]!;
              const yBot = botYs[0]!;
              ctx.moveTo(x0, yTop);
              ctx.lineTo(x1, yTop);
              ctx.lineTo(x1, yBot);
              ctx.lineTo(x0, yBot);
            } else {
              ctx.moveTo(xs[start]!, topYs[0]!);
              for (let k = 1; k < len; k++) ctx.lineTo(xs[start + k]!, topYs[k]!);
              for (let k = len - 1; k >= 0; k--) ctx.lineTo(xs[start + k]!, botYs[k]!);
            }
            ctx.closePath();
            ctx.fillStyle = fills?.[bandIdx] ?? band.color;
            ctx.globalAlpha = (band.opacity ?? 0.78) * (hoveredSeriesId && hoveredSeriesId !== band.id ? 0.35 : 1);
            ctx.fill();
            if (bandIdx < bands.length - 1 && len > 1) {
              ctx.beginPath();
              ctx.moveTo(xs[start]!, topYs[0]!);
              for (let k = 1; k < len; k++) ctx.lineTo(xs[start + k]!, topYs[k]!);
              ctx.globalAlpha = 0.9;
              ctx.strokeStyle = theme.surface;
              ctx.lineWidth = 0.8;
              ctx.stroke();
            }
            const tmp = botYs;
            botYs = topYs;
            topYs = tmp;
          }
        }
        if (total) {
          ctx.globalAlpha = 1;
          ctx.strokeStyle = total.color ?? theme.text;
          ctx.lineWidth = 1.6;
          ctx.lineJoin = 'round';
          let open = false;
          for (let i = 0; i < count; i++) {
            const tv = total.values[i];
            if (tv == null || !Number.isFinite(tv)) {
              if (open) { ctx.stroke(); open = false; }
              continue;
            }
            const x = xs[i]!;
            const y = layout.toY(tv);
            if (!open) { ctx.beginPath(); ctx.moveTo(x, y); open = true; }
            else ctx.lineTo(x, y);
          }
          if (open) ctx.stroke();
        }
        ctx.restore();
      }
    }
    drawPlayheadLine(ctx, layout.playheadX, layout.top - 2, layout.baseY + layout.markerLaneHeight, theme);
    if (layout.selectedX !== null) {
      drawSelectedHourLine(ctx, layout.selectedX, layout.top - 2, layout.baseY + layout.markerLaneHeight, theme);
    }
  } else {
    const zeroY = layout.toY(0);
    const clipZeroY = layout.yDomain[0] < 0 ? zeroY : layout.baseY + 4;
    const endDots: { readonly x: number; readonly y: number; readonly color: string; readonly opacity: number }[] = [];

    if (!timeChartCacheState.enabled) {
      for (let idx = model.series.length - 1; idx >= 0; idx--) {
        const s = model.series[idx]!;
        const { subpaths, end } = buildLineSubpaths(s, layout, model.hour);
        const paths = subpaths.map(pathOfPoints);
        const baseOpacity = s.opacity ?? (s.role === 'tail' ? 0.22 : s.role === 'ceiling' ? 0.55 : 1);
        const opacity = hoveredSeriesId && hoveredSeriesId !== s.id ? Math.min(baseOpacity, 0.33) : baseOpacity;
        const lineWidth = s.width ?? (s.role === 'tail' ? 1 : s.role === 'ceiling' ? 1.4 : model.series.length > 2 ? 1.6 : 1.75);

        ctx.save();
        ctx.beginPath();
        ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, layout.plotHeight + 4);
        ctx.clip();

        if (s.area && paths.length > 0) {
          const baselineY = layout.toY(Math.max(layout.yDomain[0], 0));
          const areaRgb = parseColor(s.color);
          const topStop = areaRgb ? rgbaRgb(areaRgb, 0.26 * opacity) : '';
          const botStop = areaRgb ? rgbaRgb(areaRgb, 0.02 * opacity) : s.color;
          for (const line of paths) {
            if (line.count < 2) continue;
            ctx.save();
            ctx.beginPath();
            ctx.rect(layout.left, layout.top - 2, layout.plotWidth + 1, Math.max(0, clipZeroY - (layout.top - 2)));
            ctx.clip();
            const area = new Path2D(line.path);
            area.lineTo(line.last[0], baselineY);
            area.lineTo(line.first[0], baselineY);
            area.closePath();
            const grad = ctx.createLinearGradient(0, layout.top, 0, baselineY);
            if (topStop) {
              grad.addColorStop(0, topStop);
              grad.addColorStop(1, botStop);
              ctx.globalAlpha = 1;
            } else {
              grad.addColorStop(0, s.color);
              grad.addColorStop(1, s.color);
              ctx.globalAlpha = 0.22 * opacity;
            }
            ctx.fillStyle = grad;
            ctx.fill(area);
            ctx.restore();

            if (s.negRed && layout.yDomain[0] < 0) {
              ctx.save();
              ctx.beginPath();
              ctx.rect(layout.left, clipZeroY, layout.plotWidth + 1, Math.max(0, layout.baseY + 4 - clipZeroY));
              ctx.clip();
              const below = new Path2D(line.path);
              below.lineTo(line.last[0], zeroY);
              below.lineTo(line.first[0], zeroY);
              below.closePath();
              ctx.globalAlpha = 0.16 * opacity;
              ctx.fillStyle = tokens.shortFill;
              ctx.fill(below);
              ctx.restore();
            }
          }
        }

        const stretches = colourStretches(s, layout, model.hour);
        for (const line of paths) {
          const strokeSegment = (strokeColor: string, clip: { readonly x0: number; readonly x1: number; readonly y0: number; readonly y1: number } | null) => {
            if (clip && (clip.x1 <= clip.x0 || clip.y1 <= clip.y0)) return;
            ctx.save();
            if (clip) {
              ctx.beginPath();
              ctx.rect(clip.x0, clip.y0, clip.x1 - clip.x0, clip.y1 - clip.y0);
              ctx.clip();
            }
            ctx.globalAlpha = opacity;
            ctx.strokeStyle = strokeColor;
            ctx.lineWidth = lineWidth;
            ctx.lineJoin = 'round';
            ctx.lineCap = 'round';
            const dash = s.dash?.length ? s.dash : s.role === 'ceiling' ? CEILING_DASH : null;
            if (dash) ctx.setLineDash([...dash]);
            ctx.stroke(line.path);
            ctx.restore();
          };
          const signed = s.negRed === true && layout.yDomain[0] < 0;
          const above = signed ? clipZeroY : layout.top + layout.plotHeight + 2;
          const columns = stretches ?? [{ color: s.color, x0: layout.left, x1: layout.left + layout.plotWidth + 1 }];
          for (const stretch of columns) {
            strokeSegment(stretch.color, stretches || signed ? { x0: stretch.x0, x1: stretch.x1, y0: layout.top - 2, y1: above } : null);
          }
          if (signed) strokeSegment(tokens.shortage, { x0: layout.left, x1: layout.left + layout.plotWidth + 1, y0: clipZeroY, y1: layout.baseY + 4 });
        }

        if (s.missingSpans && s.missingSpans.length > 0) {
          for (const span of s.missingSpans) {
            const sa = Math.max(layout.xDomain[0], span.firstHour);
            const sb = Math.min(layout.xDomain[1], model.hour, span.lastHour);
            if (sb <= sa) continue;
            const sx0 = layout.toX(sa);
            const sw = Math.max(2, layout.toX(sb) - sx0);
            drawSeriesMissingStrip(ctx, sx0, layout.baseY - 6, sw, s.color, theme, 5);
          }
        }
        ctx.restore();

        if (end && s.endLabel) {
          const isNeg = s.negRed && s.headValue != null && s.headValue < 0;
          endDots.push({ x: end[0], y: end[1], color: isNeg ? tokens.shortage : s.color, opacity });
        }
      }
    } else if (model.series.length > 0) {
      const seriesCount = model.series.length;
      const defaultPrimaryWidth = seriesCount > 2 ? 1.6 : 1.75;
      const signedGlobal = layout.yDomain[0] < 0;
      const plotClipTop = layout.top - 2;
      const plotClipWidth = layout.plotWidth + 1;
      const plotClipBottom = layout.top + layout.plotHeight + 2;
      const directEnd = { hasEnd: false, x: 0, y: 0 };
      const incomingAlpha = ctx.globalAlpha;
      const incomingLineCap = ctx.lineCap;
      const incomingLineJoin = ctx.lineJoin;
      const incomingLineWidth = ctx.lineWidth;
      const incomingStrokeStyle = ctx.strokeStyle;
      const incomingDash: readonly number[] = typeof ctx.getLineDash === 'function' ? ctx.getLineDash() : [];
      const endDotColorOf = (s: TimeChartSeriesModel) =>
        s.negRed && s.headValue != null && s.headValue < 0 ? tokens.shortage : s.color;

      ctx.save();
      ctx.beginPath();
      ctx.rect(layout.left, plotClipTop, plotClipWidth, layout.plotHeight + 4);
      ctx.clip();
      ctx.lineJoin = 'round';
      ctx.lineCap = 'round';

      let curAlpha = NaN;
      let curStroke = '';
      let curWidth = NaN;
      let curDash: readonly number[] = incomingDash;
      const applyStroke = (alpha: number, color: string, width: number, dash: readonly number[] | null) => {
        if (alpha !== curAlpha) { ctx.globalAlpha = alpha; curAlpha = alpha; }
        if (color !== curStroke) { ctx.strokeStyle = color; curStroke = color; }
        if (width !== curWidth) { ctx.lineWidth = width; curWidth = width; }
        const targetDash = dash ?? incomingDash;
        if (!sameDash(targetDash, curDash)) {
          ctx.setLineDash([...targetDash]);
          curDash = targetDash;
        }
      };

      for (let idx = seriesCount - 1; idx >= 0; idx--) {
        const s = model.series[idx]!;
        const baseOpacity = s.opacity ?? (s.role === 'tail' ? 0.22 : s.role === 'ceiling' ? 0.55 : 1);
        const opacity = hoveredSeriesId && hoveredSeriesId !== s.id ? Math.min(baseOpacity, 0.33) : baseOpacity;
        const lineWidth = s.width ?? (s.role === 'tail' ? 1 : s.role === 'ceiling' ? 1.4 : defaultPrimaryWidth);
        const dash = s.dash?.length ? s.dash : s.role === 'ceiling' ? CEILING_DASH : null;
        const signed = s.negRed === true && signedGlobal;
        const stretches = colourStretches(s, layout, model.hour);

        if (!s.area && !stretches && !signed) {
          applyStroke(opacity, s.color, lineWidth, dash);
          strokeLineDirect(ctx, s, layout, model.hour, directEnd);
          if (directEnd.hasEnd && s.endLabel) {
            endDots.push({ x: directEnd.x, y: directEnd.y, color: endDotColorOf(s), opacity });
          }
        } else {
          const buf = fillLineBuffer(s, layout, model.hour, scratchBuffer);
          const subCount = buf.subpathCount;

          if (s.area && subCount > 0) {
            const baselineY = layout.toY(Math.max(layout.yDomain[0], 0));
            const areaRgb = parseColor(s.color);
            const topStop = areaRgb ? rgbaRgb(areaRgb, 0.26 * opacity) : '';
            const botStop = areaRgb ? rgbaRgb(areaRgb, 0.02 * opacity) : s.color;
            const aboveHeight = Math.max(0, clipZeroY - plotClipTop);
            const belowHeight = Math.max(0, layout.baseY + 4 - clipZeroY);
            const coords = buf.coords;
            for (let si = 0; si < subCount; si++) {
              const count = buf.counts[si]!;
              if (count < 2) continue;
              const start = buf.starts[si]!;
              const firstX = coords[start]!;
              const lastX = coords[start + (count - 1) * 2]!;
              ctx.save();
              ctx.beginPath();
              ctx.rect(layout.left, plotClipTop, plotClipWidth, aboveHeight);
              ctx.clip();
              emitBufferSubpath(ctx, buf, si);
              ctx.lineTo(lastX, baselineY);
              ctx.lineTo(firstX, baselineY);
              ctx.closePath();
              const grad = ctx.createLinearGradient(0, layout.top, 0, baselineY);
              if (topStop) {
                grad.addColorStop(0, topStop);
                grad.addColorStop(1, botStop);
                ctx.globalAlpha = 1;
              } else {
                grad.addColorStop(0, s.color);
                grad.addColorStop(1, s.color);
                ctx.globalAlpha = 0.22 * opacity;
              }
              ctx.fillStyle = grad;
              ctx.fill();
              ctx.restore();

              if (signed) {
                ctx.save();
                ctx.beginPath();
                ctx.rect(layout.left, clipZeroY, plotClipWidth, belowHeight);
                ctx.clip();
                emitBufferSubpath(ctx, buf, si);
                ctx.lineTo(lastX, zeroY);
                ctx.lineTo(firstX, zeroY);
                ctx.closePath();
                ctx.globalAlpha = 0.16 * opacity;
                ctx.fillStyle = tokens.shortFill;
                ctx.fill();
                ctx.restore();
              }
            }
          }

          if (!stretches && !signed) {
            applyStroke(opacity, s.color, lineWidth, dash);
            for (let si = 0; si < subCount; si++) {
              emitBufferSubpath(ctx, buf, si);
              ctx.stroke();
            }
          } else {
            const above = signed ? clipZeroY : plotClipBottom;
            const columns = stretches ?? [{ color: s.color, x0: layout.left, x1: layout.left + plotClipWidth }];
            const targetDash = dash ?? incomingDash;
            const needsDashOverride = !sameDash(targetDash, curDash);
            for (let si = 0; si < subCount; si++) {
              for (let ci = 0; ci < columns.length; ci++) {
                const stretch = columns[ci]!;
                if (stretch.x1 <= stretch.x0 || above <= plotClipTop) continue;
                ctx.save();
                ctx.beginPath();
                ctx.rect(stretch.x0, plotClipTop, stretch.x1 - stretch.x0, above - plotClipTop);
                ctx.clip();
                ctx.globalAlpha = opacity;
                ctx.strokeStyle = stretch.color;
                ctx.lineWidth = lineWidth;
                if (needsDashOverride) ctx.setLineDash([...targetDash]);
                emitBufferSubpath(ctx, buf, si);
                ctx.stroke();
                ctx.restore();
              }
              if (signed && layout.baseY + 4 > clipZeroY) {
                ctx.save();
                ctx.beginPath();
                ctx.rect(layout.left, clipZeroY, plotClipWidth, layout.baseY + 4 - clipZeroY);
                ctx.clip();
                ctx.globalAlpha = opacity;
                ctx.strokeStyle = tokens.shortage;
                ctx.lineWidth = lineWidth;
                if (needsDashOverride) ctx.setLineDash([...targetDash]);
                emitBufferSubpath(ctx, buf, si);
                ctx.stroke();
                ctx.restore();
              }
            }
          }

          if (buf.hasEnd && s.endLabel) {
            endDots.push({ x: buf.endX, y: buf.endY, color: endDotColorOf(s), opacity });
          }
        }

        if (s.missingSpans && s.missingSpans.length > 0) {
          ctx.save();
          ctx.globalAlpha = incomingAlpha;
          ctx.lineCap = incomingLineCap;
          ctx.lineJoin = incomingLineJoin;
          ctx.lineWidth = incomingLineWidth;
          ctx.strokeStyle = incomingStrokeStyle;
          if (!sameDash(incomingDash, curDash)) ctx.setLineDash([...incomingDash]);
          for (const span of s.missingSpans) {
            const sa = Math.max(layout.xDomain[0], span.firstHour);
            const sb = Math.min(layout.xDomain[1], model.hour, span.lastHour);
            if (sb <= sa) continue;
            const sx0 = layout.toX(sa);
            const sw = Math.max(2, layout.toX(sb) - sx0);
            drawSeriesMissingStrip(ctx, sx0, layout.baseY - 6, sw, s.color, theme, 5);
          }
          ctx.restore();
        }
      }

      ctx.restore();
    }

    drawPlayheadLine(ctx, layout.playheadX, layout.top - 2, layout.baseY + layout.markerLaneHeight, theme);
    if (layout.selectedX !== null) {
      drawSelectedHourLine(ctx, layout.selectedX, layout.top - 2, layout.baseY + layout.markerLaneHeight, theme);
    }

    for (const dot of endDots) {
      ctx.save();
      ctx.globalAlpha = dot.opacity;
      ctx.beginPath();
      ctx.arc(dot.x, dot.y, 3.6, 0, Math.PI * 2);
      ctx.fillStyle = dot.color;
      ctx.fill();
      ctx.strokeStyle = theme.surface;
      ctx.lineWidth = 2;
      ctx.stroke();
      ctx.restore();
    }
  }

  for (const label of layout.endLabels) {
    ctx.save();
    if (label.faded) ctx.globalAlpha = 0.35;
    if (label.hasLeader) {
      ctx.strokeStyle = tokens.playhead;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(layout.playheadX + 4, label.anchorY);
      ctx.lineTo(label.x - 3, label.y);
      ctx.stroke();
    }
    let cursorX = label.x;
    if (label.colorBar) {
      ctx.fillStyle = label.colorBar;
      ctx.beginPath();
      ctx.roundRect(cursorX, label.y - 6, 3, 12, 1);
      ctx.fill();
      cursorX += 9;
    }
    if (label.flag && label.flag.art.status !== 'noArtExpected') {
      drawFlagSlot(ctx, cursorX, label.y - 6, toArtSlotState(resolveCachedArt(label.flag.art, artCache), label.flag.tag ?? label.flag.label), theme, { width: 18, height: 12 });
      cursorX += 24;
    } else if (label.icon && label.icon.art.status !== 'noArtExpected') {
      drawArtSlot(ctx, cursorX, label.y - 7.5, 16, 15, toArtSlotState(resolveCachedArt(label.icon.art, artCache), label.icon.label), theme, { radius: 0 });
      cursorX += 22;
    }
    if (label.name) {
      ctx.font = labelNameFont;
      ctx.fillStyle = label.valueMuted ? theme.muted : textSecondary;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(label.name, cursorX, label.y);
      cursorX += (label.nameWidth !== undefined && label.nameFont === labelNameFont ? label.nameWidth : measureCanvasText(ctx, label.name, labelNameFont)) + 6;
    }
    ctx.font = labelValueFont;
    ctx.fillStyle = label.valueMuted ? theme.muted : label.negative ? tokens.shortage : (label.valueColor ?? theme.text);
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(label.value, cursorX, label.y);
    ctx.restore();
  }

  if (model.overlays && layout.plotHeight >= 72 && layout.plotWidth >= 240) {
    for (const ov of model.overlays) {
      ctx.save();
      ctx.font = overlayFont;
      const tw = measureCanvasText(ctx, ov.text, overlayFont);
      const ox = layout.left + layout.plotWidth - 8 - tw;
      const oy = ov.position === 'top-right' ? layout.top + 12 : layout.baseY - 12;
      const arrowW = ov.arrow ? 12 : 0;
      const padX = 5;
      const plateLeft = Math.max(layout.left + 2, ox - arrowW - padX);
      const plateRight = Math.min(layout.left + layout.plotWidth - 2, ox + tw + padX);
      if (plateRight > plateLeft) {
        ctx.fillStyle = theme.surface;
        ctx.beginPath();
        ctx.roundRect(plateLeft, oy - 8, plateRight - plateLeft, 16, 3);
        ctx.fill();
      }
      if (ov.arrow) {
        ctx.fillStyle = ov.color ?? textSecondary;
        ctx.beginPath();
        const ax = ox - 12;
        if (ov.arrow === 'right') {
          ctx.moveTo(ax, oy - 3.5);
          ctx.lineTo(ax + 6, oy);
          ctx.lineTo(ax, oy + 3.5);
        } else {
          ctx.moveTo(ax + 6, oy - 3.5);
          ctx.lineTo(ax, oy);
          ctx.lineTo(ax + 6, oy + 3.5);
        }
        ctx.closePath();
        ctx.fill();
      }
      ctx.fillStyle = textSecondary;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.fillText(ov.text, ox, oy);
      ctx.restore();
    }
  }

  for (const m of layout.markers) {
    if (m.kind === 'flag' && m.marker.flag) {
      drawFlagSlot(ctx, m.x - 8, m.y - 5, toArtSlotState(resolveCachedArt(m.marker.flag.art, artCache), m.marker.flag.tag ?? m.marker.flag.label), theme, { width: 16, height: 10 });
    } else if (m.kind === 'icon' && m.marker.icon) {
      drawArtSlot(ctx, m.x - 9, m.y - 9, 18, 18, toArtSlotState(resolveCachedArt(m.marker.icon.art, artCache), m.marker.icon.label), theme, { radius: 0 });
    } else {
      ctx.save();
      ctx.strokeStyle = m.marker.tickColor ?? tokens.markerTick;
      ctx.lineWidth = 1.6;
      ctx.lineCap = 'round';
      ctx.beginPath();
      ctx.moveTo(m.x, m.y - m.size / 2);
      ctx.lineTo(m.x, m.y + m.size / 2);
      ctx.stroke();
      ctx.restore();
    }
  }

  ctx.save();
  ctx.strokeStyle = tokens.zeroLine;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(layout.left, layout.axisY + 0.5);
  ctx.lineTo(layout.left + layout.plotWidth, layout.axisY + 0.5);
  ctx.stroke();
  ctx.restore();

  for (const g of layout.gaps) {
    drawSeriesMissingStrip(ctx, g.x0, layout.axisY - 2, g.x1 - g.x0, theme.hatch, theme, 5);
  }
}

export interface DrawTimeChartOptions {
  readonly cacheBackdrop?: boolean;
}

export function drawTimeChart(
  ctx: Canvas2D,
  model: TimeChartModel,
  layout: TimeChartLayout,
  theme: FamilyTheme,
  inspectKey: string | null | undefined,
  dpr: number,
  legendFit?: LegendFit | null,
  artCache?: LaneArtCache,
  options?: DrawTimeChartOptions,
): void {
  const tokens = resolveKitTokens(theme);
  const parsedInspect = parseChartInspect(inspectKey);
  const hoverHour = parsedInspect?.kind === 'plot' || parsedInspect?.kind === 'line'
    ? parsedInspect.hour
    : parsedInspect?.kind === 'marker'
      ? layout.markers[parsedInspect.index]?.marker.hour ?? null
      : null;
  const hoveredSeriesId = parsedInspect?.kind === 'label' || parsedInspect?.kind === 'line'
    ? parsedInspect.seriesId
    : null;

  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, layout.width, layout.height);
  if (layout.rows) {
    backdropCache.delete(tcCacheKey(ctx));
    drawChartRows(ctx, model, layout.rows, theme, parseRowsInspect(inspectKey), artCache);
    return;
  }

  let usedBackdrop = false;
  let trackedBackdrop = false;
  if (
    options?.cacheBackdrop === true
    && timeChartCacheState.enabled
    && model.mode !== 'columns'
    && typeof OffscreenCanvas !== 'undefined'
    && typeof (ctx as { readonly drawImage?: unknown }).drawImage === 'function'
  ) {
    const colorSpace = backdropColorSpaceOf(ctx);
    const slotMeta = ctx as unknown as { readonly __bpSlotBw?: number; readonly __bpSlotBh?: number };
    const canvas = (ctx as { readonly canvas?: { readonly width?: number; readonly height?: number } }).canvas;
    const bw = typeof slotMeta.__bpSlotBw === 'number' ? slotMeta.__bpSlotBw : (canvas && typeof canvas.width === 'number' ? canvas.width : 0);
    const bh = typeof slotMeta.__bpSlotBh === 'number' ? slotMeta.__bpSlotBh : (canvas && typeof canvas.height === 'number' ? canvas.height : 0);
    if (
      colorSpace !== null
      && bw > 0
      && bh > 0
      && Math.abs(bw - layout.width * dpr) <= 1
      && Math.abs(bh - layout.height * dpr) <= 1
    ) {
      trackedBackdrop = true;
      const themeKey = backdropThemeKey(theme);
      const textRev = textWidthRevision();
      const normLegendFit = legendFit ?? null;
      const cacheKey = tcCacheKey(ctx);
      let entry = backdropCache.get(cacheKey);
      const sameKey = entry !== undefined
        && entry.bw === bw
        && entry.bh === bh
        && Object.is(entry.dpr, dpr)
        && entry.colorSpace === colorSpace
        && entry.model === model
        && entry.layout === layout
        && entry.hoveredSeriesId === hoveredSeriesId
        && entry.legendFit === normLegendFit
        && entry.themeKey === themeKey
        && entry.textRev === textRev
        && sameBackdropArtSnapshot(entry.artSnapshot, model, layout, normLegendFit, artCache);
      if (sameKey && entry) {
        if (!entry.canvas || !entry.ctx) {
          const off = new OffscreenCanvas(bw, bh);
          const offCtx = off.getContext('2d', colorSpace !== 'srgb' ? { colorSpace } : undefined);
          const offSpace = offCtx ? (backdropColorSpaceOf(offCtx) ?? null) : null;
          if (offCtx && offSpace === colorSpace) {
            snapCanvasText(offCtx);
            entry.canvas = off;
            entry.ctx = offCtx;
            offCtx.save();
            offCtx.setTransform(1, 0, 0, 1, 0, 0);
            offCtx.clearRect(0, 0, bw, bh);
            offCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
            drawTimeChartBackdrop(offCtx, model, layout, theme, tokens, null, hoveredSeriesId, legendFit, artCache);
            offCtx.restore();
          }
        }
        if (entry.canvas && entry.ctx) {
          ctx.setTransform(1, 0, 0, 1, 0, 0);
          ctx.drawImage(entry.canvas, 0, 0);
          ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
          usedBackdrop = true;
        }
      } else {
        const artSnapshot = collectBackdropArtSnapshot(model, layout, normLegendFit, artCache);
        if (!entry) {
          entry = {
            canvas: null,
            ctx: null,
            bw,
            bh,
            dpr,
            colorSpace,
            model,
            layout,
            hoveredSeriesId,
            legendFit: normLegendFit,
            themeKey,
            textRev,
            artSnapshot,
          };
          backdropCache.set(cacheKey, entry);
        } else {
          entry.canvas = null;
          entry.ctx = null;
          entry.bw = bw;
          entry.bh = bh;
          entry.dpr = dpr;
          entry.colorSpace = colorSpace;
          entry.model = model;
          entry.layout = layout;
          entry.hoveredSeriesId = hoveredSeriesId;
          entry.legendFit = normLegendFit;
          entry.themeKey = themeKey;
          entry.textRev = textRev;
          entry.artSnapshot = artSnapshot;
        }
      }
    }
  }
  if (!usedBackdrop) {
    if (!trackedBackdrop) backdropCache.delete(tcCacheKey(ctx));
    drawTimeChartBackdrop(ctx, model, layout, theme, tokens, parsedInspect, hoveredSeriesId, legendFit, artCache);
  }

  const smallFont = `${theme.fontSize.small}px ${theme.font}`;
  const textSecondary = tokens.textSecondary;
  const hoverInPlot = hoverHour != null && model.mode !== 'columns' && hoverHour >= layout.xDomain[0] && hoverHour <= model.hour;

  if (model.mode === 'columns') {
    for (const col of layout.columns) {
      const cx = col.x + col.width / 2;
      if (col.kind === 'fold') {
        ctx.save();
        ctx.fillStyle = theme.surface;
        ctx.fillRect(cx - 5, layout.axisY - 4, 10, 9);
        ctx.strokeStyle = tokens.zeroLine;
        ctx.lineWidth = 1.3;
        ctx.beginPath();
        ctx.moveTo(cx - 5, layout.axisY + 4);
        ctx.lineTo(cx - 1, layout.axisY - 4);
        ctx.moveTo(cx + 1, layout.axisY + 4);
        ctx.lineTo(cx + 5, layout.axisY - 4);
        ctx.stroke();
        ctx.restore();
      } else if (col.month?.status === 'recorded' && col.month.missingParts && col.month.missingParts.length > 0) {
        drawSeriesMissingStrip(ctx, col.barX, layout.axisY + 1, col.barWidth, theme.hatch, theme, 4);
      }
      if (col.label && Math.abs(cx - layout.dateTabX) > layout.dateTabWidth / 2 + 10) {
        ctx.save();
        ctx.font = smallFont;
        ctx.fillStyle = col.majorLabel ? textSecondary : theme.muted;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'top';
        ctx.fillText(col.label, cx, layout.axisY + 6);
        ctx.restore();
      }
    }
    drawDateTab(ctx, layout.dateTabText, layout.dateTabX, layout.axisY + 3, theme, 'playhead', {
      left: layout.left,
      right: layout.width - 6,
    });
  } else {
    if (hoverInPlot) {
      const xh = layout.toX(hoverHour);
      drawCrosshairLine(ctx, xh, layout.top - 2, layout.axisY, theme);
      if (model.mode === 'stack' && model.stack?.total && model.stack.hours.length > 0) {
        let at = -1;
        for (let i = 0; i < model.stack.hours.length; i++) {
          if (model.stack.hours[i]! <= hoverHour) at = i;
          else break;
        }
        const tv = at >= 0 ? model.stack.total.values[at] : null;
        if (tv != null && Number.isFinite(tv)) {
          ctx.save();
          ctx.beginPath();
          ctx.arc(xh, layout.toY(tv), 3.4, 0, Math.PI * 2);
          ctx.fillStyle = model.stack.total.color ?? theme.text;
          ctx.fill();
          ctx.strokeStyle = theme.surface;
          ctx.lineWidth = 1.8;
          ctx.stroke();
          ctx.restore();
        }
      }
      for (const s of model.series) {
        const sampled = sampleSeriesValueAt(s, hoverHour, model.hour);
        if (!sampled) continue;
        const dotColor = s.negRed && sampled.value < 0 ? tokens.shortage : seriesLookAt(s, hoverHour).color;
        ctx.save();
        ctx.beginPath();
        ctx.arc(xh, layout.toY(sampled.value), 3.4, 0, Math.PI * 2);
        ctx.fillStyle = dotColor;
        ctx.fill();
        ctx.strokeStyle = theme.surface;
        ctx.lineWidth = 1.8;
        ctx.stroke();
        ctx.restore();
      }
    }
    drawTimeAxis(ctx, layout.rawAxisTicks, {
      plotLeft: layout.left,
      plotRight: layout.left + layout.plotWidth,
      startHour: layout.xDomain[0],
      endHour: layout.xDomain[1],
      toX: layout.toX,
      axisTop: layout.axisY,
      playhead: model.hour,
      playheadLabel: layout.dateTabText,
      hover: hoverInPlot ? hoverHour : null,
      hoverLabel: hoverInPlot ? formatDateOnly(hoverHour) : undefined,
      playheadLineTop: null,
      hairline: 'none',
      tickColor: tokens.zeroLine,
      firstLabelOnly: layout.small,
      tabBounds: { left: layout.left, right: layout.width - 6 },
      tabY: layout.axisY + 3,
      labelY: layout.axisY + 12,
      theme,
    });
  }
}
