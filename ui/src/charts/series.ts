/** Recorded series as runs of equal hourly values. A hour outside every run is a
 * gap: it is never drawn as zero, and lines break across it. */

/** `breakBefore` ends the line before this run even when the hours touch, such as
 * when a different country takes over the tag. */
export interface Run { firstHour: number; lastHour: number; value: number; breakBefore?: boolean }
export interface Point { x: number; y: number }
export type PathCommand = readonly ['M', number, number] | readonly ['L', number, number] | readonly ['C', number, number, number, number, number, number];

/** The run holding `hour`, by binary search over runs sorted by hour. */
export function runAt(runs: readonly Run[], hour: number): Run | null {
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
