/** Recorded series as runs of equal hourly values. A hour outside every run is a
 * gap: it is never drawn as zero, and lines break across it. */

/** `breakBefore` ends the line before this run even when the hours touch, such as
 * when a different country takes over the tag. */
export interface Run { firstHour: number; lastHour: number; value: number; breakBefore?: boolean }
export interface Point { x: number; y: number }
export type PathCommand = readonly ['M', number, number] | readonly ['L', number, number] | readonly ['C', number, number, number, number, number, number];

/** Hours from `firstHour` to `lastHour`, both included. */
export interface Span { firstHour: number; lastHour: number }

/** A run whose line may break before it: `breakBefore` on plotting runs,
 * `startsSegment` on the runs products send. */
export type Breakable = Span & { breakBefore?: boolean; startsSegment?: boolean };

/** Whether a line breaks before this run even if the hours touch. */
export const breaksBefore = (run: Breakable) => run.breakBefore === true || run.startsSegment === true;

/** The run holding `hour`, by binary search over runs sorted by hour that never overlap. */
export function runAt<T extends Span>(runs: readonly T[], hour: number): T | null {
  let low = 0;
  let high = runs.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = runs[middle];
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return run;
  }
  return null;
}

export function valueAt(runs: readonly Run[], hour: number): number | null {
  return runAt(runs, Math.floor(hour))?.value ?? null;
}

/** Unbroken stretches of runs overlapping [from, to]. Runs that touch hour to
 * hour join; any missing hour between them starts a new segment. */
export function segments(runs: readonly Run[], from: number, to: number): Run[][] {
  const result: Run[][] = [];
  let current: Run[] = [];
  // The hour a fractional edge falls in is visible, as valueAt treats it.
  const first = Math.floor(from);
  for (const run of runs) {
    if (run.lastHour < first || run.firstHour > to) continue;
    const clipped = { firstHour: Math.max(run.firstHour, Math.floor(from)), lastHour: Math.min(run.lastHour, Math.ceil(to)), value: run.value };
    const previous = current.at(-1);
    if (previous && (previous.lastHour + 1 !== run.firstHour || run.breakBefore)) { result.push(current); current = []; }
    current.push(clipped);
  }
  if (current.length) result.push(current);
  return result;
}

/** Points along one segment about every `stepPx` pixels, always including both
 * ends. Each sample takes the exact recorded value at its hour, and any peak or dip
 * between two samples is kept too, so a short spike is drawn rather than skipped. */
export function samplePoints(segment: readonly Run[], toX: (hour: number) => number, toY: (value: number) => number, stepPx = 12): Point[] {
  if (!segment.length) return [];
  const first = segment[0].firstHour;
  const last = segment.at(-1)!.lastHour;
  const pixels = Math.abs(toX(last) - toX(first));
  const count = Math.max(1, Math.round(pixels / stepPx));
  const points: Point[] = [];
  const push = (hour: number, value: number) => {
    const x = toX(hour);
    if (points.at(-1)?.x !== x) points.push({ x, y: toY(value) });
  };
  let index = 0;
  let previous: number | null = null;
  for (let step = 0; step <= count; step++) {
    const hour = Math.round(first + (last - first) * (step / count));
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
  let low = Infinity;
  let high = -Infinity;
  for (const run of runs) {
    if (run.lastHour < Math.floor(from) || run.firstHour > to) continue;
    low = Math.min(low, run.value);
    high = Math.max(high, run.value);
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
  const first = Math.min(from, to);
  const last = Math.max(from, to);
  let index = runs.findIndex(run => run.firstHour <= first && first <= run.lastHour);
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
  const clipped: T[] = [];
  for (const run of runs) {
    for (const span of spans) {
      const first = Math.max(run.firstHour, span.firstHour);
      const last = Math.min(run.lastHour, span.lastHour);
      if (first > last) continue;
      const piece: T = { ...run, firstHour: first, lastHour: last };
      if (first !== run.firstHour) {
        if ('breakBefore' in piece) delete piece.breakBefore;
        if ('startsSegment' in piece) piece.startsSegment = false;
      }
      clipped.push(piece);
    }
  }
  return clipped.sort((a, b) => a.firstHour - b.firstHour);
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
