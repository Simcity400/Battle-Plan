import type { EventBadge } from './badges.ts';
import {
  buildSummaryShape,
  drawApproximateSpans,
  drawGapSummary,
  drawSummaryShape,
  summaryFits,
  type BucketStreamReader,
  type ChartBucket,
  type ChartSummary,
  type SummaryLineOptions,
  type SummaryShape,
} from './chartSummary.ts';
import { drawLine, drawValueRule, type LineEnd } from './draw.ts';
import { plotFrame, type Padding, type PlotFrame } from './frame.ts';
import type { LostSpan } from './gaps.ts';
import type { MarkSummary } from './markSummary.ts';
import {
  drawTimeChartScene,
  filterEndLabelsByRoom,
  formatEndLabel,
  scalePlotFrameY,
  type ChartAxisFormatterId,
  type ChartLayer,
  type DeclarativeEndLabels,
} from './renderTimeChart.ts';
import type { Run } from './series.ts';
import { sizeCanvas } from './surface.ts';
import { canvasMeasureText, clearTextWidthCache, type Canvas2D, type MeasureText } from './textWidth.ts';
import type { ChartTokens } from './tokens.ts';
import type { PlaybackMode, Range } from './view.ts';

const FLAG_PRESENT = 1;
const FLAG_FIRST_BREAK = 2;
const FLAG_HAS_BREAK = 4;

export interface PackedChartSummary {
  readonly rev: number;
  readonly from: number;
  readonly to: number;
  readonly width: number;
  readonly binHours: number | null;
  readonly bucketCount: number;
  readonly flags: Uint8Array;
  readonly bucketData: Float64Array;
  readonly runs?: Float64Array;
  readonly approximate?: Float64Array;
}

export interface WireMarkSummary {
  readonly rev: number;
  readonly summary: MarkSummary;
}

export interface FrameSpec {
  readonly width: number;
  readonly height: number;
  readonly padding: Padding;
  readonly x: Range;
  readonly y: Range;
  readonly log: boolean;
  readonly strip: number;
  readonly timeAxisTop?: boolean;
  readonly valueTicks?: readonly number[];
}

export type WorkerLayerRef =
  | {
      readonly kind: 'line-summary';
      readonly summaryRev: number | null;
      readonly color: string;
      readonly held: boolean;
      readonly options?: SummaryLineOptions;
      readonly yScale?: number;
      readonly gaps?: readonly LostSpan[];
    }
  | {
      readonly kind: 'gap-summary';
      readonly firstRev: number | null;
      readonly secondRev: number | null;
      readonly aboveColor: string;
      readonly belowColor: string;
      readonly alpha?: number;
      readonly gaps?: readonly LostSpan[];
    }
  | {
      readonly kind: 'value-rule';
      readonly value: number;
      readonly gaps?: readonly LostSpan[];
    };

export interface WorkerFontFace {
  readonly family: string;
  readonly source: string;
  readonly descriptors?: FontFaceDescriptors;
}

export interface WorkerCanvasLike {
  width: number;
  height: number;
  getContext(contextId: '2d'): Canvas2D | null;
}

export interface DrawMessage {
  readonly type: 'draw';
  readonly id: number;
  readonly backingWidth: number;
  readonly backingHeight: number;
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly scaleX: number;
  readonly scaleY: number;
  readonly frame: FrameSpec;
  readonly layers: readonly WorkerLayerRef[];
  readonly headValues: readonly (number | null)[];
  readonly endHeadValues?: readonly (number | null)[];
  readonly gaps?: readonly LostSpan[];
  readonly playhead: number | null;
  readonly playback: PlaybackMode;
  readonly afterPlayhead?: boolean;
  readonly until: number | null;
  readonly markRevs?: readonly number[];
  readonly badges?: readonly EventBadge[];
  readonly rowLabels?: readonly string[];
  readonly formatValue?: ChartAxisFormatterId;
  readonly endLabels?: DeclarativeEndLabels;
}

