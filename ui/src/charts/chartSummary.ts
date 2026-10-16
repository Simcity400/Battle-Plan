import type { PlotFrame } from './frame.ts';
import type { ChartTokens } from './tokens.ts';
import { drawGapFill, drawLine, tracePath } from './draw.ts';
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
}

export function isChartSummary(value: unknown): value is ChartSummary {
  if (!value || typeof value !== 'object') return false;
  const summary = value as ChartSummary;
  return Number.isFinite(summary.from) && Number.isFinite(summary.to) && summary.from <= summary.to
    && Number.isSafeInteger(summary.width) && summary.width > 0
    && (summary.runs === undefined || isOrderedSpans(summary.runs) && summary.runs.length <= 2 * summary.width + 2
      && summary.runs.every(run => Number.isFinite(run.value) && typeof run.breakBefore === 'boolean'))
    && (summary.approximate === undefined || isOrderedSpans(summary.approximate) && summary.approximate.every(span => Number.isSafeInteger(span.firstHour) && Number.isSafeInteger(span.lastHour)))
    && Array.isArray(summary.buckets) && summary.buckets.length <= 2 * summary.width + 2
    && summary.buckets.every(bucket => bucket === null || bucket && Number.isSafeInteger(bucket.firstHour) && Number.isSafeInteger(bucket.lastHour)
      && bucket.firstHour <= bucket.lastHour && [bucket.minHour, bucket.maxHour, bucket.lastValueHour].every(hour => Number.isSafeInteger(hour)
        && hour >= bucket.firstHour && hour <= bucket.lastHour)
      && [bucket.firstValue, bucket.lastValue, bucket.minValue, bucket.maxValue].every(Number.isFinite)
      && typeof bucket.firstBreak === 'boolean' && typeof bucket.hasBreak === 'boolean');
}

/** Bucket extrema alone lose supplying bounds and equal plotted-value boundaries. */
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
  let low = Infinity, high = -Infinity;
  for (const bucket of summary.buckets) if (bucket) { low = Math.min(low, bucket.minValue); high = Math.max(high, bucket.maxValue); }
  return low <= high ? [low, high] : null;
}

const summaryMatches = (frame: PlotFrame, summary: ChartSummary, until: number | null) => summary.from === frame.x[0]
  && summary.to === Math.min(frame.x[1], until ?? frame.x[1]) && summary.width === Math.ceil(frame.plotWidth);

/** A bounded summary never substitutes for exact picks, hover or keyboard reads.
 * Broken columns show separate extrema dots: no stroke crosses an unknown span.
 * Continuous columns retain the time order of extrema and held step direction. */
