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
