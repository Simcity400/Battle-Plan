import type { PlotFrame } from './frame.ts';
import type { Canvas2D } from './textWidth.ts';
import type { ChartTokens } from './tokens.ts';
import { drawLine, tracePath } from './draw.ts';
import { extentOf, runAt } from './series.ts';
import type { PathCommand, Point, Run } from './series.ts';
import { addStackAmount, stackEdges } from './stack.ts';
import type { CompactStackModel } from './stack';

export interface ChartBucket {
  firstHour: number;
  lastHour: number;
  firstValue: number;
  lastValue: number;
  lastValueHour: number;
  minHour: number;
  minValue: number;
  maxHour: number;
  maxValue: number;
  firstBreak: boolean;
  hasBreak: boolean;
}

export interface ChartSummary {
  from: number;
  to: number;
  width: number;
  buckets: readonly (ChartBucket | null)[];
  runs?: readonly Run[];
  approximate?: readonly { firstHour: number; lastHour: number }[];
  tiles?: TiledSummary;
}

export interface TiledSummary {
  binHours: number;
  valueAt(hour: number): number | null;
}

export const exactRuns = (summary: ChartSummary): readonly Run[] | null => summary.runs ?? null;

function approximateUnion(spans: ChartSummary['approximate']) {
  const union: NonNullable<ChartSummary['approximate']>[number][] = [];
  for (const span of [...(spans ?? [])].sort((a, b) => a.firstHour - b.firstHour)) {
    const last = union.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else union.push({ ...span });
  }
  return union;
}

export function extentOfSummary(summary: ChartSummary, from = summary.from, to = summary.to): [number, number] | null {
  const runs = exactRuns(summary);
  if (runs) return extentOf(runs, from, to);
  const first = Math.floor(from);
  let low = Infinity, high = -Infinity;
  if (summary.tiles) {
    const { binHours } = summary.tiles, cut = Math.floor(to / binHours) * binHours;
    const start = Math.max(0, Math.floor(first / binHours) - Math.floor(summary.from / binHours));
    for (let at = start; at < summary.buckets.length; at++) {
      const bucket = summary.buckets[at];
      if (!bucket) continue;
      if (bucket.lastHour >= cut) break;
      low = Math.min(low, bucket.minValue); high = Math.max(high, bucket.maxValue);
    }
    const head = summary.tiles.valueAt(to);
    if (head !== null && Number.isFinite(head)) { low = Math.min(low, head); high = Math.max(high, head); }
    return low <= high ? [low, high] : null;
  }
  for (const bucket of summary.buckets) {
    if (bucket && bucket.lastHour >= first && bucket.firstHour <= to) { low = Math.min(low, bucket.minValue); high = Math.max(high, bucket.maxValue); }
  }
  return low <= high ? [low, high] : null;
}

export function summaryExtent(summary: ChartSummary, x: readonly [number, number], until: number | null): [number, number] | null {
  const end = Math.min(x[1], until ?? x[1]);
  if (summary.tiles) return summary.from > x[1] || summary.to < x[0] ? null : extentOfSummary(summary, x[0], end);
  return summary.to > end || summary.to < x[0] ? null : extentOfSummary(summary);
}

export const summaryFits = (frame: Pick<PlotFrame, 'x' | 'plotWidth'>, summary: { from: number; to: number; width: number; tiles?: unknown }, until: number | null) => summary.width === Math.ceil(frame.plotWidth)
  && (summary.tiles !== undefined || summary.to <= Math.min(frame.x[1], until ?? frame.x[1])) && summary.to >= frame.x[0] && summary.from <= frame.x[1];

export interface BucketCorner {
  hour: number;
  value: number;
  order: number;
}

export interface BucketStreamReader {
  readonly count: number;
  present(index: number): boolean;
  firstHour(index: number): number;
  lastHour(index: number): number;
  firstValue(index: number): number;
  lastValue(index: number): number;
  lastValueHour(index: number): number;
  minHour(index: number): number;
  minValue(index: number): number;
  maxHour(index: number): number;
  maxValue(index: number): number;
  firstBreak(index: number): boolean;
  hasBreak(index: number): boolean;
}

