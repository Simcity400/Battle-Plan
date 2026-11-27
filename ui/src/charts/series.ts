
export interface Run { firstHour: number; lastHour: number; value: number; breakBefore?: boolean }

export interface Span { firstHour: number; lastHour: number }

export function isSpan(value: unknown): value is Record<string, unknown> & Span {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const span = value as Record<string, unknown>;
  return typeof span.firstHour === 'number' && Number.isSafeInteger(span.firstHour)
    && typeof span.lastHour === 'number' && Number.isSafeInteger(span.lastHour) && span.firstHour <= span.lastHour;
}

export function isOrderedSpans(value: unknown): value is (Record<string, unknown> & Span)[] {
  if (!Array.isArray(value)) return false;
  let last: number | null = null;
  for (const span of value) {
    if (!isSpan(span) || (last !== null && last >= span.firstHour)) return false;
    last = span.lastHour;
  }
  return true;
}

export type Breakable = Span & { breakBefore?: boolean; startsSegment?: boolean };

export function runIndexAt(runs: readonly Span[], hour: number): number {
  let low = 0;
  let high = runs.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = runs[middle];
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return middle;
  }
  return -1;
}

export function runAt<T extends Span>(runs: readonly T[], hour: number): T | null {
  const index = runIndexAt(runs, hour);
  return index < 0 ? null : runs[index];
}

export function spansCovered(values: readonly Span[], observed: readonly Span[]): boolean {
  let index = 0;
  for (const value of values) {
    while (index < observed.length && observed[index].lastHour < value.firstHour) index++;
    let end = value.firstHour - 1;
    for (let at = index; at < observed.length && end < value.lastHour; at++) {
      const span = observed[at];
      if (span.firstHour > end + 1) break;
      end = span.lastHour;
    }
    if (end < value.lastHour) return false;
  }
  return true;
}

export function clipToSpans<T extends Breakable>(runs: readonly T[], spans: readonly Span[]): T[] {
  const ordered = <S extends Span>(list: readonly S[]) =>
    list.every((item, index) => index === 0 || list[index - 1].firstHour <= item.firstHour) ? list : [...list].sort((a, b) => a.firstHour - b.firstHour);
  const sortedRuns = ordered(runs);
  const merged: Span[] = [];
  for (const span of ordered(spans)) {
    const last = merged.at(-1);
    if (last && span.firstHour <= last.lastHour) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else merged.push({ firstHour: span.firstHour, lastHour: span.lastHour });
  }
  const clipped: T[] = [];
  let runIndex = 0;
  let spanIndex = 0;
  while (runIndex < sortedRuns.length && spanIndex < merged.length) {
    const run = sortedRuns[runIndex];
    const span = merged[spanIndex];
    const first = Math.max(run.firstHour, span.firstHour);
    const last = Math.min(run.lastHour, span.lastHour);
    if (first <= last) {
      const piece: T = { ...run, firstHour: first, lastHour: last };
      if (first !== run.firstHour) {
        if ('breakBefore' in piece) delete piece.breakBefore;
        if ('startsSegment' in piece) piece.startsSegment = false;
      }
      clipped.push(piece);
    }
    if (run.lastHour < span.lastHour) runIndex++;
    else spanIndex++;
  }
  return clipped;
}
