/** Stacked areas over time from runs of recorded values, one band per layer (a
 * party's popularity, a factory type's count): where each band's edges lie at
 * every hour a value can change, and the pieces to draw between two hours. Values
 * are signed: at each hour the positive ones stack up from zero and the negative
 * ones down from it, each in band order, so no recorded value is ever replaced.
 * Missing never stacks as zero: an hour where any band is unrecorded breaks the
 * whole stack. A sample holds until the next one, as a run holds its value. */
import { breaksBefore, runAt } from './series.ts';
import type { Breakable } from './series';

/** A run of one band's value, with the line break it may start. */
export type StackRun = Breakable & { value: number };

/** A band's lower and upper edge at one sample. */
export interface Edge { low: number; high: number }

/** Every hour where any band's value can change, with each band's edges there;
 * null where a band is unrecorded, and a break before an hour where a band starts
 * a new line (a new country identity). Built once per reply. */
export interface StackModel { hours: number[]; edges: (Edge[] | null)[]; breakBefore: boolean[] }

/** The bands' edges at one sample: positive values up from zero, negative down. */
export function stackEdges(values: readonly number[]): Edge[] {
  let up = 0;
  let down = 0;
  return values.map(value => {
    if (value >= 0) { const low = up; up += value; return { low, high: up }; }
    const high = down; down += value; return { low: down, high };
  });
}

export function buildStack(bands: readonly (readonly StackRun[])[]): StackModel {
  const boundaries = new Set<number>();
  for (const runs of bands) {
    for (const run of runs) {
      boundaries.add(run.firstHour);
      boundaries.add(run.lastHour);
      // The hour after a run is sampled too, so a gap there breaks the bands.
      boundaries.add(run.lastHour + 1);
    }
  }
  const hours = [...boundaries].sort((a, b) => a - b);
  const model: StackModel = { hours, edges: [], breakBefore: [] };
  for (const hour of hours) {
    let known = true;
    let fresh = false;
    const values: number[] = [];
    for (const runs of bands) {
      const run = runAt(runs, hour);
      if (!run) { known = false; break; }
      if (run.firstHour === hour && breaksBefore(run)) fresh = true;
      values.push(run.value);
    }
    model.edges.push(known ? stackEdges(values) : null);
    model.breakBefore.push(fresh);
  }
  return model;
}

/** A piece of the stack to draw: its sample hours and each band's edges there
 * (`drawBands` draws them). */
export interface StackSegment { hours: number[]; bands: { low: number[]; high: number[] }[] }

/** The pieces to draw between `from` and `to` (the view's start, and its end or the
 * playhead). A sample before `from` holds into it, so a piece starts at `from`, and
 * the sample holding at `to` is drawn up to it, so a view or playhead ending inside
 * a run still shows it. Samples closer than `minHours` merge, but every piece keeps
 * its first and last sample, and a merged run of samples keeps its highest top and
 * lowest bottom where they pass the samples kept either side, so a peak or dip
 * narrower than a pixel is still drawn; an isolated recorded hour draws as one hour
 * wide. */
export function stackedSegments(model: StackModel, from: number, to: number, minHours: number): StackSegment[] {
  // An empty interval (a playhead before the view's start) draws nothing.
  if (!(to >= from)) return [];
  const segments: StackSegment[] = [];
  let current: StackSegment | null = null;
  // The samples merged since the last one drawn, in hour order.
  let bucket: { hour: number; edges: Edge[] }[] = [];
  const push = (hour: number, edges: Edge[]) => {
    current!.hours.push(hour);
    edges.forEach((edge, index) => { current!.bands[index].low.push(edge.low); current!.bands[index].high.push(edge.high); });
  };
  const lastEdges = () => current!.bands.map(band => ({ low: band.low.at(-1)!, high: band.high.at(-1)! }));
  // Draws the merged samples' highest top and lowest bottom, in hour order, where they
  // pass both the sample drawn before them and `next`, the one drawn after them.
  const pushExtremes = (samples: readonly { hour: number; edges: Edge[] }[], next: Edge[]) => {
    if (!samples.length) return;
    const top = (edges: readonly Edge[]) => Math.max(0, ...edges.map(edge => edge.high));
    const bottom = (edges: readonly Edge[]) => Math.min(0, ...edges.map(edge => edge.low));
    const before = lastEdges();
    let highest = samples[0];
    let lowest = samples[0];
    for (const sample of samples) {
      if (top(sample.edges) > top(highest.edges)) highest = sample;
      if (bottom(sample.edges) < bottom(lowest.edges)) lowest = sample;
    }
    const kept = new Set<{ hour: number; edges: Edge[] }>();
    if (top(highest.edges) > Math.max(top(before), top(next))) kept.add(highest);
    if (bottom(lowest.edges) < Math.min(bottom(before), bottom(next))) kept.add(lowest);
    for (const sample of samples) if (kept.has(sample)) push(sample.hour, sample.edges);
  };
  // Draws the merged samples' extremes, then the last of them.
  const flush = () => {
    const pending = bucket.at(-1);
    if (current && pending) { pushExtremes(bucket.slice(0, -1), pending.edges); push(pending.hour, pending.edges); }
    bucket = [];
  };
  // Where a piece's first sample stops holding: the next sample's hour.
  let holdsUntil = Infinity;
  const close = () => {
    flush();
    // A lone sample is drawn an hour wide, but never past where it holds (into a gap
    // after it), however `from` cut it.
    const end = current ? Math.min(current.hours[0] + 1, holdsUntil) : 0;
    if (current && current.hours.length === 1 && end > current.hours[0]) push(end, lastEdges());
    current = null;
    bucket = [];
  };
  // Start from the sample at or before `from`, so the visible start is covered.
  let start = 0;
  for (let low = 0, high = model.hours.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (model.hours[middle] <= from) { start = middle; low = middle + 1; } else high = middle - 1;
  }
  let held: Edge[] | null = null;
  for (let index = start; index < model.hours.length && model.hours[index] <= to; index++) {
    const hour = Math.max(model.hours[index], from);
    const edges = model.edges[index];
    held = edges;
    if (!edges || model.breakBefore[index]) close();
    if (!edges) continue;
    if (!current) {
      current = { hours: [], bands: edges.map(() => ({ low: [], high: [] })) };
      segments.push(current);
      holdsUntil = model.hours[index + 1] ?? Infinity;
      push(hour, edges);
      continue;
    }
    if (hour - current.hours.at(-1)! < minHours) { bucket.push({ hour, edges }); continue; }
    pushExtremes(bucket, edges);
    bucket = [];
    push(hour, edges);
  }
  // The sample holding at `to` is drawn up to it.
  if (current && held) {
    flush();
    if ((current as StackSegment).hours.at(-1)! < to) push(to, held);
  }
  close();
  return segments;
}

/** The lowest and highest edge inside [from, to], zero included, or null when
 * nothing is recorded there. */
export function stackExtent(model: StackModel, from: number, to: number): [number, number] | null {
  let high = -Infinity;
  let low = Infinity;
  model.hours.forEach((hour, index) => {
    const edges = model.edges[index];
    const next = model.hours[index + 1] ?? Infinity;
    // A sample holds until the next one, so one before `from` still shows.
    if (!edges || next <= from || hour > to) return;
    for (const edge of edges) { high = Math.max(high, edge.high); low = Math.min(low, edge.low); }
  });
  return high >= low ? [Math.min(0, low), Math.max(0, high)] : null;
}