export function emitBucketCorners(
  firstHour: number,
  minHour: number,
  maxHour: number,
  lastValueHour: number,
  firstValue: number,
  minValue: number,
  maxValue: number,
  lastValue: number,
  out?: BucketCorner[],
): BucketCorner[] {
  const corners = out && out.length === 4
    ? out
    : [
        { hour: 0, value: 0, order: 0 },
        { hour: 0, value: 0, order: 1 },
        { hour: 0, value: 0, order: 2 },
        { hour: 0, value: 0, order: 3 },
      ];
  corners[0].hour = firstHour; corners[0].value = firstValue; corners[0].order = 0;
  corners[1].hour = minHour; corners[1].value = minValue; corners[1].order = 1;
  corners[2].hour = maxHour; corners[2].value = maxValue; corners[2].order = 2;
  corners[3].hour = lastValueHour; corners[3].value = lastValue; corners[3].order = 3;
  corners.sort((a, b) => a.hour - b.hour || a.order - b.order);
  return corners;
}

export function bucketsStream(buckets: readonly (ChartBucket | null)[]): BucketStreamReader {
  return {
    count: buckets.length,
    present: i => buckets[i] != null,
    firstHour: i => buckets[i]!.firstHour,
    lastHour: i => buckets[i]!.lastHour,
    firstValue: i => buckets[i]!.firstValue,
    lastValue: i => buckets[i]!.lastValue,
    lastValueHour: i => buckets[i]!.lastValueHour,
    minHour: i => buckets[i]!.minHour,
    minValue: i => buckets[i]!.minValue,
    maxHour: i => buckets[i]!.maxHour,
    maxValue: i => buckets[i]!.maxValue,
    firstBreak: i => buckets[i]!.firstBreak,
    hasBreak: i => buckets[i]!.hasBreak,
  };
}

export interface SummaryShape {
  points: Float64Array;
  starts: Uint32Array;
  dots: Float64Array;
  end: readonly [number, number] | null;
  low: number;
  high: number;
}
const shapes = new WeakMap<ChartSummary, [SummaryShape | undefined, SummaryShape | undefined]>();

export function buildSummaryShape(stream: BucketStreamReader, held: boolean, to = Infinity): SummaryShape {
  const points: number[] = [], starts: number[] = [], dots: number[] = [];
  const step = held ? 1 : 0;
  const vertex = (hour: number, value: number) => {
    const at = points.length, open = at - 2 * starts[starts.length - 1];
    if (open >= 2 && points[at - 2] === hour && points[at - 1] === value) return;
    if (open >= 4 && points[at - 1] === value && points[at - 3] === value && points[at - 2] <= hour) { points[at - 2] = hour; return; }
    points.push(hour, value);
  };
  const corners = emitBucketCorners(0, 0, 0, 0, 0, 0, 0, 0);
  let hasPrev = false;
  let prevLastHour = 0;
  let prevLastValue = 0;
  let end: [number, number] | null = null;
  for (let i = 0; i < stream.count; i++) {
    if (!stream.present(i)) { hasPrev = false; continue; }
    const fh = stream.firstHour(i);
    const lh = stream.lastHour(i);
    const fv = stream.firstValue(i);
    const lv = stream.lastValue(i);
    emitBucketCorners(fh, stream.minHour(i), stream.maxHour(i), stream.lastValueHour(i), fv, stream.minValue(i), stream.maxValue(i), lv, corners);
    if (stream.hasBreak(i) || corners.some(corner => !Number.isFinite(corner.value))) {
      for (const corner of corners) if (Number.isFinite(corner.value)) dots.push(corner.hour, corner.value);
      hasPrev = false;
    } else {
      if (hasPrev && !stream.firstBreak(i) && prevLastHour + 1 === fh) {
        const at = points.length - 2;
        if (points[at] !== prevLastHour + step || points[at + 1] !== prevLastValue) {
          starts.push(points.length / 2); points.push(prevLastHour + step, prevLastValue);
        }
        vertex(fh, prevLastValue); vertex(fh, fv);
      } else { starts.push(points.length / 2); points.push(fh, fv); }
      let value = fv;
      for (const corner of corners) {
        if (held) vertex(corner.hour, value);
        vertex(corner.hour, corner.value); value = corner.value;
      }
      vertex(Math.min(to, lh + step), lv);
      hasPrev = true;
      prevLastHour = lh;
      prevLastValue = lv;
    }
    end = [Math.min(to, lh + step), lv];
  }
  starts.push(points.length / 2);
  let low = Infinity, high = -Infinity;
  for (const values of [points, dots]) for (let at = 1; at < values.length; at += 2) { low = Math.min(low, values[at]); high = Math.max(high, values[at]); }
  return { points: Float64Array.from(points), starts: Uint32Array.from(starts), dots: Float64Array.from(dots), end, low, high };
}

