import type { PlotFrame } from './frame.ts';
import type { ChartTokens } from './tokens.ts';
import { dayLabel, drawGapFill, drawLine, spanLabel, tracePath } from './draw.ts';
import { ordinates } from './linePick.ts';
import { clipToSpans, curveSegments, extentOf, heldSegments, isOrderedSpans, runAt } from './series.ts';
import type { PathCommand, Point, Run } from './series.ts';
import type { ChartPick } from './TimeChart';
import { addStackAmount, stackEdges } from './stack.ts';
import type { CompactStackModel } from './stack';

/** Display geometry only. Exact amounts and supplying reads use the hour query. */
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
  /** Original supplying runs when every requested bin is one recorded hour. */
  runs?: readonly Run[];
  approximate?: readonly { firstHour: number; lastHour: number }[];
  /** Assembled from tiles: its bins are whole tiles', past the hours shown; a painter clips them at the head and
   * draws the head's own bin at `valueAt(head)`. */
  tiles?: TiledSummary;
}

export interface TiledSummary {
  binHours: number;
  /** The line's value at the head, drawn from the head bin's first hour to the head. */
  valueAt(hour: number): number | null;
}

const decoded = new WeakMap<object, ChartSummary | null>();
const safe = Number.isSafeInteger;
const within = (at: number, span: number) => safe(at) && at >= 0 && at <= span;

/** A summary as the dashboard sends it, its bucket columns (`ChartColumns`) made buckets, or null
 * when malformed. Decoded once per reply object, however many validators ask. */
export function chartSummaryFrom(value: unknown): ChartSummary | null {
  if (!value || typeof value !== 'object') return null;
  let summary = decoded.get(value);
  if (summary === undefined) decoded.set(value, summary = decodeSummary(value as WireSummary));
  return summary;
}

/** A record of wire summaries, each decoded, or null when it or any summary in it is malformed. */
export function chartSummaryRecord(value: unknown): Record<string, ChartSummary> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const record: Record<string, ChartSummary> = {};
  for (const [key, raw] of Object.entries(value)) {
    const summary = chartSummaryFrom(raw);
    if (!summary) return null;
    record[key] = summary;
  }
  return record;
}

/** A reply line as the UI holds it: its wire `summary` decoded. */
export type Decoded<L extends { summary: unknown }> = Omit<L, 'summary'> & { summary: ChartSummary };

/** Lines holding a wire `summary` each, decoded, or null when any line fails `valid` or its summary is malformed. */
export function chartSummaryLines<L extends { summary: ChartSummary }>(value: unknown, valid: (line: Record<string, unknown>) => boolean): L[] | null {
  if (!Array.isArray(value)) return null;
  const lines: L[] = [];
  for (const line of value as unknown[]) {
    const summary = line && typeof line === 'object' && valid(line as Record<string, unknown>) ? chartSummaryFrom((line as { summary: unknown }).summary) : null;
    if (!summary) return null;
    lines.push({ ...line as object, summary } as L);
  }
  return lines;
}

interface WireSummary extends Omit<ChartSummary, 'buckets'> {
  buckets: { slots: number; skip: readonly number[]; hours: readonly number[]; values: readonly number[]; flags: readonly number[]; totals: readonly string[] };
}

