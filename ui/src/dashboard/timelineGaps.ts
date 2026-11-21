import type { RecordingGap } from '../generated/dashboard.ts';

export function timelineGaps(gaps: readonly RecordingGap[], offset: number, first: number, last: number) {
  const span = last - first;
  if (span < 0) return [];
  return gaps.flatMap(gap => {
    const from = Math.max(first, gap.firstHour - offset);
    const to = Math.min(last, gap.lastHour - offset);
    if (from > to) return [];
    const start = Math.max(first, from - 0.5);
    const end = Math.min(last, to + 0.5);
    return [{ ...gap, left: span ? 100 * (start - first) / span : 0, width: span ? 100 * (end - start) / span : 100 }];
  });
}


export function timelineHourAt(clientX: number, rect: { left: number; width: number }, first: number, last: number): { left: number; hour: number } {
  const fraction = Math.min(1, Math.max(0, (clientX - rect.left - 7) / Math.max(1, rect.width - 14)));
  return { left: 7 + fraction * (rect.width - 14), hour: Math.min(last, Math.max(first, Math.round(first + fraction * (last - first)))) };
}