function summaryShape(summary: ChartSummary, held: boolean): SummaryShape {
  const kept = shapes.get(summary) ?? [undefined, undefined];
  const known = kept[held ? 1 : 0];
  if (known) return known;
  const shape = buildSummaryShape(bucketsStream(summary.buckets), held, summary.tiles ? Infinity : summary.to);
  kept[held ? 1 : 0] = shape;
  shapes.set(summary, kept);
  return shape;
}

const placeable = (frame: PlotFrame, shape: SummaryShape) => shape.low > shape.high
  || Number.isFinite(frame.toY(shape.low)) && Number.isFinite(frame.toY(shape.high));

function columnGeometryFromStream(frame: PlotFrame, stream: BucketStreamReader, held: boolean, to: number, before = Infinity): { paths: PathCommand[][]; dots: Point[]; end: Point | null } {
  const paths: PathCommand[][] = [], dots: Point[] = [], step = held ? 1 : 0;
  const corners = emitBucketCorners(0, 0, 0, 0, 0, 0, 0, 0);
  let hasPrev = false;
  let prevLastHour = 0;
  let prevLastValue = 0;
  let end: Point | null = null;
  for (let i = 0; i < stream.count; i++) {
    if (!stream.present(i)) { hasPrev = false; continue; }
    const lh = stream.lastHour(i);
    if (lh >= before) break;
    const fh = stream.firstHour(i);
    const fv = stream.firstValue(i);
    const lv = stream.lastValue(i);
    emitBucketCorners(fh, stream.minHour(i), stream.maxHour(i), stream.lastValueHour(i), fv, stream.minValue(i), stream.maxValue(i), lv, corners);
    if (stream.hasBreak(i) || corners.some(corner => !Number.isFinite(frame.toY(corner.value)))) {
      for (const corner of corners) if (Number.isFinite(frame.toY(corner.value))) dots.push({ x: frame.toX(corner.hour), y: frame.toY(corner.value) });
      hasPrev = false;
    } else {
      const path: PathCommand[] = [];
      const push = (move: boolean, hour: number, value: number) => path.push([move ? 'M' : 'L', frame.toX(hour), frame.toY(value)]);
      if (hasPrev && !stream.firstBreak(i) && prevLastHour + 1 === fh) {
        push(true, prevLastHour + step, prevLastValue); push(false, fh, prevLastValue); push(false, fh, fv);
      } else push(true, fh, fv);
      let value = fv;
      for (const corner of corners) {
        if (held) push(false, corner.hour, value);
        push(false, corner.hour, corner.value); value = corner.value;
      }
      push(false, Math.min(to, lh + step), lv);
      paths.push(path);
      hasPrev = true;
      prevLastHour = lh;
      prevLastValue = lv;
    }
    end = { x: frame.toX(Math.min(to, lh + step)), y: frame.toY(lv) };
  }
  return { paths, dots, end: end && Number.isFinite(end.y) ? end : null };
}

function shapeEnd(frame: PlotFrame, shape: SummaryShape): Point | null {
  if (!shape.end) return null;
  const end = { x: frame.toX(shape.end[0]), y: frame.toY(shape.end[1]) };
  return Number.isFinite(end.y) ? end : null;
}

