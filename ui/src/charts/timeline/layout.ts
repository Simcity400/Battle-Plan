/** Pure layout for lane timelines: which lane and span a point is over, which
 * span holds a lane at an hour, and where a span's bar runs. */

export interface TimelineSpan {
  id: string;
  /** First and last recorded hour the holder was observed. */
  firstHour: number;
  lastHour: number;
  label: string;
  /** A PNG data URL of the holder's icon or portrait. */
  icon?: string;
  /** 0..1 along a sequential ramp (a law's escalation); none draws the neutral fill. */
  shade?: number;
}

export interface TimelineLane {
  id: string;
  /** The slot's name, under the holder in the left column. */
  label: string;
  spans: readonly TimelineSpan[];
  /** Icons and names inside this lane's bars; the chart's settings otherwise. */
  icons?: boolean;
  names?: boolean;
}

export const AXIS_HEIGHT = 24;
export const ROW_HEIGHT = 40;

/** The lane under a y position, or null above the lanes or below the last. */
export function laneAt(y: number, count: number): number | null {
  if (y < AXIS_HEIGHT) return null;
  const index = Math.floor((y - AXIS_HEIGHT) / ROW_HEIGHT);
  return index >= 0 && index < count ? index : null;
}

/** The span holding a lane at an hour. A span covers its last observed hour up to
 * the next one, so a daily-sampled holder has no gaps between its samples. */
export function spanAt(spans: readonly TimelineSpan[], hour: number): TimelineSpan | null {
  return spans.find(span => span.firstHour <= hour && hour < span.lastHour + 1) ?? null;
}

/** The hours a span's bar covers: never past the playhead, none when it starts after it. */
export function barHours(span: TimelineSpan, playhead: number | null): [number, number] | null {
  const end = Math.min(span.lastHour + 1, playhead ?? Infinity);
  return span.firstHour < end ? [span.firstHour, end] : null;
}

/** Whether an hour falls in one of the given spans (such as the unobserved hours). */
export function inSpans(spans: readonly { firstHour: number; lastHour: number }[] | undefined, hour: number): boolean {
  return !!spans?.some(span => span.firstHour <= hour && hour <= span.lastHour);
}

/** The left column's width: what its longest text needs, but never more than
 * 40% of the chart, so the bars always keep room. */
export function leftColumn(fitted: number, width: number): number {
  return Math.min(fitted, Math.max(96, Math.floor(width * 0.4)));
}

/** Every span's hours together, for fitting the time axis. */
export function spansExtent(lanes: readonly TimelineLane[]): [number, number] | null {
  let first = Infinity;
  let last = -Infinity;
  for (const lane of lanes) {
    for (const span of lane.spans) {
      first = Math.min(first, span.firstHour);
      last = Math.max(last, span.lastHour + 1);
    }
  }
  return first <= last ? [first, last] : null;
}