function decodeSummary(summary: WireSummary): ChartSummary | null {
  if (!Number.isFinite(summary.from) || !Number.isFinite(summary.to) || summary.from > summary.to || !safe(summary.width) || summary.width <= 0
    || summary.runs !== undefined && !(isOrderedSpans(summary.runs) && summary.runs.length <= 2 * summary.width + 2
      && summary.runs.every(run => Number.isFinite(run.value) && typeof run.breakBefore === 'boolean'))
    || summary.approximate !== undefined && !(isOrderedSpans(summary.approximate)
      && summary.approximate.every(span => safe(span.firstHour) && safe(span.lastHour)))) return null;
  const columns = summary.buckets;
  if (!columns || typeof columns !== 'object' || !safe(columns.slots) || columns.slots < 0 || columns.slots > 2 * summary.width + 2) return null;
  const { slots, skip, hours, values, flags, totals } = columns;
  const present = Array.isArray(skip) ? skip.length : -1;
  if (present < 0 || !Array.isArray(hours) || hours.length !== 5 * present || !Array.isArray(values) || values.length !== 4 * present
    || !Array.isArray(flags) || flags.length !== present || !Array.isArray(totals) || totals.length !== present) return null;
  const buckets: (ChartBucket | null)[] = new Array<ChartBucket | null>(slots).fill(null);
  let slot = 0, previous: number | null = null;
  for (let index = 0; index < present; index++) {
    const gap = skip[index], flag = flags[index], absoluteTotal = totals[index];
    const firstHour = previous === null ? hours[5 * index] : previous + hours[5 * index];
    const span = hours[5 * index + 1], lastValueAt = hours[5 * index + 2], minAt = hours[5 * index + 3], maxAt = hours[5 * index + 4];
    const firstValue = values[4 * index], lastValue = values[4 * index + 1], minValue = values[4 * index + 2], maxValue = values[4 * index + 3];
    slot += gap;
    if (!safe(gap) || gap < 0 || slot >= slots || !safe(firstHour) || !safe(span) || span < 0 || !safe(firstHour + span)
      || !within(lastValueAt, span) || !within(minAt, span) || !within(maxAt, span)
      || !Number.isFinite(firstValue) || !Number.isFinite(lastValue) || !Number.isFinite(minValue) || !Number.isFinite(maxValue)
      || !(flag === 0 || flag === 1 || flag === 2 || flag === 3) || typeof absoluteTotal !== 'string' || !/^\d+$/.test(absoluteTotal)) return null;
    const bucket: ChartBucket & { absoluteTotal: string } = { firstHour, lastHour: firstHour + span, firstValue, lastValue, lastValueHour: firstHour + lastValueAt,
      minHour: firstHour + minAt, minValue, maxHour: firstHour + maxAt, maxValue, firstBreak: (flag & 1) !== 0, hasBreak: (flag & 2) !== 0, absoluteTotal };
    buckets[slot++] = bucket;
    previous = bucket.lastHour;
  }
  return { ...summary, buckets };
}

/** Bucket extrema alone lose supplying bounds and equal plotted-value boundaries. */
export const exactRuns = (summary: ChartSummary): readonly Run[] | null => summary.runs ?? null;

/** The bin of `summary` holding `hour`, or null. */
export const binAt = (summary: ChartSummary | null | undefined, hour: number): ChartBucket | null =>
  summary?.buckets.find(bin => bin && bin.firstHour <= hour && hour <= bin.lastHour) ?? null;

/** When the values of `bins` (each line's bin holding `hour`) were so: the days their hours span, since a bin's value
 * and its extremes' hours are known only to the bin (its summary's 6- or 96-hour bucket at best); `hour`'s day where
 * no bin is wider than one hour. */