export function summaryGeometry(frame: PlotFrame, summary: ChartSummary, held = false, until: number | null = null):
  { paths: PathCommand[][]; dots: Point[]; end: Point | null } {
  // A retained reply belongs to its requested view; awaiting a new view cannot
  // stretch old geometry into the new range or reveal data after its playhead.
  const paths: PathCommand[][] = [], dots: Point[] = [];
  if (!summaryMatches(frame, summary, until)) return { paths, dots, end: null };
  const runs = exactRuns(summary);
  if (runs) {
    const segments = (held ? heldSegments : curveSegments)(runs, frame.x[0], summary.to, frame.toX, frame.toY);
    return { paths: segments.map(segment => segment.path), dots, end: segments.at(-1)?.points.at(-1) ?? null };
  }
  let previous: ChartBucket | null = null;
  let end: { x: number; y: number } | null = null;
  for (const bucket of summary.buckets) {
    if (!bucket) { previous = null; continue; }
    const points = [
      { hour: bucket.firstHour, value: bucket.firstValue, order: 0 },
      { hour: bucket.minHour, value: bucket.minValue, order: 1 },
      { hour: bucket.maxHour, value: bucket.maxValue, order: 2 },
      { hour: bucket.lastValueHour, value: bucket.lastValue, order: 3 },
    ].sort((a, b) => a.hour - b.hour || a.order - b.order);
    if (bucket.hasBreak || points.some(point => !Number.isFinite(frame.toY(point.value)))) {
      for (const point of points) {
        if (Number.isFinite(frame.toY(point.value))) dots.push({ x: frame.toX(point.hour), y: frame.toY(point.value) });
      }
      previous = null;
    } else {
      const path: PathCommand[] = [];
      const push = (move: boolean, hour: number, value: number) => path.push([move ? 'M' : 'L', frame.toX(hour), frame.toY(value)]);
      const joins = previous && !bucket.firstBreak && previous.lastHour + 1 === bucket.firstHour;
      if (joins) {
        push(true, previous!.lastHour + (held ? 1 : 0), previous!.lastValue);
        push(false, bucket.firstHour, previous!.lastValue);
        push(false, bucket.firstHour, bucket.firstValue);
      } else push(true, bucket.firstHour, bucket.firstValue);
      let value = bucket.firstValue;
      for (const point of points) {
        if (held) push(false, point.hour, value);
        push(false, point.hour, point.value); value = point.value;
      }
      push(false, Math.min(summary.to, bucket.lastHour + (held ? 1 : 0)), bucket.lastValue);
      paths.push(path); previous = bucket;
    }
    end = { x: frame.toX(Math.min(summary.to, bucket.lastHour + (held ? 1 : 0))), y: frame.toY(bucket.lastValue) };
  }
  return { paths, dots, end: end && Number.isFinite(end.y) ? end : null };
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

export function drawLineSummary(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens,
  summary: ChartSummary, color: string, held = false, until: number | null = null,
  options: { area?: boolean; dash?: readonly number[]; endDot?: boolean } = {}): Point | null {
  if (!summaryMatches(frame, summary, until)) return null;
  if (summary.approximate?.length) {
    let end: Point | null = null;
    drawApproximateSpans(ctx, frame, summary.approximate, until, approximate => {
      end = drawLineSummary(ctx, frame, tokens, { ...summary, approximate: undefined }, color, held, until,
        approximate ? { ...options, dash: [4, 4], area: false, endDot: false } : options) ?? end;
    });
    return end;
  }
  const runs = exactRuns(summary);
  if (runs) return drawLine(ctx, frame, tokens, runs, color, { ...options, held, until: until ?? undefined });
  const geometry = summaryGeometry(frame, summary, held, until);
  ctx.save(); ctx.strokeStyle = color; ctx.fillStyle = color;
  ctx.lineWidth = tokens.lineWidth.series; ctx.lineJoin = 'round';
  if (options.dash) ctx.setLineDash(options.dash);
  const baseline = frame.toY(frame.log ? frame.y[0] : Math.max(frame.y[0], Math.min(0, frame.y[1])));
  for (const path of geometry.paths) {
    if (options.area) {
      ctx.beginPath(); tracePath(ctx, path);
      const last = path.at(-1)!;
      ctx.lineTo(last[1], baseline); ctx.lineTo(path[0][1], baseline); ctx.closePath();
      ctx.globalAlpha = 0.12; ctx.fill(); ctx.globalAlpha = 1;
    }
    ctx.beginPath(); tracePath(ctx, path); ctx.stroke();
  }
  for (const point of geometry.dots) { ctx.beginPath(); ctx.arc(point.x, point.y, tokens.lineWidth.series / 2, 0, Math.PI * 2); ctx.fill(); }
  if (geometry.end && options.endDot !== false) {
    ctx.beginPath(); ctx.arc(geometry.end.x, geometry.end.y, tokens.lineWidth.series * 2, 0, Math.PI * 2); ctx.fill();
    ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  ctx.restore(); return geometry.end;
}

/** Picks share the summary geometry that was painted, then require a separately
 * fetched exact known value. A display envelope never establishes a balance. */
export function pickSummaryLines(lines: readonly { key: string; summary: ChartSummary; held?: boolean }[], hour: number,
  known: ReadonlySet<string>, context?: ChartPick): string[] {
  if (!context?.frame || hour < Math.floor(context.frame.x[0]) || hour > Math.min(context.frame.x[1], context.until ?? Infinity)) return [];
  const frame = context.frame, distances = new Map<string, number>();
  const x = context.point?.x ?? frame.toX(Math.max(frame.x[0], hour));
  for (const line of lines) {
    if (!known.has(line.key)) continue;
    const runs = exactRuns(line.summary);
    if (runs && !runAt(runs, hour)) continue;
    const geometry = summaryGeometry(frame, line.summary, line.held ?? false, context.until);
    const ys = geometry.paths.flatMap(path => ordinates(path, x, context.point?.y ?? frame.plotTop + frame.plotHeight / 2));
    for (const point of geometry.dots) if (Math.abs(point.x - x) <= 1) ys.push(point.y);
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
  if (!summaryMatches(frame, first, until) || !summaryMatches(frame, second, until)) return;
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
      const same: boolean = !next.hasBreak && !next.firstBreak && merged.lastHour + 1 === next.firstHour && next.minValue === next.maxValue && merged.lastValue === next.firstValue;
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
  const points: {firstHour:number;lastHour:number;column:number;stage:number;count:number;breakBefore:boolean}[]=[];
  for (let column=0;column<(summaries[0]?.buckets.length ?? 0);column++) {
    const buckets=summaries.map(summary=>summary.buckets[column]);
    if (buckets.some(bucket=>!bucket)) continue;
    const count=buckets.reduce((count,bucket)=>Math.max(count,samples(bucket!).length),1);
    const range=buckets.at(-1)!;
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
    const values=summaries.map(summary=> {
      const values=samples(summary.buckets[point.column]!);
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
