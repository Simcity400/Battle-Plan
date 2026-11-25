import { breaksBefore, runAt } from './series.ts';
import type { Breakable } from './series';

export type StackRun = Breakable & { value: number };

export interface Edge { low: number; high: number }

export interface StackModel { hours: number[]; edges: (Edge[] | null)[]; breakBefore: boolean[] }

export type StackSum = { sum: number; error: number };
export function addStackAmount(sum: StackSum, value: number): number {
  const next = sum.sum + value;
  sum.error += Math.abs(sum.sum) >= Math.abs(value) ? (sum.sum - next) + value : (value - next) + sum.sum;
  sum.sum = next;
  return sum.sum + sum.error;
}

export function stackEdges(values: readonly number[]): Edge[] {
  const up: StackSum = { sum: 0, error: 0 }; const down: StackSum = { sum: 0, error: 0 };
  return values.map(value => {
    if (value >= 0) return { low: up.sum + up.error, high: addStackAmount(up, value) };
    return { high: down.sum + down.error, low: addStackAmount(down, value) };
  });
}

export function buildStack(bands: readonly (readonly StackRun[])[]): StackModel {
  const boundaries = new Set<number>();
  for (const runs of bands) {
    for (const run of runs) {
      boundaries.add(run.firstHour);
      boundaries.add(run.lastHour);
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

export interface StackSegment {
  hours: number[]; bands: { low: number[]; high: number[] }[];
  start?: boolean; end?: boolean;
}

export function stackedSegments(model: StackModel, from: number, to: number, minHours: number, bandExtremes = false): StackSegment[] {
  if (!(to >= from)) return [];
  const segments: StackSegment[] = [];
  let current: StackSegment | null = null;
  let bucket: { hour: number; edges: Edge[] }[] = [];
  const push = (hour: number, edges: Edge[]) => {
    current!.hours.push(hour);
    edges.forEach((edge, index) => { current!.bands[index].low.push(edge.low); current!.bands[index].high.push(edge.high); });
  };
  const lastEdges = () => current!.bands.map(band => ({ low: band.low.at(-1)!, high: band.high.at(-1)! }));
  const pushExtremes = (samples: readonly { hour: number; edges: Edge[] }[], next: Edge[]) => {
    if (!samples.length) return;
    const top = (edges: readonly Edge[]) => Math.max(0, ...edges.map(edge => edge.high));
    const bottom = (edges: readonly Edge[]) => Math.min(0, ...edges.map(edge => edge.low));
    const before = lastEdges();
    if (bandExtremes) {
      const kept = new Set<typeof samples[number]>();
      for (let band = 0; band < before.length; band++) {
        for (const edge of ['low', 'high'] as const) {
          let highest = samples[0]; let lowest = samples[0];
          for (const sample of samples) {
            if (sample.edges[band][edge] > highest.edges[band][edge]) highest = sample;
            if (sample.edges[band][edge] < lowest.edges[band][edge]) lowest = sample;
          }
          if (highest.edges[band][edge] > Math.max(before[band][edge], next[band][edge])) kept.add(highest);
          if (lowest.edges[band][edge] < Math.min(before[band][edge], next[band][edge])) kept.add(lowest);
        }
      }
      for (const sample of samples) if (kept.has(sample)) push(sample.hour, sample.edges);
      return;
    }
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
  const flush = () => {
    const pending = bucket.at(-1);
    if (current && pending) { pushExtremes(bucket.slice(0, -1), pending.edges); push(pending.hour, pending.edges); }
    bucket = [];
  };
  let holdsUntil = Infinity;
  const close = () => {
    flush();
    const end = current ? Math.min(current.hours[0] + 1, holdsUntil) : 0;
    if (current && current.hours.length === 1 && end > current.hours[0]) push(end, lastEdges());
    current = null;
    bucket = [];
  };
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
  if (current && held) {
    flush();
    if ((current as StackSegment).hours.at(-1)! < to) push(to, held);
  }
  close();
  return segments;
}

export function stackExtent(model: StackModel, from: number, to: number): [number, number] | null {
  let high = -Infinity;
  let low = Infinity;
  model.hours.forEach((hour, index) => {
    const edges = model.edges[index];
    const next = model.hours[index + 1] ?? Infinity;
    if (!edges || next <= from || hour > to) return;
    for (const edge of edges) { high = Math.max(high, edge.high); low = Math.min(low, edge.low); }
  });
  return high >= low ? [Math.min(0, low), Math.max(0, high)] : null;
}

export interface CompactStackModel {
  bands: readonly (readonly StackRun[])[];
  hours: number[];
  bounds: ([number, number] | null)[];
  breakBefore: boolean[];
}

export function forEachCompactStackSegment(model: CompactStackModel, from: number, to: number, minHours: number,
  visit: (segment: StackSegment) => void, sample?: (hour: number) => Edge[]): void {
  if (!(to >= from)) return;
  type Point = { hour: number; index: number };
  const cursor: { previous: { point: Point; edges: Edge[]; bounds: [number, number] } | null; piece: StackSegment | null } = { previous: null, piece: null };
  let firstHour = 0;
  let emitted = 0;
  let holdsUntil = Infinity;
  let pending: Point | null = null;
  let highest: Point | null = null;
  let lowest: Point | null = null;
  const top = (point: Point) => model.bounds[point.index]![1];
  const bottom = (point: Point) => model.bounds[point.index]![0];
  const push = (point: Point) => {
    if (cursor.previous?.point.hour === point.hour) return;
    const edges = sample ? sample(point.hour) : stackEdges(model.bands.map(runs => runAt(runs, model.hours[point.index])!.value));
    let bounds = model.bounds[point.index]!;
    if (sample) {
      bounds = [0, 0];
      for (const edge of edges) { bounds[0] = Math.min(bounds[0], edge.low); bounds[1] = Math.max(bounds[1], edge.high); }
    }
    if (cursor.previous) {
      if (cursor.piece) visit(cursor.piece);
      cursor.piece = { hours: [cursor.previous.point.hour, point.hour], start: emitted === 1, end: false,
        bands: edges.map((edge, index) => ({ low: [cursor.previous!.edges[index].low, edge.low], high: [cursor.previous!.edges[index].high, edge.high] })) };
    }
    cursor.previous = { point, edges, bounds };
    emitted++;
  };
  const extremes = (next: Point) => {
    if (!cursor.previous) return;
    const kept: Point[] = [];
    if (highest && top(highest) > Math.max(cursor.previous.bounds[1], top(next))) kept.push(highest);
    if (lowest && bottom(lowest) < Math.min(cursor.previous.bounds[0], bottom(next)) && !kept.includes(lowest)) kept.push(lowest);
    kept.sort((a, b) => a.hour - b.hour).forEach(push);
  };
  const flush = () => {
    if (pending) { extremes(pending); push(pending); }
    pending = highest = lowest = null;
  };
  const close = () => {
    flush();
    if (cursor.previous && emitted === 1) {
      const end = Math.min(firstHour + 1, holdsUntil);
      if (end > firstHour) push({ ...cursor.previous.point, hour: end });
    }
    if (cursor.piece) { cursor.piece.end = true; visit(cursor.piece); cursor.piece = null; }
    cursor.previous = null;
    emitted = 0;
  };
  let start = 0;
  for (let low = 0, high = model.hours.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (model.hours[middle] <= from) { start = middle; low = middle + 1; } else high = middle - 1;
  }
  let held: Point | null = null;
  for (let index = start; index < model.hours.length && model.hours[index] <= to; index++) {
    const point = { hour: Math.max(from, model.hours[index]), index };
    held = model.bounds[index] ? point : null;
    if (!held || model.breakBefore[index]) close();
    if (!held) continue;
    if (!cursor.previous) {
      firstHour = point.hour;
      holdsUntil = model.hours[index + 1] ?? Infinity;
      push(point);
    } else if (point.hour - cursor.previous.point.hour < minHours) {
      pending = point;
      if (!highest || top(point) > top(highest)) highest = point;
      if (!lowest || bottom(point) < bottom(lowest)) lowest = point;
    } else {
      extremes(point);
      pending = highest = lowest = null;
      push(point);
    }
  }
  flush();
  if (cursor.previous && held && cursor.previous.point.hour < to) push({ ...held, hour: to });
  close();
}

export function forEachJoinedSegment(pieces: (visit: (segment: StackSegment) => void) => void, visit: (segment: StackSegment) => void): void {
  let joined: StackSegment | null = null;
  pieces(segment => {
    if (joined && !segment.start && joined.end !== true && joined.hours.at(-1) === segment.hours[0] && joined.bands.length === segment.bands.length) {
      for (let index = 1; index < segment.hours.length; index++) joined.hours.push(segment.hours[index]);
      segment.bands.forEach((band, layer) => {
        for (let index = 1; index < band.low.length; index++) { joined!.bands[layer].low.push(band.low[index]); joined!.bands[layer].high.push(band.high[index]); }
      });
      joined.end = segment.end;
      return;
    }
    if (joined) visit(joined);
    joined = { ...segment, hours: [...segment.hours], bands: segment.bands.map(band => ({ low: [...band.low], high: [...band.high] })) };
  });
  if (joined) visit(joined);
}

export function forEachCompactStackStretch(model: CompactStackModel, from: number, to: number, minHours: number,
  visit: (segment: StackSegment) => void, sample?: (hour: number) => Edge[]): void {
  forEachJoinedSegment(piece => forEachCompactStackSegment(model, from, to, minHours, piece, sample), visit);
}

export function compactStackExtent(model: CompactStackModel, from: number, to: number): [number, number] | null {
  let low = Infinity;
  let high = -Infinity;
  model.hours.forEach((hour, index) => {
    const bounds = model.bounds[index];
    if (!bounds || (model.hours[index + 1] ?? Infinity) <= from || hour > to) return;
    low = Math.min(low, bounds[0]);
    high = Math.max(high, bounds[1]);
  });
  return high >= low ? [low, high] : null;
}