export function binsLabel(hour: number, bins: readonly (ChartBucket | null | undefined)[]): string {
  let first = Infinity, last = -Infinity;
  for (const bin of bins) if (bin) { first = Math.min(first, bin.firstHour); last = Math.max(last, bin.lastHour); }
  return first < last ? spanLabel(first, last) : dayLabel(hour);
}

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
  // The inclusive hours exact runs use; a bucket partly in them keeps its whole envelope.
  const first = Math.floor(from);
  let low = Infinity, high = -Infinity;
  if (summary.tiles) {
    // Tiled bins run past `to`: only those wholly before its bin, then the value drawn at it.
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

/** `summary` drawn in another unit: every value times `factor`, the head a tiled summary draws at included. */
export function scaleSummary(summary: ChartSummary, factor: number): ChartSummary {
  if (factor === 1) return summary;
  const tiles = summary.tiles;
  return { ...summary, buckets: summary.buckets.map(bucket => bucket && { ...bucket, firstValue: bucket.firstValue * factor,
    lastValue: bucket.lastValue * factor, minValue: bucket.minValue * factor, maxValue: bucket.maxValue * factor }),
  ...(summary.runs ? { runs: summary.runs.map(run => ({ ...run, value: run.value * factor })) } : {}),
  ...(tiles ? { tiles: { ...tiles, valueAt: (hour: number) => { const value = tiles.valueAt(hour); return value === null ? null : value * factor; } } } : {}) };
}

/** A layer's value extent of `summary` in a frame showing `x` up to `until`: none when it cannot be drawn there. */
export function summaryExtent(summary: ChartSummary, x: readonly [number, number], until: number | null): [number, number] | null {
  const end = Math.min(x[1], until ?? x[1]);
  if (summary.tiles) return summary.from > x[1] || summary.to < x[0] ? null : extentOfSummary(summary, x[0], end);
  return summary.to > end || summary.to < x[0] ? null : extentOfSummary(summary);
}

/** Whether a summary can be drawn in this frame: one requested at the plot's width
 * that overlaps it and ends at or before the hours shown end. Its geometry is placed
 * by hour, so a summary asked for a moment ago (the view has moved on during
 * playback or a zoom) draws in place, never past the playhead, until the next arrives. */
export const summaryFits = (frame: Pick<PlotFrame, 'x' | 'plotWidth'>, summary: { from: number; to: number; width: number; tiles?: unknown }, until: number | null) => summary.width === Math.ceil(frame.plotWidth)
  && (summary.tiles !== undefined || summary.to <= Math.min(frame.x[1], until ?? frame.x[1])) && summary.to >= frame.x[0] && summary.from <= frame.x[1];

/** A dense summary's drawing in hours and values: a frame only places it, so it is built once
 * per summary and step kind. Joined columns continue one polyline, so a line strokes once per
 * unbroken stretch rather than once per column. */
interface SummaryShape {
  /** Each vertex's hour and value, interleaved, polyline after polyline, hours never falling. */
  points: Float64Array;
  /** The vertex each polyline starts at, then the vertex count. */
  starts: Uint32Array;
  /** Broken columns' extrema, hour and value interleaved, in hour order. */
  dots: Float64Array;
  /** The last column's end hour and value. */
  end: readonly [number, number] | null;
  /** The least and greatest value drawn, so a frame checks it can place them all at once. */
  low: number;
  high: number;
}
const shapes = new WeakMap<ChartSummary, [SummaryShape | undefined, SummaryShape | undefined]>();

/** A bounded summary never substitutes for exact picks, hover or keyboard reads.
 * Broken columns show separate extrema dots: no stroke crosses an unknown span.
 * Continuous columns retain the time order of extrema and held step direction. */
function summaryShape(summary: ChartSummary, held: boolean): SummaryShape {
  const kept = shapes.get(summary) ?? [undefined, undefined];
  const known = kept[held ? 1 : 0];
  if (known) return known;
  const points: number[] = [], starts: number[] = [], dots: number[] = [];
  const step = held ? 1 : 0;
  // Tiled bins run on past the hours shown; their own last hour ends them.
  const to = summary.tiles ? Infinity : summary.to;
  // A vertex the polyline already ends at is left out, and one inside a level run moves the run's end: the stroke
  // is the same, with far fewer vertices for a frame to place.
  const vertex = (hour: number, value: number) => {
    const at = points.length, open = at - 2 * starts[starts.length - 1];
    if (open >= 2 && points[at - 2] === hour && points[at - 1] === value) return;
    if (open >= 4 && points[at - 1] === value && points[at - 3] === value && points[at - 2] <= hour) { points[at - 2] = hour; return; }
    points.push(hour, value);
  };
  let previous: ChartBucket | null = null;
  let end: [number, number] | null = null;
  for (const bucket of summary.buckets) {
    if (!bucket) { previous = null; continue; }
    const corners = [
      { hour: bucket.firstHour, value: bucket.firstValue, order: 0 },
      { hour: bucket.minHour, value: bucket.minValue, order: 1 },
      { hour: bucket.maxHour, value: bucket.maxValue, order: 2 },
      { hour: bucket.lastValueHour, value: bucket.lastValue, order: 3 },
    ].sort((a, b) => a.hour - b.hour || a.order - b.order);
    // A value no frame can place breaks the column, as an unknown span does.
    if (bucket.hasBreak || corners.some(corner => !Number.isFinite(corner.value))) {
      for (const corner of corners) if (Number.isFinite(corner.value)) dots.push(corner.hour, corner.value);
      previous = null;
    } else {
      if (previous && !bucket.firstBreak && previous.lastHour + 1 === bucket.firstHour) {
        // The previous column ended where this join starts, so its polyline goes on.
        const at = points.length - 2;
        if (points[at] !== previous.lastHour + step || points[at + 1] !== previous.lastValue) {
          starts.push(points.length / 2); points.push(previous.lastHour + step, previous.lastValue);
        }
        vertex(bucket.firstHour, previous.lastValue); vertex(bucket.firstHour, bucket.firstValue);
      } else { starts.push(points.length / 2); points.push(bucket.firstHour, bucket.firstValue); }
      let value = bucket.firstValue;
      for (const corner of corners) {
        if (held) vertex(corner.hour, value);
        vertex(corner.hour, corner.value); value = corner.value;
      }
      vertex(Math.min(to, bucket.lastHour + step), bucket.lastValue);
      previous = bucket;
    }
    end = [Math.min(to, bucket.lastHour + step), bucket.lastValue];
  }
  starts.push(points.length / 2);
  let low = Infinity, high = -Infinity;
  for (const values of [points, dots]) for (let at = 1; at < values.length; at += 2) { low = Math.min(low, values[at]); high = Math.max(high, values[at]); }
  const shape = { points: Float64Array.from(points), starts: Uint32Array.from(starts), dots: Float64Array.from(dots), end, low, high };
  kept[held ? 1 : 0] = shape;
  shapes.set(summary, kept);
  return shape;
}

/** Whether `frame` places every value of `shape`: its projection is monotonic, so the extremes decide. */
const placeable = (frame: PlotFrame, shape: SummaryShape) => shape.low > shape.high
  || Number.isFinite(frame.toY(shape.low)) && Number.isFinite(frame.toY(shape.high));

/** Column by column geometry, for a frame that cannot place some value: such a column is only its placed dots. */
function columnGeometry(frame: PlotFrame, summary: ChartSummary, held: boolean, before = Infinity): { paths: PathCommand[][]; dots: Point[]; end: Point | null } {
  const paths: PathCommand[][] = [], dots: Point[] = [], step = held ? 1 : 0;
  let previous: ChartBucket | null = null, end: Point | null = null;
  for (const bucket of summary.buckets) {
    if (!bucket) { previous = null; continue; }
    if (bucket.lastHour >= before) break;
    const corners = [
      { hour: bucket.firstHour, value: bucket.firstValue, order: 0 },
      { hour: bucket.minHour, value: bucket.minValue, order: 1 },
      { hour: bucket.maxHour, value: bucket.maxValue, order: 2 },
      { hour: bucket.lastValueHour, value: bucket.lastValue, order: 3 },
    ].sort((a, b) => a.hour - b.hour || a.order - b.order);
    if (bucket.hasBreak || corners.some(corner => !Number.isFinite(frame.toY(corner.value)))) {
      for (const corner of corners) if (Number.isFinite(frame.toY(corner.value))) dots.push({ x: frame.toX(corner.hour), y: frame.toY(corner.value) });
      previous = null;
    } else {
      const path: PathCommand[] = [];
      const push = (move: boolean, hour: number, value: number) => path.push([move ? 'M' : 'L', frame.toX(hour), frame.toY(value)]);
      if (previous && !bucket.firstBreak && previous.lastHour + 1 === bucket.firstHour) {
        push(true, previous.lastHour + step, previous.lastValue); push(false, bucket.firstHour, previous.lastValue); push(false, bucket.firstHour, bucket.firstValue);
      } else push(true, bucket.firstHour, bucket.firstValue);
      let value = bucket.firstValue;
      for (const corner of corners) {
        if (held) push(false, corner.hour, value);
        push(false, corner.hour, corner.value); value = corner.value;
      }
      push(false, Math.min(summary.to, bucket.lastHour + step), bucket.lastValue);
      paths.push(path); previous = bucket;
    }
    end = { x: frame.toX(Math.min(summary.to, bucket.lastHour + step)), y: frame.toY(bucket.lastValue) };
  }
  return { paths, dots, end: end && Number.isFinite(end.y) ? end : null };
}

/** The shape's end placed in `frame`, when it can be. */
function shapeEnd(frame: PlotFrame, shape: SummaryShape): Point | null {
  if (!shape.end) return null;
  const end = { x: frame.toX(shape.end[0]), y: frame.toY(shape.end[1]) };
  return Number.isFinite(end.y) ? end : null;
}

/** Vertices `from` up to `to` of the shape as path commands placed in `frame`, opening with a move. */
function shapeCommands(frame: PlotFrame, shape: SummaryShape, from: number, to: number): PathCommand[] {
  const path: PathCommand[] = [];
  for (let vertex = from; vertex < to; vertex++) {
    path.push([vertex === from ? 'M' : 'L', frame.toX(shape.points[2 * vertex]), frame.toY(shape.points[2 * vertex + 1])]);
  }
  return path;
}

export function summaryGeometry(frame: PlotFrame, summary: ChartSummary, held = false, until: number | null = null):
  { paths: PathCommand[][]; dots: Point[]; end: Point | null } {
  // A retained reply keeps its own hours: drawn in a moved view it is never
  // stretched, and it never reaches past the hours shown.
  if (!summaryFits(frame, summary, until)) return { paths: [], dots: [], end: null };
  const runs = exactRuns(summary);
  if (runs) {
    const segments = (held ? heldSegments : curveSegments)(runs, frame.x[0], summary.to, frame.toX, frame.toY);
    return { paths: segments.map(segment => segment.path), dots: [], end: segments.at(-1)?.points.at(-1) ?? null };
  }
  const shape = summaryShape(summary, held), paths: PathCommand[][] = [], dots: Point[] = [];
  if (!placeable(frame, shape)) return columnGeometry(frame, summary, held);
  for (let line = 0; line + 1 < shape.starts.length; line++) paths.push(shapeCommands(frame, shape, shape.starts[line], shape.starts[line + 1]));
  for (let at = 0; at < shape.dots.length; at += 2) dots.push({ x: frame.toX(shape.dots[at]), y: frame.toY(shape.dots[at + 1]) });
  return { paths, dots, end: shapeEnd(frame, shape) };
}

/** Paint the original geometry in at most two clipped passes. Approximate
 * strokes share one fade and dash; the caller can also omit area and end dots. */
export function drawApproximateSpans(ctx: CanvasRenderingContext2D, frame: PlotFrame,
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
    // Two clipped passes retain the original geometry without redrawing it once
    // per span. A recording with many gaps still costs screen width plus spans.
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

type SummaryLineOptions = { area?: boolean; dash?: readonly number[]; endDot?: boolean };

export function drawLineSummary(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens,
  summary: ChartSummary, color: string, held = false, until: number | null = null,
  options: SummaryLineOptions = {}): Point | null {
  if (!summaryFits(frame, summary, until)) return null;
  // A tiled hourly line is its runs, drawn to the head as runs are; binned tiles stop at the head bin.
  if (summary.tiles && !summary.runs) return drawTiledHead(ctx, frame, tokens, summary, summary.tiles, color, held, until ?? frame.x[1], options);
  const runs = exactRuns(summary);
  const shape = runs ? null : summaryShape(summary, held);
  const paint = (drawn: SummaryLineOptions) => !shape ? drawLine(ctx, frame, tokens, runs!, color, { ...drawn, held, until: until ?? undefined })
    : placeable(frame, shape) ? strokeShape(ctx, frame, tokens, shape, color, drawn)
      : strokeColumns(ctx, frame, tokens, columnGeometry(frame, summary, held), color, drawn);
  if (!summary.approximate?.length) return paint(options);
  let end: Point | null = null;
  drawApproximateSpans(ctx, frame, summary.approximate, until, approximate => {
    end = paint(approximate ? { ...options, dash: [4, 4], area: false, endDot: false } : options) ?? end;
  });
  return end;
}

/** Sets up a summary line's stroke and returns its area's baseline in `frame`. */
function lineStyle(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, color: string, options: SummaryLineOptions): number {
  ctx.save(); ctx.strokeStyle = color; ctx.fillStyle = color;
  ctx.lineWidth = tokens.lineWidth.series; ctx.lineJoin = 'round';
  if (options.dash) ctx.setLineDash(options.dash);
  return frame.toY(frame.log ? frame.y[0] : Math.max(frame.y[0], Math.min(0, frame.y[1])));
}

/** Paints a summary line's end dot unless left out, and ends its stroke. */
function lineEnd(ctx: CanvasRenderingContext2D, tokens: ChartTokens, end: Point | null, options: SummaryLineOptions): Point | null {
  if (end && options.endDot !== false) {
    ctx.beginPath(); ctx.arc(end.x, end.y, tokens.lineWidth.series * 2, 0, Math.PI * 2); ctx.fill();
    ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  ctx.restore(); return end;
}

/** Strokes column by column geometry, as a frame that cannot place the whole shape draws it. */
function strokeColumns(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens,
  geometry: ReturnType<typeof columnGeometry>, color: string, options: SummaryLineOptions): Point | null {
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

/** Strokes each polyline of `shape` placed in `frame`, with its area, broken columns' dots and end dot; with
 * `hours`, only the vertices about them (a tiled shape runs far past the frame). */
function strokeShape(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, shape: SummaryShape,
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
      // From the vertex before the first hour through the first vertex past the last.
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

/** A tiled line up to the head: its bins wholly before the head's bin, clipped there, then the head's bin from its
 * first hour to the head at the value at the head. Only vertices in the frame are placed. */
function drawTiledHead(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, summary: ChartSummary,
  tiles: TiledSummary, color: string, held: boolean, until: number, options: SummaryLineOptions): Point | null {
  const cut = Math.floor(until / tiles.binHours) * tiles.binHours;
  const shape = summaryShape(summary, held);
  ctx.save();
  ctx.beginPath(); ctx.rect(frame.plotLeft, frame.plotTop, Math.max(0, frame.toX(cut) - frame.plotLeft), frame.plotHeight); ctx.clip();
  if (placeable(frame, shape)) strokeShape(ctx, frame, tokens, shape, color, { ...options, endDot: false }, [frame.x[0], cut]);
  else strokeColumns(ctx, frame, tokens, columnGeometry(frame, summary, held, cut), color, { ...options, endDot: false });
  ctx.restore();
  const value = tiles.valueAt(until), y = value === null ? NaN : frame.toY(value);
  if (!Number.isFinite(y)) return null;
  const baseline = lineStyle(ctx, frame, tokens, color, options);
  // From the last hour of the bin before, which the clip leaves out of the line's join.
  const left = frame.toX(Math.max(cut - 1, frame.x[0])), right = frame.toX(Math.min(until, frame.x[1]));
  if (right < left) { ctx.restore(); return null; }
  if (options.area) {
    ctx.globalAlpha = 0.12; ctx.fillRect(left, Math.min(y, baseline), right - left, Math.abs(baseline - y)); ctx.globalAlpha = 1;
  }
  ctx.beginPath(); ctx.moveTo(left, y); ctx.lineTo(right, y); ctx.stroke();
  return lineEnd(ctx, tokens, { x: right, y }, options);
}

/** The first of `count` indices `before` no longer holds for; it holds for a prefix. */
function lower(count: number, before: (index: number) => boolean): number {
  let low = 0, high = count;
  while (low < high) { const middle = (low + high) >> 1; if (before(middle)) low = middle + 1; else high = middle; }
  return low;
}

/** The heights `shape` crosses `x` at in `frame`, as `ordinates` finds them over each polyline;
 * only the vertices about `x` are placed. */
function shapeOrdinates(frame: PlotFrame, shape: SummaryShape, x: number, targetY: number): number[] {
  const { points, starts } = shape, lines = starts.length - 1;
  const at = (vertex: number) => frame.toX(points[2 * vertex]);
  const ys: number[] = [];
  // Polylines run left to right: from the first ending at or past x, while they start by it.
  for (let line = lower(lines, index => at(starts[index + 1] - 1) < x - 1e-6); line < lines; line++) {
    const from = starts[line], to = starts[line + 1];
    if (at(from) > x + 1e-6) break;
    // A window opening left of x (or at the polyline's own start) and closing right of it holds every crossing.
    const first = Math.max(from, from + lower(to - from, index => at(from + index) < x - 1e-6) - 1);
    const past = from + lower(to - from, index => at(from + index) <= x + 1e-6);
    ys.push(...ordinates(shapeCommands(frame, shape, first, Math.min(to, past + 1)), x, targetY));
  }
  return ys;
}

/** Picks share the summary geometry that was painted, then require a separately
 * fetched exact known value. A display envelope never establishes a balance. */
export function pickSummaryLines(lines: readonly { key: string; summary: ChartSummary; held?: boolean }[], hour: number,
  known: ReadonlySet<string>, context?: ChartPick): string[] {
  if (!context?.frame || hour < Math.floor(context.frame.x[0]) || hour > Math.min(context.frame.x[1], context.until ?? Infinity)) return [];
  const frame = context.frame, distances = new Map<string, number>();
  const x = context.point?.x ?? frame.toX(Math.max(frame.x[0], hour));
  const targetY = context.point?.y ?? frame.plotTop + frame.plotHeight / 2;
  for (const line of lines) {
    if (!known.has(line.key)) continue;
    const runs = exactRuns(line.summary);
    if (runs && !runAt(runs, hour)) continue;
    const ys: number[] = [];
    const shape = !runs && summaryFits(frame, line.summary, context.until) ? summaryShape(line.summary, line.held ?? false) : null;
    if (shape && placeable(frame, shape)) {
      const dots = shape.dots;
      ys.push(...shapeOrdinates(frame, shape, x, targetY));
      for (let at = 2 * lower(dots.length / 2, index => frame.toX(dots[2 * index]) < x - 1); at < dots.length && frame.toX(dots[at]) <= x + 1; at += 2)
        ys.push(frame.toY(dots[at + 1]));
    } else {
      const geometry = summaryGeometry(frame, line.summary, line.held ?? false, context.until);
      ys.push(...geometry.paths.flatMap(path => ordinates(path, x, targetY)));
      for (const point of geometry.dots) if (Math.abs(point.x - x) <= 1) ys.push(point.y);
    }
    const visible = ys.filter(y => Number.isFinite(y) && y >= frame.plotTop && y <= frame.plotTop + frame.plotHeight);
    if (!visible.length) continue;
    distances.set(line.key, context.point ? visible.reduce((distance, y) => Math.min(distance, Math.abs(y - context.point!.y)), Infinity) : 0);
  }
  if (!context.point) return [...distances.keys()];
  let nearest = Infinity;
  for (const distance of distances.values()) nearest = Math.min(nearest, distance);
  return [...distances].filter(([, distance]) => distance <= nearest + 2).map(([key]) => key);
}

/** Dense shading uses the two bucket envelopes; exact windows keep the curve fill. */
export function drawGapSummary(ctx: CanvasRenderingContext2D, frame: PlotFrame, _tokens: ChartTokens,
  first: ChartSummary, second: ChartSummary, aboveColor: string, belowColor: string,
  until: number | null = null, options: { alpha?: number } = {}): void {
  if (!summaryFits(frame, first, until) || !summaryFits(frame, second, until)) return;
  const approximate = [...(first.approximate ?? []), ...(second.approximate ?? [])];
  if (approximate.length) {
    drawApproximateSpans(ctx, frame, approximate, until, approximate => {
      drawGapSummary(ctx, frame, _tokens, { ...first, approximate: undefined }, { ...second, approximate: undefined },
        aboveColor, belowColor, until, { ...options, alpha: (options.alpha ?? 0.16) * (approximate ? 0.55 : 1) });
    });
    return;
  }
  const firstRuns = exactRuns(first), secondRuns = exactRuns(second);
  if (firstRuns && secondRuns) {
    drawGapFill(ctx, frame, firstRuns, secondRuns, aboveColor, belowColor, { ...options, until: until ?? undefined });
    return;
  }
  ctx.save(); ctx.globalAlpha = options.alpha ?? 0.16;
  for (let index = 0; index < first.buckets.length; index++) {
    const a = first.buckets[index], b = second.buckets[index];
    if (!a || !b || a.hasBreak || b.hasBreak) continue;
    const start = Math.max(frame.x[0], a.firstHour, b.firstHour);
    const end = Math.min(first.to, a.lastHour + 1, b.lastHour + 1);
    if (end <= start) continue;
    const paint = (low: number, high: number, color: string) => {
      if (low >= high) return;
      const top = frame.toY(high), bottom = frame.toY(low);
      if (!Number.isFinite(top) || !Number.isFinite(bottom)) return;
      ctx.fillStyle = color;
      ctx.fillRect(frame.toX(start), top, frame.toX(end) - frame.toX(start), bottom - top);
    };
    paint(b.minValue, a.maxValue, aboveColor);
    paint(a.minValue, b.maxValue, belowColor);
  }
  ctx.restore();
}

/** Join dated aliases in each display column without filling unknown intervals. */
export function mergeSummaries(parts: readonly ChartSummary[]): ChartSummary {
  if (!parts.length) throw new Error('A chart summary join needs a viewport.');
  const view = parts[0];
  if (parts.some(part => part.from !== view.from || part.to !== view.to || part.width !== view.width || part.buckets.length !== view.buckets.length))
    throw new Error('Chart summary aliases belong to different viewports.');
  const buckets = view.buckets.map((_bucket, column) => {
    const recorded = parts.flatMap(part => part.buckets[column] ? [part.buckets[column]!] : []).sort((a, b) => a.firstHour - b.firstHour);
    let merged: ChartBucket | null = null;
    for (const next of recorded) {
      if (!merged) { merged = { ...next }; continue; }
      if (merged.lastHour >= next.firstHour) throw new Error('Chart summary aliases overlap.');
      // A break after next's one stretch is a coverage gap: the value still began in merged.
      const same: boolean = next.lastValueHour === next.firstHour && !next.firstBreak && merged.lastHour + 1 === next.firstHour && next.minValue === next.maxValue && merged.lastValue === next.firstValue;
      const minimum: ChartBucket = merged.minValue <= next.minValue ? merged : next;
      const maximum: ChartBucket = merged.maxValue >= next.maxValue ? merged : next;
      merged = { ...merged, lastHour: next.lastHour, lastValue: next.lastValue,
        lastValueHour: same ? merged.lastValueHour : next.lastValueHour,
        minHour: minimum.minHour, minValue: minimum.minValue, maxHour: maximum.maxHour, maxValue: maximum.maxValue,
        hasBreak: merged.hasBreak || next.hasBreak || next.firstBreak || merged.lastHour + 1 !== next.firstHour };
    }
    return merged;
  });
  const exact = parts.every(part => exactRuns(part) !== null);
  const runs = exact ? parts.flatMap(part => [...exactRuns(part)!]).sort((a, b) => a.firstHour - b.firstHour) : undefined;
  const approximate = clipToSpans([{ firstHour: Math.floor(view.from), lastHour: Math.floor(view.to) }],
    parts.flatMap(part => part.approximate ?? []));
  return { from: view.from, to: view.to, width: view.width, buckets, ...(runs ? { runs } : {}),
    ...(approximate.length ? { approximate } : {}) };
}

/** Approved D-12 geometry: each series' chronological extrema occupy a common
 * bounded grid. Selected sums and Other use those samples. Exact values never
 * come from this geometry. The existing signed stack builds and draws its bands. */
export function summaryStack(parts: readonly ChartSummary[], total?: ChartSummary) {
  const summaries = total ? [...parts,total] : parts;
  const approximate = approximateUnion(summaries.flatMap(summary => summary.approximate ?? []));
  const samples = (bucket: ChartBucket) => [...new Map([
    [bucket.firstHour,bucket.firstValue],[bucket.minHour,bucket.minValue],
    [bucket.maxHour,bucket.maxValue],[bucket.lastValueHour,bucket.lastValue],
    [bucket.lastHour,bucket.lastValue],
  ]).entries()].sort(([a],[b])=>a-b).map(([,value])=>value);
  // Drawing samples every screen point, so each bucket's samples are built once.
  const sampled = summaries.map(summary => summary.buckets.map(bucket => bucket ? samples(bucket) : null));
  const points: {firstHour:number;lastHour:number;column:number;stage:number;count:number;breakBefore:boolean}[]=[];
  for (let column=0;column<(summaries[0]?.buckets.length ?? 0);column++) {
    const buckets=summaries.map(summary=>summary.buckets[column]);
    if (buckets.some(bucket=>!bucket)) continue;
    // Only the hours every part was recorded: a part never stands in for hours it lacks.
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
  // Only emitted screen points allocate band edges; no country-by-date matrix.
  const sample=(hour:number)=> {
    let low=0,high=points.length;
    while (low<high) {const middle=(low+high)>>1;if(points[middle].firstHour<=hour)low=middle+1;else high=middle;}
    return stackEdges(valuesAt(points[Math.max(0,low-1)]));
  };
  return {model,sample,approximate};
}