export type ToTimeChartWorker =
  | { readonly type: 'attach'; readonly id: number; readonly canvas: WorkerCanvasLike }
  | { readonly type: 'detach'; readonly id: number }
  | { readonly type: 'tokens'; readonly tokens: ChartTokens }
  | { readonly type: 'rate'; readonly rate: number }
  | { readonly type: 'fonts'; readonly faces: readonly WorkerFontFace[] }
  | {
      readonly type: 'data';
      readonly id: number;
      readonly summaries?: readonly PackedChartSummary[];
      readonly marks?: readonly WireMarkSummary[];
      readonly keepSummaries?: readonly number[];
      readonly keepMarks?: readonly number[];
    }
  | DrawMessage;

export type FromTimeChartWorker =
  | { readonly type: 'error'; readonly reason: string };

export function packChartSummary(rev: number, summary: ChartSummary): { packed: PackedChartSummary; transfer: Transferable[] } {
  const buckets = summary.buckets;
  const bucketCount = buckets.length;
  const flags = new Uint8Array(bucketCount);
  let present = 0;
  for (let i = 0; i < bucketCount; i++) {
    const bucket = buckets[i];
    if (!bucket) continue;
    flags[i] = FLAG_PRESENT | (bucket.firstBreak ? FLAG_FIRST_BREAK : 0) | (bucket.hasBreak ? FLAG_HAS_BREAK : 0);
    present++;
  }
  const bucketData = new Float64Array(present * 9);
  let at = 0;
  for (let i = 0; i < bucketCount; i++) {
    const bucket = buckets[i];
    if (!bucket) continue;
    bucketData[at++] = bucket.firstHour;
    bucketData[at++] = bucket.lastHour;
    bucketData[at++] = bucket.lastValueHour;
    bucketData[at++] = bucket.minHour;
    bucketData[at++] = bucket.maxHour;
    bucketData[at++] = bucket.firstValue;
    bucketData[at++] = bucket.lastValue;
    bucketData[at++] = bucket.minValue;
    bucketData[at++] = bucket.maxValue;
  }
  const transfer: Transferable[] = [flags.buffer, bucketData.buffer];
  let runs: Float64Array | undefined;
  if (summary.runs) {
    runs = new Float64Array(summary.runs.length * 4);
    for (let i = 0; i < summary.runs.length; i++) {
      const run = summary.runs[i]!;
      runs[i * 4] = run.firstHour;
      runs[i * 4 + 1] = run.lastHour;
      runs[i * 4 + 2] = run.value;
      runs[i * 4 + 3] = run.breakBefore ? 1 : 0;
    }
    transfer.push(runs.buffer);
  }
  let approximate: Float64Array | undefined;
  if (summary.approximate && summary.approximate.length > 0) {
    approximate = new Float64Array(summary.approximate.length * 2);
    for (let i = 0; i < summary.approximate.length; i++) {
      const span = summary.approximate[i]!;
      approximate[i * 2] = span.firstHour;
      approximate[i * 2 + 1] = span.lastHour;
    }
    transfer.push(approximate.buffer);
  }
  return {
    packed: {
      rev,
      from: summary.from,
      to: summary.to,
      width: summary.width,
      binHours: summary.tiles ? summary.tiles.binHours : null,
      bucketCount,
      flags,
      bucketData,
      ...(runs ? { runs } : {}),
      ...(approximate ? { approximate } : {}),
    },
    transfer,
  };
}

interface RetainedSummary {
  readonly packed: PackedChartSummary;
  readonly offsets: Int32Array;
  readonly stream: BucketStreamReader;
  readonly runs: Run[] | null;
  readonly approximate: { firstHour: number; lastHour: number }[] | undefined;
  shapes: [SummaryShape | undefined, SummaryShape | undefined];
  buckets?: (ChartBucket | null)[];
}

