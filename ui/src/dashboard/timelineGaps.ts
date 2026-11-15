import type { RecordingGap } from '../generated/dashboard.ts';

/** Inclusive hour cells, clipped to the shared clock's bounds. Never bridge branches. */
export function timelineGaps(gaps: readonly RecordingGap[], offset: number, first: number, last: number) {
  const span = last - first;
  if (span < 0) return [];
  return gaps.flatMap(gap => {
    const from = Math.max(first, gap.firstHour - offset);
    const to = Math.min(last, gap.lastHour - offset);
    if (from > to) return [];
    // Range inputs place their endpoints at the first/last hour's centres.
    const start = Math.max(first, from - 0.5);
    const end = Math.min(last, to + 0.5);
    return [{ ...gap, left: span ? 100 * (start - first) / span : 0, width: span ? 100 * (end - start) / span : 100 }];
  });
}


/** The clock hour under `clientX` on the timeline's range input (`left`, `width`), whose thumb is 14 px wide: one hour for
 * the pointer's tooltip and its seek alike, so a click goes to the hour its tooltip names whatever the browser rounds its
 * value to (a 13-year recording is about 70 hours a pixel). */
export function timelineHourAt(clientX: number, rect: { left: number; width: number }, first: number, last: number): { left: number; hour: number } {
  const fraction = Math.min(1, Math.max(0, (clientX - rect.left - 7) / Math.max(1, rect.width - 14)));
  return { left: 7 + fraction * (rect.width - 14), hour: Math.min(last, Math.max(first, Math.round(first + fraction * (last - first)))) };
}
