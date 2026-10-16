/** Recorded series as runs of equal hourly values. A hour outside every run is a
 * gap: it is never drawn as zero, and lines break across it. */

/** `breakBefore` ends the line before this run even when the hours touch, such as
 * when a different country takes over the tag. */
export interface Run { firstHour: number; lastHour: number; value: number; breakBefore?: boolean }
export interface Point { x: number; y: number }
export type PathCommand = readonly ['M', number, number] | readonly ['L', number, number] | readonly ['C', number, number, number, number, number, number];

/** Hours from `firstHour` to `lastHour`, both included. */
export interface Span { firstHour: number; lastHour: number }

/** Admit a recorded inclusive-hour span without changing its fields. */
export function isSpan(value: unknown): value is Record<string, unknown> & Span {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const span = value as Record<string, unknown>;
  return typeof span.firstHour === 'number' && Number.isSafeInteger(span.firstHour)
    && typeof span.lastHour === 'number' && Number.isSafeInteger(span.lastHour) && span.firstHour <= span.lastHour;
}

/** Admit ordered, disjoint spans; adjacent spans and gaps both remain intact. */
export function isOrderedSpans(value: unknown): value is (Record<string, unknown> & Span)[] {
  if (!Array.isArray(value)) return false;
  let last: number | null = null;
  for (const span of value) {
    if (!isSpan(span) || (last !== null && last >= span.firstHour)) return false;
    last = span.lastHour;
  }
  return true;
}

/** A run whose line may break before it: `breakBefore` on plotting runs,
 * `startsSegment` on the runs products send. */
export type Breakable = Span & { breakBefore?: boolean; startsSegment?: boolean };

/** Whether a line breaks before this run even if the hours touch. */
export const breaksBefore = (run: Breakable) => run.breakBefore === true || run.startsSegment === true;

/** Replace reached hours without allowing coarse runs to shadow exact values or missing reads. */
export function overlayRuns<T extends Breakable>(before: readonly T[], incoming: readonly T[], equal: (a:T,b:T)=>boolean, within: readonly Span[] = incoming): T[] {
  const cuts = clipToSpans([{firstHour:-2147483648,lastHour:2147483647}],within);
  const retained: T[] = [];
  let index = 0;
  for (const run of before) {
    while (index < cuts.length && cuts[index].lastHour < run.firstHour) index++;
    let first = run.firstHour;
    for (let at = index; at < cuts.length && cuts[at].firstHour <= run.lastHour; at++) {
      if (first < cuts[at].firstHour) retained.push({...run,firstHour:first,lastHour:cuts[at].firstHour-1,
        ...('startsSegment' in run ? {startsSegment:first===run.firstHour && run.startsSegment} : {}),
        ...('breakBefore' in run ? {breakBefore:first===run.firstHour && run.breakBefore} : {})});
      first = Math.max(first,cuts[at].lastHour+1);
    }
    if (first <= run.lastHour) retained.push({...run,firstHour:first,
      ...('startsSegment' in run ? {startsSegment:first===run.firstHour && run.startsSegment} : {}),
      ...('breakBefore' in run ? {breakBefore:first===run.firstHour && run.breakBefore} : {})});
  }
  const joined:T[]=[];
  for (const run of [...retained,...incoming.map(run=>({...run}))].sort((a,b)=>a.firstHour-b.firstHour)) {
    const previous=joined.at(-1);
    if(previous && previous.lastHour+1===run.firstHour && !breaksBefore(run) && equal(previous,run)) previous.lastHour=run.lastHour;
    else joined.push(run);
  }
  return joined;
}

/** The index of the run holding `hour`, by binary search over runs sorted by hour
 * that never overlap; -1 when none holds it. */
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

/** The run holding `hour`, by binary search over runs sorted by hour that never overlap. */
export function runAt<T extends Span>(runs: readonly T[], hour: number): T | null {
  const index = runIndexAt(runs, hour);
  return index < 0 ? null : runs[index];
}

/** Ordered, disjoint value spans must fit wholly inside the observed hours.
 * Two cursors keep validation linear even for thousands of country histories. */
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

/** For each run, the first hour of the unbroken stretch it belongs to: no missing
 * hour, line break or change of country since. Built once per series, in one pass. */
export function stretchStarts(runs: readonly Breakable[]): number[] {
  const starts: number[] = [];
  runs.forEach((run, index) => {
    const previous = runs[index - 1];
    const continues = previous != null && !breaksBefore(run) && previous.lastHour + 1 === run.firstHour;
    starts.push(continues ? starts[index - 1] : run.firstHour);
  });
  return starts;
}

/** The first hour of the unbroken stretch holding `hour`, or null when no run holds
 * it, given the runs' `stretchStarts`. Two hours are in the same stretch, so a value
 * may glide between them (`sameStretch`), exactly when both give the same hour. */
