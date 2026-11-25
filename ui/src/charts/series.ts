
export interface Run { firstHour: number; lastHour: number; value: number; breakBefore?: boolean }
export interface Point { x: number; y: number }
export type PathCommand = readonly ['M', number, number] | readonly ['L', number, number] | readonly ['C', number, number, number, number, number, number];

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

export const breaksBefore = (run: Breakable) => run.breakBefore === true || run.startsSegment === true;

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

export function stretchStarts(runs: readonly Breakable[]): number[] {
  const starts: number[] = [];
  runs.forEach((run, index) => {
    const previous = runs[index - 1];
    const continues = previous != null && !breaksBefore(run) && previous.lastHour + 1 === run.firstHour;
    starts.push(continues ? starts[index - 1] : run.firstHour);
  });
  return starts;
}

export function rangeBounds(runs: readonly Span[], from: number, to: number): [number, number] {
  let low = 0, high = runs.length;
  while (low < high) { const mid = (low + high) >>> 1; if (runs[mid].lastHour < Math.floor(from)) low = mid + 1; else high = mid; }
  const first = low;
  high = runs.length;
  while (low < high) { const mid = (low + high) >>> 1; if (!(runs[mid].firstHour > to)) low = mid + 1; else high = mid; }
  return [first, low];
}

export function segments(runs: readonly Run[], from: number, to: number): Run[][] {
  if (!(to >= from)) return [];
  const result: Run[][] = [];
  let current: Run[] = [];
  const first = Math.floor(from);
  const [start, end] = rangeBounds(runs, first, to);
  for (let i = start; i < end; i++) {
    const run = runs[i];
    const clipped = { firstHour: Math.max(run.firstHour, Math.floor(from)), lastHour: Math.min(run.lastHour, Math.ceil(to)), value: run.value };
    const previous = current.at(-1);
    if (previous && (previous.lastHour + 1 !== run.firstHour || run.breakBefore)) { result.push(current); current = []; }
    current.push(clipped);
  }
  if (current.length) result.push(current);
  return result;
}

export function samplePoints(segment: readonly Run[], toX: (hour: number) => number, toY: (value: number) => number, stepPx = 12): Point[] {
  if (!segment.length) return [];
  const first = segment[0].firstHour;
  const last = segment.at(-1)!.lastHour;
  const pixels = Math.abs(toX(last) - toX(first));
  const every = last > first && pixels > 0 ? 2 ** Math.max(0, Math.round(Math.log2(stepPx * (last - first) / pixels))) : 1;
  const hours = [first];
  for (let hour = (Math.floor(first / every) + 1) * every; hour < last; hour += every) hours.push(hour);
  if (last > first) hours.push(last);
  const points: Point[] = [];
  const push = (hour: number, value: number) => {
    const x = toX(hour);
    if (points.at(-1)?.x !== x) points.push({ x, y: toY(value) });
  };
  let index = 0;
  let previous: number | null = null;
  for (const hour of hours) {
    const skipped: Run[] = [];
    while (index < segment.length - 1 && hour > segment[index].lastHour) skipped.push(segment[index++]);
    const run = segment[index];
    if (previous != null && skipped.length) {
      const low = Math.min(previous, run.value);
      const high = Math.max(previous, run.value);
      const lowest = skipped.reduce((a, b) => (b.value < a.value ? b : a));
      const highest = skipped.reduce((a, b) => (b.value > a.value ? b : a));
      const extremes = [...new Set([lowest, highest])].filter(item => item.value < low || item.value > high)
        .sort((a, b) => a.firstHour - b.firstHour);
      for (const item of extremes) push((item.firstHour + item.lastHour) / 2, item.value);
    }
    push(hour, run.value);
    previous = run.value;
  }
  return points;
}

export function monotonePath(points: readonly Point[]): PathCommand[] {
  if (!points.length) return [];
  const commands: PathCommand[] = [['M', points[0].x, points[0].y]];
  if (points.length === 2) commands.push(['L', points[1].x, points[1].y]);
  if (points.length < 3) return commands;
  const n = points.length;
  const slopes: number[] = [];
  for (let i = 0; i < n - 1; i++) {
    const dx = points[i + 1].x - points[i].x;
    slopes.push(dx === 0 ? 0 : (points[i + 1].y - points[i].y) / dx);
  }
  const tangents = new Array<number>(n);
  tangents[0] = slopes[0];
  tangents[n - 1] = slopes[n - 2];
  for (let i = 1; i < n - 1; i++) tangents[i] = slopes[i - 1] * slopes[i] <= 0 ? 0 : (slopes[i - 1] + slopes[i]) / 2;
  for (let i = 0; i < n - 1; i++) {
    if (slopes[i] === 0) { tangents[i] = 0; tangents[i + 1] = 0; continue; }
    const a = tangents[i] / slopes[i];
    const b = tangents[i + 1] / slopes[i];
    const length = a * a + b * b;
    if (length > 9) {
      const t = 3 / Math.sqrt(length);
      tangents[i] = t * a * slopes[i];
      tangents[i + 1] = t * b * slopes[i];
    }
  }
  for (let i = 0; i < n - 1; i++) {
    const p = points[i];
    const q = points[i + 1];
    const third = (q.x - p.x) / 3;
    commands.push(['C', p.x + third, p.y + tangents[i] * third, q.x - third, q.y - tangents[i + 1] * third, q.x, q.y]);
  }
  return commands;
}