export function drawApproximateSpans(ctx: Canvas2D, frame: PlotFrame,
  approximate: ChartSummary['approximate'], until: number | null, draw: (approximate: boolean) => void): void {
  if (approximate?.length) {
    const regions: { first: number; last: number; approximate: boolean }[] = [];
    const end = Math.min(frame.x[1], until ?? frame.x[1]);
    let first = frame.x[0];
    for (const union of approximateUnion(approximate)) {
      const span = { first: Math.max(frame.x[0], union.firstHour), last: Math.min(end, union.lastHour + 1) };
      if (span.first > span.last) continue;
      if (first < span.first) regions.push({ first, last: span.first, approximate: false });
      regions.push({ ...span, approximate: true }); first = span.last;
    }
    if (first < end) regions.push({ first, last: end, approximate: false });
    for (const approximate of [false, true]) {
      const spans = regions.filter(region => region.approximate === approximate);
      if (!spans.length) continue;
      ctx.save(); ctx.beginPath();
      for (const region of spans) ctx.rect(frame.toX(region.first), frame.plotTop, frame.toX(region.last) - frame.toX(region.first), frame.plotHeight);
      ctx.clip();
      try {
        if (approximate) { ctx.globalAlpha *= 0.55; ctx.setLineDash([4, 4]); }
        draw(approximate);
      } finally { ctx.restore(); }
    }
  } else draw(false);
}

export type SummaryLineOptions = { area?: boolean; dash?: readonly number[]; endDot?: boolean };

function withHourClip<T>(ctx: Canvas2D, frame: PlotFrame, hours: readonly [number, number] | undefined, paint: () => T): T {
  if (!hours) return paint();
  const left = Math.max(frame.plotLeft, frame.toX(hours[0]));
  const right = frame.toX(hours[1]);
  ctx.save();
  ctx.beginPath();
  ctx.rect(left, frame.plotTop, Math.max(0, right - left), frame.plotHeight);
  ctx.clip();
  try {
    return paint();
  } finally {
    ctx.restore();
  }
}

export function drawSummaryShape(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens,
  shape: SummaryShape, color: string, options: SummaryLineOptions = {},
  hours?: readonly [number, number],
  fallback?: { readonly stream: BucketStreamReader; readonly held: boolean; readonly to: number }): Point | null {
  const drawn = hours ? { ...options, endDot: false } : options;
  if (placeable(frame, shape)) {
    return withHourClip(ctx, frame, hours, () => strokeShape(ctx, frame, tokens, shape, color, drawn, hours));
  }
  if (!fallback) return null;
  return withHourClip(ctx, frame, hours, () => {
    const geometry = columnGeometryFromStream(frame, fallback.stream, fallback.held, fallback.to, hours ? hours[1] : Infinity);
    return strokeColumns(ctx, frame, tokens, hours ? { ...geometry, end: null } : geometry, color, drawn);
  });
}

export function drawLineSummary(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens,
  summary: ChartSummary, color: string, held = false, until: number | null = null,
  options: SummaryLineOptions = {}): Point | null {
  if (!summaryFits(frame, summary, until)) return null;
  if (summary.tiles && !summary.runs) return drawTiledHead(ctx, frame, tokens, summary, summary.tiles, color, held, until ?? frame.x[1], options);
  const runs = exactRuns(summary);
  const shape = runs ? null : summaryShape(summary, held);
  const paint = (drawn: SummaryLineOptions) => !shape ? drawLine(ctx, frame, tokens, runs!, color, { ...drawn, held, until: until ?? undefined })
    : drawSummaryShape(ctx, frame, tokens, shape, color, drawn, undefined, { stream: bucketsStream(summary.buckets), held, to: summary.to });
  if (!summary.approximate?.length) return paint(options);
  let end: Point | null = null;
  drawApproximateSpans(ctx, frame, summary.approximate, until, approximate => {
    end = paint(approximate ? { ...options, dash: [4, 4], area: false, endDot: false } : options) ?? end;
  });
  return end;
}

function lineStyle(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, color: string, options: SummaryLineOptions): number {
  ctx.save(); ctx.strokeStyle = color; ctx.fillStyle = color;
  ctx.lineWidth = tokens.lineWidth.series; ctx.lineJoin = 'round';
  if (options.dash) ctx.setLineDash(options.dash);
  return frame.toY(frame.log ? frame.y[0] : Math.max(frame.y[0], Math.min(0, frame.y[1])));
}