export function stretchAt(runs: readonly Span[], starts: readonly number[], hour: number): number | null {
  const index = runIndexAt(runs, Math.floor(hour));
  return index < 0 ? null : starts[index];
}

export function valueAt(runs: readonly Run[], hour: number): number | null {
  return runAt(runs, Math.floor(hour))?.value ?? null;
}

/** Bounds of ordered spans intersecting an inclusive hour range. */
export function rangeBounds(runs: readonly Span[], from: number, to: number): [number, number] {
  let low = 0, high = runs.length;
  while (low < high) { const mid = (low + high) >>> 1; if (runs[mid].lastHour < Math.floor(from)) low = mid + 1; else high = mid; }
  const first = low;
  high = runs.length;
  while (low < high) { const mid = (low + high) >>> 1; if (!(runs[mid].firstHour > to)) low = mid + 1; else high = mid; }
  return [first, low];
}

/** Unbroken stretches of runs overlapping [from, to]. Runs that touch hour to
 * hour join; any missing hour between them starts a new segment. */
export function segments(runs: readonly Run[], from: number, to: number): Run[][] {
  // An empty interval (a playhead before the view's start) draws nothing.
  if (!(to >= from)) return [];
  const result: Run[][] = [];
  let current: Run[] = [];
  // The hour a fractional edge falls in is visible, as valueAt treats it.
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

/** Points along one segment about every `stepPx` pixels, always including both
 * ends. Between the ends, samples sit on a fixed grid of hours a power of two apart,
 * counted from hour 0, not spread evenly between the ends: a line growing during
 * playback, or seen through a sliding window, keeps every sample it had, so only its
 * ends move instead of the whole curve reshaping each frame. As the scale changes the
 * grid only halves or doubles, keeping the samples on both grids. Each sample takes
 * the exact recorded value at its hour, and any peak or dip between two samples is
 * kept too, so a short spike is drawn rather than skipped. */
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

/** A monotone cubic (Fritsch–Carlson) through the points. Between two samples the
 * curve stays inside their values, so it never shows a peak or dip that wasn't
 * recorded. */
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

/** The curves a line through `runs` is drawn with between `from` and `to`: per
 * unbroken segment, its sample points and the monotone path through them. Lines
 * and anything that must agree with them (a fill between two lines) use this. */
export function curveSegments(runs: readonly Run[], from: number, to: number, toX: (hour: number) => number, toY: (value: number) => number): { points: Point[]; path: PathCommand[] }[] {
  return segments(runs, from, to).map(segment => samplePoints(segment, toX, toY)).filter(points => points.length)
    .map(points => ({ points, path: monotonePath(points) }));
}

/** Exact held values: horizontal until the next read, vertical at that read.
 * Missing hours and identity breaks start separate paths, including at zoom edges. */
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

/** Closed regions for shading between two lines drawn with `curveSegments`, split
 * by which is on top wherever they cross: `above` (the first line higher) is what
 * lies under the first curve and over the second; `below` the reverse. Each region
 * is a clip path and a fill path, both closed to the plot's `top` or `bottom` edge
 * (screen y), so the shaded area's edges are exactly the drawn curves. Segments are
 * paired by index: both lines must break at the same hours. */
export interface GapRegion { clip: PathCommand[]; fill: PathCommand[] }
export function gapRegions(first: readonly { points: Point[]; path: PathCommand[] }[], second: readonly { points: Point[]; path: PathCommand[] }[],
  top: number, bottom: number): { above: GapRegion[]; below: GapRegion[] } {
  // Closes a curve down (or up) to an edge of the plot, into an area.
  const toEdge = (curve: { points: Point[]; path: PathCommand[] }, edge: number): PathCommand[] => {
    const start = curve.points[0];
    const end = curve.points.at(-1)!;
    return [...curve.path, ['L', end.x, edge], ['L', start.x, edge], ['L', start.x, start.y]];
  };
  const above: GapRegion[] = [];
  const below: GapRegion[] = [];
  for (let index = 0; index < Math.min(first.length, second.length); index++) {
    const a = first[index];
    const b = second[index];
    if (a.points.length < 2 || b.points.length < 2) continue;
    // Screen y grows downward: "under the first curve" is toward the bottom edge.
    above.push({ clip: toEdge(a, bottom), fill: toEdge(b, top) });
    below.push({ clip: toEdge(b, bottom), fill: toEdge(a, top) });
  }
  return { above, below };
}

/** Cumulative totals for stacked layers sampled at the same points: `result[k][i]`
 * is the top of layer k at point i, or null when it is unknown because that layer
 * or one below it was not recorded there. Missing never stacks as zero. */
export function stack(layers: readonly (readonly (number | null)[])[]): (number | null)[][] {
  const tops: (number | null)[][] = [];
  let base: (number | null)[] = layers[0]?.map(() => 0) ?? [];
  for (const layer of layers) {
    const top = layer.map((value, index) => {
      const below = base[index];
      return value == null || below == null ? null : below + value;
    });
    tops.push(top);
    base = top;
  }
  return tops;
}

/** The lowest and highest value of the runs inside [from, to], or null. */
export function extentOf(runs: readonly Run[], from: number, to: number): [number, number] | null {
  const [start, end] = rangeBounds(runs, from, to);
  let low = Infinity, high = -Infinity;
  for (let index = start; index < end; index++) {
    low = Math.min(low, runs[index].value);
    high = Math.max(high, runs[index].value);
  }
  return low <= high ? [low, high] : null;
}

/** A stacked layer's lower and upper edge at each point. */
export interface Band { low: number[]; high: number[] }

/** Stacks layers of signed values: at each point the positive values stack up
 * from zero and the negative ones down, each in layer order, so a layer's band
 * is its own share either side of the axis. */
export function divergingStack(layers: readonly (readonly number[])[]): Band[] {
  const points = layers[0]?.length ?? 0;
  const up = new Array<number>(points).fill(0);
  const down = new Array<number>(points).fill(0);
  return layers.map(layer => {
    // A zero sample stays on the side the layer is on around it, so a band that
    // shrinks to nothing closes where it is instead of jumping across the axis.
    // Leading zeros take the side of the first non-zero sample.
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

/** Whether the value at `to` continues the one at `from` in the same unbroken
 * stretch of runs: no missing hour, line break or change of country between them.
 * Something drawn from these runs (a bar, a number) glides between the two only
 * then; across a break it jumps, as a line breaks. */
export function sameStretch(runs: readonly Breakable[], from: number, to: number): boolean {
  if (!Number.isFinite(from) || !Number.isFinite(to)) return false;
  const first = Math.min(from, to);
  const last = Math.max(from, to);
  let index = runIndexAt(runs, first);
  if (index < 0) return false;
  while (runs[index].lastHour < last) {
    const next = runs[index + 1];
    if (!next || breaksBefore(next) || next.firstHour !== runs[index].lastHour + 1) return false;
    index++;
  }
  return true;
}

/** Runs cut to the hours of `spans` (such as a date range, or the hours a country
 * was a group member), in hour order, so nothing outside them is drawn, shaded,
 * fitted or looked up. A cut piece keeps its run's line break (a new country
 * identity) only where the run itself starts. */
export function clipToSpans<T extends Breakable>(runs: readonly T[], spans: readonly Span[]): T[] {
  // One pass over both, in hour order: linear in the runs and spans, so a history
  // of tens of thousands of each (a country swapping ranks every hour) cuts at once.
  const ordered = <S extends Span>(list: readonly S[]) =>
    list.every((item, index) => index === 0 || list[index - 1].firstHour <= item.firstHour) ? list : [...list].sort((a, b) => a.firstHour - b.firstHour);
  const sortedRuns = ordered(runs);
  // Overlapping spans merge, so no hour is cut out twice; spans that only touch stay apart.
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
    // Move past whichever ends first; the other may still meet the next one.
    if (run.lastHour < span.lastHour) runIndex++;
    else spanIndex++;
  }
  return clipped;
}

/** The value range of several run lists over [from, to], or null when none has a value there. */
export function extentOfAll(lists: readonly (readonly Run[])[], from: number, to: number): [number, number] | null {
  const ranges = lists.map(runs => extentOf(runs, from, to)).filter((range): range is [number, number] => range != null);
  return ranges.length ? [Math.min(...ranges.map(range => range[0])), Math.max(...ranges.map(range => range[1]))] : null;
}

/** The last hour at or before `at` that runs (in hour order) hold a value for, or
 * null: where a line drawn up to `at` ends, even when `at` falls in a gap after it. */
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

/** Where a line's end label belongs: the last hour it is drawn at, up to `at` (the
 * playhead or the view's end) and not before `from` (the view's start); null when
 * none of it is drawn there. A playhead or view ending in a gap after the line's
 * last value still labels the line where it ends, so a drawn line is always named. */
export function lineEndHour(runs: readonly Span[], from: number, at: number): number | null {
  const last = lastSampleHour(runs, Math.floor(at));
  return last != null && last >= Math.floor(from) ? last : null;
}

/** The hour each line is labelled: its last drawn hour within the view, capped
 * where drawing stops (`until`, the playhead unless lines continue past it). A
 * line absent there can still have
 * an earlier end label; a line with no visible hours has none. */
export function lineLabelHours(lines: readonly { tag: string; runs: readonly Span[] }[], from: number, to: number, until: number | null): Map<string, number> {
  const hours = new Map<string, number>();
  for (const line of lines) {
    const hour = lineEndHour(line.runs, from, Math.min(to, until ?? to));
    if (hour !== null) hours.set(line.tag, hour);
  }
  return hours;
}