export function curveSegments(runs: readonly Run[], from: number, to: number, toX: (hour: number) => number, toY: (value: number) => number): { points: Point[]; path: PathCommand[] }[] {
  return segments(runs, from, to).map(segment => samplePoints(segment, toX, toY)).filter(points => points.length)
    .map(points => ({ points, path: monotonePath(points) }));
}

export function heldSegments(runs: readonly Run[], from: number, to: number, toX: (hour: number) => number, toY: (value: number) => number): { points: Point[]; path: PathCommand[] }[] {
  if (to < from) return [];
  const result: { points: Point[]; path: PathCommand[] }[] = [];
  let previous: Run | null = null;
  let current: { points: Point[]; path: PathCommand[] } | null = null;
  const push = (hour: number, value: number, move = false) => {
    const point = { x: toX(hour), y: toY(value) };
    if (!move && current!.points.at(-1)?.x === point.x && current!.points.at(-1)?.y === point.y) return;
    current!.points.push(point);
    current!.path.push([move ? 'M' : 'L', point.x, point.y]);
  };
  const [start, end] = rangeBounds(runs, from, to);
  for (let i = start; i < end; i++) {
    const run = runs[i];
    const first = Math.max(from, run.firstHour);
    const last = Math.min(to, run.lastHour + 1);
    const continues = previous && previous.lastHour + 1 === run.firstHour && !run.breakBefore;
    if (!current || !continues) {
      current = { points: [], path: [] };
      result.push(current);
      push(first, run.value, true);
    } else { push(first, previous!.value); push(first, run.value); }
    if (last > first) push(last, run.value);
    previous = run;
  }
  return result;
}

export function extentOf(runs: readonly Run[], from: number, to: number): [number, number] | null {
  const [start, end] = rangeBounds(runs, from, to);
  let low = Infinity, high = -Infinity;
  for (let index = start; index < end; index++) {
    low = Math.min(low, runs[index].value);
    high = Math.max(high, runs[index].value);
  }
  return low <= high ? [low, high] : null;
}

export interface Band { low: number[]; high: number[] }

export function divergingStack(layers: readonly (readonly number[])[]): Band[] {
  const points = layers[0]?.length ?? 0;
  const up = new Array<number>(points).fill(0);
  const down = new Array<number>(points).fill(0);
  return layers.map(layer => {
    const firstValue = layer.find(value => (value ?? 0) !== 0) ?? 0;
    let last = firstValue >= 0;
    const side = new Array<boolean>(points);
    for (let index = 0; index < points; index++) {
      const value = layer[index] ?? 0;
      if (value !== 0) last = value > 0;
      side[index] = last;
    }
    const low: number[] = [];
    const high: number[] = [];
    for (let index = 0; index < points; index++) {
      const value = layer[index] ?? 0;
      if (side[index]) { low.push(up[index]); up[index] += value; high.push(up[index]); }
      else { high.push(down[index]); down[index] += value; low.push(down[index]); }
    }
    return { low, high };
  });
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

export function extentOfAll(lists: readonly (readonly Run[])[], from: number, to: number): [number, number] | null {
  const ranges = lists.map(runs => extentOf(runs, from, to)).filter((range): range is [number, number] => range != null);
  return ranges.length ? [Math.min(...ranges.map(range => range[0])), Math.max(...ranges.map(range => range[1]))] : null;
}

export function lastSampleHour(runs: readonly Span[], at: number): number | null {
  let low = 0;
  let high = runs.length - 1;
  let found: Span | null = null;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (runs[middle].firstHour <= at) { found = runs[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found ? Math.min(found.lastHour, at) : null;
}

export function lineEndHour(runs: readonly Span[], from: number, at: number): number | null {
  const last = lastSampleHour(runs, Math.floor(at));
  return last != null && last >= Math.floor(from) ? last : null;
}

export function lineLabelHours(lines: readonly { tag: string; runs: readonly Span[] }[], from: number, to: number, until: number | null): Map<string, number> {
  const hours = new Map<string, number>();
  for (const line of lines) {
    const hour = lineEndHour(line.runs, from, Math.min(to, until ?? to));
    if (hour !== null) hours.set(line.tag, hour);
  }
  return hours;
}