function lineEnd(ctx: Canvas2D, tokens: ChartTokens, end: Point | null, options: SummaryLineOptions): Point | null {
  if (end && options.endDot !== false) {
    ctx.beginPath(); ctx.arc(end.x, end.y, tokens.lineWidth.series * 2, 0, Math.PI * 2); ctx.fill();
    ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  ctx.restore(); return end;
}

function strokeColumns(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens,
  geometry: ReturnType<typeof columnGeometryFromStream>, color: string, options: SummaryLineOptions): Point | null {
  const baseline = lineStyle(ctx, frame, tokens, color, options);
  for (const path of geometry.paths) {
    if (options.area) {
      ctx.beginPath(); tracePath(ctx, path);
      ctx.lineTo(path.at(-1)![1], baseline); ctx.lineTo(path[0][1], baseline); ctx.closePath();
      ctx.globalAlpha = 0.12; ctx.fill(); ctx.globalAlpha = 1;
    }
    ctx.beginPath(); tracePath(ctx, path); ctx.stroke();
  }
  for (const point of geometry.dots) { ctx.beginPath(); ctx.arc(point.x, point.y, tokens.lineWidth.series / 2, 0, Math.PI * 2); ctx.fill(); }
  return lineEnd(ctx, tokens, geometry.end, options);
}

function strokeShape(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, shape: SummaryShape,
  color: string, options: SummaryLineOptions, hours?: readonly [number, number]): Point | null {
  const { points, starts, dots } = shape;
  const baseline = lineStyle(ctx, frame, tokens, color, options);
  const trace = (from: number, to: number) => {
    ctx.moveTo(frame.toX(points[2 * from]), frame.toY(points[2 * from + 1]));
    for (let vertex = from + 1; vertex < to; vertex++) ctx.lineTo(frame.toX(points[2 * vertex]), frame.toY(points[2 * vertex + 1]));
  };
  const lines = starts.length - 1, hourAt = (vertex: number) => points[2 * vertex];
  for (let line = hours ? lower(lines, index => hourAt(starts[index + 1] - 1) < hours[0]) : 0; line < lines; line++) {
    let from = starts[line], to = starts[line + 1];
    if (hours) {
      if (hourAt(from) > hours[1]) break;
      const start = from;
      from = Math.max(start, start + lower(to - start, index => hourAt(start + index) < hours[0]) - 1);
      to = Math.min(to, start + lower(to - start, index => hourAt(start + index) <= hours[1]) + 1);
    }
    if (options.area) {
      ctx.beginPath(); trace(from, to);
      ctx.lineTo(frame.toX(points[2 * (to - 1)]), baseline); ctx.lineTo(frame.toX(points[2 * from]), baseline); ctx.closePath();
      ctx.globalAlpha = 0.12; ctx.fill(); ctx.globalAlpha = 1;
    }
    ctx.beginPath(); trace(from, to); ctx.stroke();
  }
  for (let at = hours ? 2 * lower(dots.length / 2, index => dots[2 * index] < hours[0]) : 0; at < dots.length && (!hours || dots[at] <= hours[1]); at += 2) {
    ctx.beginPath(); ctx.arc(frame.toX(dots[at]), frame.toY(dots[at + 1]), tokens.lineWidth.series / 2, 0, Math.PI * 2); ctx.fill();
  }
  return lineEnd(ctx, tokens, hours ? null : shapeEnd(frame, shape), options);
}

function drawTiledHead(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, summary: ChartSummary,
  tiles: TiledSummary, color: string, held: boolean, until: number, options: SummaryLineOptions): Point | null {
  const cut = Math.floor(until / tiles.binHours) * tiles.binHours;
  const shape = summaryShape(summary, held);
  drawSummaryShape(ctx, frame, tokens, shape, color, options, [frame.x[0], cut], { stream: bucketsStream(summary.buckets), held, to: summary.to });
  const value = tiles.valueAt(until), y = value === null ? NaN : frame.toY(value);
  if (!Number.isFinite(y)) return null;
  const baseline = lineStyle(ctx, frame, tokens, color, options);
  const left = frame.toX(Math.max(cut - 1, frame.x[0])), right = frame.toX(Math.min(until, frame.x[1]));
  if (right < left) { ctx.restore(); return null; }
  if (options.area) {
    ctx.globalAlpha = 0.12; ctx.fillRect(left, Math.min(y, baseline), right - left, Math.abs(baseline - y)); ctx.globalAlpha = 1;
  }
  ctx.beginPath(); ctx.moveTo(left, y); ctx.lineTo(right, y); ctx.stroke();
  return lineEnd(ctx, tokens, { x: right, y }, options);
}

function lower(count: number, before: (index: number) => boolean): number {
  let low = 0, high = count;
  while (low < high) { const middle = (low + high) >> 1; if (before(middle)) low = middle + 1; else high = middle; }
  return low;
}

export function summaryStack(parts: readonly ChartSummary[], total?: ChartSummary) {
  const summaries = total ? [...parts,total] : parts;
  const approximate = approximateUnion(summaries.flatMap(summary => summary.approximate ?? []));
  const samples = (bucket: ChartBucket) => [...new Map([
    [bucket.firstHour,bucket.firstValue],[bucket.minHour,bucket.minValue],
    [bucket.maxHour,bucket.maxValue],[bucket.lastValueHour,bucket.lastValue],
    [bucket.lastHour,bucket.lastValue],
  ]).entries()].sort(([a],[b])=>a-b).map(([,value])=>value);
  const sampled = summaries.map(summary => summary.buckets.map(bucket => bucket ? samples(bucket) : null));
  const points: {firstHour:number;lastHour:number;column:number;stage:number;count:number;breakBefore:boolean}[]=[];
  for (let column=0;column<(summaries[0]?.buckets.length ?? 0);column++) {
    const buckets=summaries.map(summary=>summary.buckets[column]);
    if (buckets.some(bucket=>!bucket)) continue;
    let firstHour=-Infinity,lastHour=Infinity;
    for (const bucket of buckets) {firstHour=Math.max(firstHour,bucket!.firstHour);lastHour=Math.min(lastHour,bucket!.lastHour);}
    if (firstHour>lastHour) continue;
    const count=sampled.reduce((count,columns)=>Math.max(count,columns[column]!.length),1);
    const range={firstHour,lastHour};
    const broken=buckets.some(bucket=>bucket!.hasBreak);
    const firstBreak=buckets.some(bucket=>bucket!.firstBreak);
    const hours=Array.from({length:count},(_,index)=>count===1 ? range.firstHour
      : range.firstHour+Math.floor((range.lastHour-range.firstHour)*index/(count-1)));
    for (let stage=0;stage<count;stage++) points.push({
      firstHour:hours[stage],lastHour:broken ? hours[stage] : (hours[stage+1] ?? range.lastHour+1)-1,
      column,stage,count,breakBefore:broken || stage===0 && firstBreak,
    });
  }
  const valuesAt = (point:typeof points[number]) => {
    const values=sampled.map(columns=> {
      const values=columns[point.column]!;
      return values[point.count===1 ? 0 : Math.floor(point.stage*(values.length-1)/(point.count-1))];
    });
    if (total) {
      const sum={sum:0,error:0};
      for (let index=0;index<parts.length;index++) addStackAmount(sum,values[index]);
      values[parts.length]-=sum.sum+sum.error;
    }
    return values;
  };
  const hours=[...new Set(points.flatMap(point=>[point.firstHour,point.lastHour,point.lastHour+1]))].sort((a,b)=>a-b);
  const model: CompactStackModel={bands:[],hours,bounds:[],breakBefore:[]};
  for (const hour of hours) {
    const point=runAt(points,hour);
    if (!point) {model.bounds.push(null);model.breakBefore.push(false);continue;}
    const up={sum:0,error:0},down={sum:0,error:0};
    for (const value of valuesAt(point)) addStackAmount(value>=0 ? up : down,value);
    model.bounds.push([down.sum+down.error,up.sum+up.error]);
    model.breakBefore.push(point.firstHour===hour && point.breakBefore);
  }
  const sample=(hour:number)=> {
    let low=0,high=points.length;
    while (low<high) {const middle=(low+high)>>1;if(points[middle].firstHour<=hour)low=middle+1;else high=middle;}
    return stackEdges(valuesAt(points[Math.max(0,low-1)]));
  };
  return {model,sample,approximate};
}