function retainSummary(packed: PackedChartSummary): RetainedSummary {
  const { bucketCount, flags, bucketData } = packed;
  const offsets = new Int32Array(bucketCount);
  let pos = 0;
  for (let i = 0; i < bucketCount; i++) {
    if (flags[i]! & FLAG_PRESENT) {
      offsets[i] = pos;
      pos += 9;
    } else {
      offsets[i] = -1;
    }
  }
  const stream: BucketStreamReader = {
    count: bucketCount,
    present: i => offsets[i]! >= 0,
    firstHour: i => bucketData[offsets[i]!]!,
    lastHour: i => bucketData[offsets[i]! + 1]!,
    lastValueHour: i => bucketData[offsets[i]! + 2]!,
    minHour: i => bucketData[offsets[i]! + 3]!,
    maxHour: i => bucketData[offsets[i]! + 4]!,
    firstValue: i => bucketData[offsets[i]! + 5]!,
    lastValue: i => bucketData[offsets[i]! + 6]!,
    minValue: i => bucketData[offsets[i]! + 7]!,
    maxValue: i => bucketData[offsets[i]! + 8]!,
    firstBreak: i => (flags[i]! & FLAG_FIRST_BREAK) !== 0,
    hasBreak: i => (flags[i]! & FLAG_HAS_BREAK) !== 0,
  };
  let runs: Run[] | null = null;
  if (packed.runs) {
    const count = packed.runs.length / 4;
    runs = new Array<Run>(count);
    for (let i = 0; i < count; i++) {
      runs[i] = {
        firstHour: packed.runs[i * 4]!,
        lastHour: packed.runs[i * 4 + 1]!,
        value: packed.runs[i * 4 + 2]!,
        breakBefore: packed.runs[i * 4 + 3]! !== 0,
      };
    }
  }
  let approximate: { firstHour: number; lastHour: number }[] | undefined;
  if (packed.approximate && packed.approximate.length > 0) {
    const count = packed.approximate.length / 2;
    approximate = new Array<{ firstHour: number; lastHour: number }>(count);
    for (let i = 0; i < count; i++) {
      approximate[i] = { firstHour: packed.approximate[i * 2]!, lastHour: packed.approximate[i * 2 + 1]! };
    }
  }
  return { packed, offsets, stream, runs, approximate, shapes: [undefined, undefined] };
}

function retainedShape(retained: RetainedSummary, held: boolean): SummaryShape {
  const index = held ? 1 : 0;
  let shape = retained.shapes[index];
  if (!shape) {
    shape = buildSummaryShape(retained.stream, held, retained.packed.binHours !== null ? Infinity : retained.packed.to);
    retained.shapes[index] = shape;
  }
  return shape;
}

function retainedBuckets(retained: RetainedSummary): (ChartBucket | null)[] {
  if (retained.buckets) return retained.buckets;
  const { bucketCount, flags, bucketData } = retained.packed;
  const out = new Array<ChartBucket | null>(bucketCount);
  for (let i = 0; i < bucketCount; i++) {
    const at = retained.offsets[i]!;
    if (at < 0) { out[i] = null; continue; }
    out[i] = {
      firstHour: bucketData[at]!,
      lastHour: bucketData[at + 1]!,
      lastValueHour: bucketData[at + 2]!,
      minHour: bucketData[at + 3]!,
      maxHour: bucketData[at + 4]!,
      firstValue: bucketData[at + 5]!,
      lastValue: bucketData[at + 6]!,
      minValue: bucketData[at + 7]!,
      maxValue: bucketData[at + 8]!,
      firstBreak: (flags[i]! & FLAG_FIRST_BREAK) !== 0,
      hasBreak: (flags[i]! & FLAG_HAS_BREAK) !== 0,
    };
  }
  retained.buckets = out;
  return out;
}

function retainedAsChartSummary(retained: RetainedSummary): ChartSummary {
  const { from, to, width, binHours } = retained.packed;
  return {
    from,
    to,
    width,
    buckets: retainedBuckets(retained),
    ...(retained.runs ? { runs: retained.runs } : {}),
    ...(retained.approximate ? { approximate: retained.approximate } : {}),
    ...(binHours !== null ? { tiles: { binHours, valueAt: () => null } } : {}),
  };
}

