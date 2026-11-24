import type { HourSpan } from '../../generated/dashboard';
import { hourToX, hoursPerPixel, type TimelineLayout } from './layout.ts';
import type { TimelineLane, TimelineModel, TimelineSpan } from './model.ts';

export interface TimelineResolution {
  readonly model: TimelineModel;
  readonly plotWidth: number;
  readonly bucket: number;
  readonly minHours: number;
  readonly lanes: Map<string, readonly TimelineSpan[]>;
  gaps: readonly HourSpan[] | null;
}

export interface PaintedSpan {
  readonly span: TimelineSpan;
  readonly x0: number;
  readonly x1: number;
}

export function resolutionFor(previous: TimelineResolution | null, model: TimelineModel, layout: TimelineLayout): TimelineResolution {
  const plotWidth = layout.plotRight - layout.plotLeft;
  const bucket = Math.floor(Math.log2(Math.max(1e-6, hoursPerPixel(layout))));
  if (previous && previous.model === model && previous.plotWidth === plotWidth && previous.bucket === bucket) return previous;
  return { model, plotWidth, bucket, minHours: 2 ** bucket, lanes: new Map(), gaps: null };
}

function spanEnd(span: TimelineSpan): number {
  return span.open ? Infinity : span.lastHour + 1;
}

export function coalescedSpans(resolution: TimelineResolution, lane: TimelineLane): readonly TimelineSpan[] {
  const held = resolution.lanes.get(lane.key);
  if (held) return held;
  const out: TimelineSpan[] = [];
  const min = resolution.minHours;
  for (const span of lane.spans) {
    const prev = out.at(-1);
    const tiny = spanEnd(span) - span.firstHour < min;
    if (prev && tiny && !prev.open && span.firstHour - spanEnd(prev) < min && prev.color === span.color) {
      out[out.length - 1] = { ...prev, lastHour: span.lastHour, open: span.open, endUnseen: span.endUnseen };
      continue;
    }
    out.push(span);
  }
  resolution.lanes.set(lane.key, out);
  return out;
}

export function coalescedGaps(resolution: TimelineResolution): readonly HourSpan[] {
  if (resolution.gaps) return resolution.gaps;
  const out: HourSpan[] = [];
  const reach = resolution.minHours * 1.5;
  for (const gap of resolution.model.unobserved) {
    const prev = out.at(-1);
    if (prev && gap.firstHour - (prev.lastHour + 1) < reach) {
      out[out.length - 1] = { firstHour: prev.firstHour, lastHour: Math.max(prev.lastHour, gap.lastHour) };
      continue;
    }
    out.push(gap);
  }
  resolution.gaps = out;
  return out;
}

export function paintedSpans(resolution: TimelineResolution, lane: TimelineLane, layout: TimelineLayout): PaintedSpan[] {
  const out: PaintedSpan[] = [];
  for (const span of coalescedSpans(resolution, lane)) {
    const x0 = Math.max(layout.plotLeft, hourToX(layout, span.firstHour));
    if (x0 >= layout.plotRight) continue;
    const x1 = span.open ? layout.plotRight : Math.min(layout.plotRight, hourToX(layout, span.lastHour + 1));
    if (x1 <= layout.plotLeft) continue;
    out.push({ span, x0, x1: Math.max(x0 + 1.5, x1) });
  }
  return out;
}