function retainedEndValue(retained: RetainedSummary, at: number, headValue: number | null): number | null {
  const { bucketCount, bucketData, binHours } = retained.packed;
  for (let index = bucketCount - 1; index >= 0; index--) {
    const pos = retained.offsets[index]!;
    if (pos < 0 || bucketData[pos]! > at) continue;
    const lastHour = bucketData[pos + 1]!;
    return lastHour >= at && binHours !== null && !retained.runs ? headValue : bucketData[pos + 6]!;
  }
  return null;
}

function drawRetainedLine(
  ctx: Canvas2D,
  frame: PlotFrame,
  tokens: ChartTokens,
  retained: RetainedSummary,
  color: string,
  held: boolean,
  until: number | null,
  options: SummaryLineOptions | undefined,
  headValue: number | null,
): void {
  const { packed, runs, approximate } = retained;
  const fits = summaryFits(frame, { from: packed.from, to: packed.to, width: packed.width, tiles: packed.binHours !== null ? true : undefined }, until);
  if (!fits) return;
  const opts = options ?? {};
  if (packed.binHours !== null && !runs) {
    const end = until ?? frame.x[1];
    const cut = Math.floor(end / packed.binHours) * packed.binHours;
    const shape = retainedShape(retained, held);
    drawSummaryShape(ctx, frame, tokens, shape, color, opts, [frame.x[0], cut], { stream: retained.stream, held, to: packed.to });
    const y = headValue === null ? NaN : frame.toY(headValue);
    if (!Number.isFinite(y)) return;
    ctx.save();
    ctx.strokeStyle = color;
    ctx.fillStyle = color;
    ctx.lineWidth = tokens.lineWidth.series;
    ctx.lineJoin = 'round';
    if (opts.dash) ctx.setLineDash(opts.dash);
    const baseline = frame.toY(frame.log ? frame.y[0] : Math.max(frame.y[0], Math.min(0, frame.y[1])));
    const left = frame.toX(Math.max(cut - 1, frame.x[0]));
    const right = frame.toX(Math.min(end, frame.x[1]));
    if (right < left) { ctx.restore(); return; }
    if (opts.area) {
      ctx.globalAlpha = 0.12;
      ctx.fillRect(left, Math.min(y, baseline), right - left, Math.abs(baseline - y));
      ctx.globalAlpha = 1;
    }
    ctx.beginPath();
    ctx.moveTo(left, y);
    ctx.lineTo(right, y);
    ctx.stroke();
    if (opts.endDot !== false) {
      ctx.beginPath();
      ctx.arc(right, y, tokens.lineWidth.series * 2, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = tokens.surface;
      ctx.stroke();
    }
    ctx.restore();
    return;
  }
  const shape = runs ? null : retainedShape(retained, held);
  const paint = (drawn: SummaryLineOptions) => {
    if (!shape) drawLine(ctx, frame, tokens, runs!, color, { ...drawn, held, until: until ?? undefined });
    else drawSummaryShape(ctx, frame, tokens, shape, color, drawn, undefined, { stream: retained.stream, held, to: packed.to });
  };
  if (!approximate?.length) { paint(opts); return; }
  drawApproximateSpans(ctx, frame, approximate, until, isApproximate => {
    paint(isApproximate ? { ...opts, dash: [4, 4], area: false, endDot: false } : opts);
  });
}

export interface TimeChartWorkerHost {
  schedule(run: () => void): void;
  clock?: () => number;
  stretch?: (workMs: number, rate: number) => void;
  loadFonts?: (faces: readonly WorkerFontFace[]) => Promise<boolean>;
  onError?: (reason: string) => void;
}

interface ChartSlot {
  readonly id: number;
  readonly canvas: WorkerCanvasLike;
  readonly ctx: Canvas2D;
  readonly measure: MeasureText;
  readonly summaries: Map<number, RetainedSummary>;
  readonly marks: Map<number, MarkSummary>;
  pending: DrawMessage | null;
  lastDrawn: DrawMessage | null;
}

export interface TimeChartWorkerHandler {
  handle(message: ToTimeChartWorker): void;
  drain(): void;
  readonly attachedCount: number;
}

export function createTimeChartWorkerHandler(host: TimeChartWorkerHost): TimeChartWorkerHandler {
  const slots = new Map<number, ChartSlot>();
  let tokens: ChartTokens | null = null;
  let rate = 1;
  let scheduled = false;

  const scheduleDrain = () => {
    if (scheduled) return;
    scheduled = true;
    host.schedule(() => {
      scheduled = false;
      drain();
    });
  };

  const paintSlot = (slot: ChartSlot, job: DrawMessage) => {
    if (!tokens) return;
    const clock = host.clock ?? (() => performance.now());
    const started = rate > 1 ? clock() : 0;
    sizeCanvas(slot.canvas as unknown as OffscreenCanvas, job.backingWidth, job.backingHeight);
    const frame = plotFrame(
      job.frame.width,
      job.frame.height,
      job.frame.padding,
      job.frame.x,
      job.frame.y,
      job.frame.log,
      job.frame.strip,
      job.frame.timeAxisTop,
    );
    frame.valueTicks = job.frame.valueTicks;

    const layers: ChartLayer[] = job.layers.map((ref, index) => {
      if (ref.kind === 'value-rule') {
        return {
          gaps: ref.gaps,
          draw(ctx, drawFrame, drawTokens) { drawValueRule(ctx, drawFrame, drawTokens, ref.value); },
        };
      }
      if (ref.kind === 'gap-summary') {
        return {
          gaps: ref.gaps,
          draw(ctx, drawFrame, drawTokens, state) {
            const first = ref.firstRev !== null ? slot.summaries.get(ref.firstRev) : undefined;
            const second = ref.secondRev !== null ? slot.summaries.get(ref.secondRev) : undefined;
            if (first && second) {
              drawGapSummary(
                ctx,
                drawFrame,
                drawTokens,
                retainedAsChartSummary(first),
                retainedAsChartSummary(second),
                ref.aboveColor,
                ref.belowColor,
                state.playhead,
                ref.alpha === undefined ? undefined : { alpha: ref.alpha },
              );
            }
          },
        };
      }
      const headValue = job.headValues[index] ?? null;
      const scale = ref.yScale ?? 1;
      return {
        gaps: ref.gaps,
        draw(ctx, drawFrame, drawTokens, state) {
          if (ref.summaryRev === null) return;
          const retained = slot.summaries.get(ref.summaryRev);
          if (!retained) return;
          const targetFrame = scale === 1 ? drawFrame : scalePlotFrameY(drawFrame, scale);
          drawRetainedLine(ctx, targetFrame, drawTokens, retained, ref.color, ref.held, state.playhead, ref.options, headValue);
        },
      };
    });

    const markSummaries = job.markRevs
      ? job.markRevs.flatMap(rev => {
          const mark = slot.marks.get(rev);
          return mark ? [mark] : [];
        })
      : undefined;

    let endLabels: LineEnd[] | undefined;
    if (job.endLabels) {
      const spec = job.endLabels;
      const maxHour = spec.extentMax ?? frame.x[1];
      const endHour = Math.min(job.until ?? maxHour, frame.x[1], maxHour);
      if (!(spec.overflow === 'filter-then-hide' && endHour < frame.x[0])) {
        const ends: LineEnd[] = [];
        for (const item of spec.items) {
          const ref = job.layers[item.layerIndex];
          if (!ref || ref.kind !== 'line-summary' || ref.summaryRev === null) continue;
          const retained = slot.summaries.get(ref.summaryRev);
          if (!retained) continue;
          if (item.format.kind === 'static' && item.format.requireFit) {
            const p = retained.packed;
            if (!summaryFits(frame, { from: p.from, to: p.to, width: p.width, tiles: p.binHours !== null ? true : undefined }, job.until)) continue;
          }
          const headValue = (job.endHeadValues ?? job.headValues)[item.layerIndex] ?? null;
          const value = retainedEndValue(retained, endHour, headValue);
          if (value == null) continue;
          const formatted = formatEndLabel(item, value, ref.yScale ?? 1);
          const y = frame.toY(formatted.yValue);
          if (!Number.isFinite(y)) continue;
          ends.push({ id: item.id, y, text: formatted.text, suffix: formatted.suffix, color: item.color, muted: item.muted });
        }
        const filtered = filterEndLabelsByRoom(ends, spec.items, frame.plotHeight, spec.gap ?? 2, spec.overflow);
        if (filtered.length > 0) endLabels = filtered;
      }
    }

    drawTimeChartScene(
      slot.ctx,
      frame,
      tokens,
      {
        layoutWidth: job.layoutWidth,
        layoutHeight: job.layoutHeight,
        scaleX: job.scaleX,
        scaleY: job.scaleY,
        layers,
        gaps: job.gaps,
        playhead: job.playhead,
        playback: job.playback,
        afterPlayhead: job.afterPlayhead,
        until: job.until,
        markSummaries,
        markSummariesMatched: true,
        badges: job.badges,
        rowLabels: job.rowLabels,
        formatValue: job.formatValue,
        endLabels,
        endLabelGap: job.endLabels?.gap,
      },
      slot.measure,
    );
    slot.lastDrawn = job;
    if (rate > 1 && host.stretch) {
      host.stretch(clock() - started, rate);
    }
  };

  const drain = () => {
    for (const slot of slots.values()) {
      const job = slot.pending;
      if (!job) continue;
      slot.pending = null;
      try {
        paintSlot(slot, job);
      } catch (error) {
        host.onError?.(error instanceof Error ? error.message : String(error));
      }
    }
  };

  const handle = (message: ToTimeChartWorker) => {
    switch (message.type) {
      case 'attach': {
        const ctx = message.canvas.getContext('2d');
        if (!ctx) {
          host.onError?.('Time chart worker could not acquire a 2D context for the transferred canvas.');
          return;
        }
        slots.set(message.id, {
          id: message.id,
          canvas: message.canvas,
          ctx,
          measure: canvasMeasureText(ctx),
          summaries: new Map(),
          marks: new Map(),
          pending: null,
          lastDrawn: null,
        });
        break;
      }
      case 'detach': {
        slots.delete(message.id);
        break;
      }
      case 'tokens': {
        tokens = message.tokens;
        break;
      }
      case 'rate': {
        rate = Number.isFinite(message.rate) && message.rate > 1 ? message.rate : 1;
        break;
      }
      case 'fonts': {
        if (!host.loadFonts || !message.faces.length) break;
        void host.loadFonts(message.faces).then(loaded => {
          if (!loaded) return;
          clearTextWidthCache();
          let any = false;
          for (const slot of slots.values()) {
            if (!slot.pending && slot.lastDrawn) {
              slot.pending = slot.lastDrawn;
              any = true;
            }
          }
          if (any) scheduleDrain();
        });
        break;
      }
      case 'data': {
        const slot = slots.get(message.id);
        if (!slot) break;
        if (message.summaries) {
          for (const packed of message.summaries) slot.summaries.set(packed.rev, retainSummary(packed));
        }
        if (message.marks) {
          for (const entry of message.marks) slot.marks.set(entry.rev, entry.summary);
        }
        if (message.keepSummaries) {
          const keep = new Set(message.keepSummaries);
          for (const rev of slot.summaries.keys()) if (!keep.has(rev)) slot.summaries.delete(rev);
        }
        if (message.keepMarks) {
          const keep = new Set(message.keepMarks);
          for (const rev of slot.marks.keys()) if (!keep.has(rev)) slot.marks.delete(rev);
        }
        break;
      }
      case 'draw': {
        const slot = slots.get(message.id);
        if (!slot) break;
        slot.pending = message;
        scheduleDrain();
        break;
      }
    }
  };

  return {
    handle,
    drain,
    get attachedCount() { return slots.size; },
  };
}
