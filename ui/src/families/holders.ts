import { compareTags } from '../dashboard/tagOrder.ts';

export type HourRanges = readonly (readonly [number, number])[];

export interface HolderRun {
  readonly tag: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly countryId?: number;
  readonly unresolved?: boolean;
}

export interface RawSlotHolderRun {
  readonly tag: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly countryId: number;
}

function beatsSlotHolder(candidate: { readonly countryId: number; readonly firstHour: number; readonly tag: string }, current: { readonly countryId: number; readonly firstHour: number; readonly tag: string }): boolean {
  if (candidate.countryId !== current.countryId) return candidate.countryId < current.countryId;
  if (candidate.firstHour !== current.firstHour) return candidate.firstHour < current.firstHour;
  return compareTags(candidate.tag, current.tag) < 0;
}

export function resolveSlotHolderRuns(raw: readonly RawSlotHolderRun[]): HolderRun[] {
  if (!raw.length) return [];
  const edges = new Set<number>();
  for (let i = 0; i < raw.length; i++) {
    const run = raw[i]!;
    if (run.firstHour <= run.lastHour) {
      edges.add(run.firstHour);
      edges.add(run.lastHour + 1);
    }
  }
  const sortedEdges = [...edges].sort((a, b) => a - b);
  const out: { tag: string; firstHour: number; lastHour: number; countryId: number }[] = [];
  for (let i = 0; i + 1 < sortedEdges.length; i++) {
    const from = sortedEdges[i]!;
    const to = sortedEdges[i + 1]! - 1;
    let winner: RawSlotHolderRun | null = null;
    for (let j = 0; j < raw.length; j++) {
      const run = raw[j]!;
      if (run.firstHour <= from && to <= run.lastHour) {
        if (!winner || beatsSlotHolder(run, winner)) {
          winner = run;
        }
      }
    }
    if (!winner) continue;
    const prev = out[out.length - 1];
    if (prev && prev.lastHour + 1 === from && prev.tag === winner.tag && prev.countryId === winner.countryId) {
      prev.lastHour = to;
    } else {
      out.push({ tag: winner.tag, firstHour: from, lastHour: to, countryId: winner.countryId });
    }
  }
  return out;
}

export function mergeTagExistenceRuns(
  tag: string,
  identities: readonly { readonly firstHour: number; readonly lastHour: number }[],
  unresolved: readonly { readonly firstHour: number; readonly lastHour: number }[] = [],
): HolderRun[] {
  const combined: HolderRun[] = [];
  for (const run of identities) {
    if (run.firstHour <= run.lastHour) combined.push({ tag, firstHour: run.firstHour, lastHour: run.lastHour });
  }
  for (const span of unresolved) {
    if (span.firstHour <= span.lastHour) combined.push({ tag, firstHour: span.firstHour, lastHour: span.lastHour, unresolved: true });
  }
  combined.sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour || (a.unresolved ? 1 : 0) - (b.unresolved ? 1 : 0));
  const out: { tag: string; firstHour: number; lastHour: number; unresolved?: boolean }[] = [];
  for (const run of combined) {
    const prev = out[out.length - 1];
    if (prev && prev.lastHour + 1 >= run.firstHour && Boolean(prev.unresolved) === Boolean(run.unresolved)) {
      if (run.lastHour > prev.lastHour) prev.lastHour = run.lastHour;
    } else {
      out.push({ ...run });
    }
  }
  return out;
}

export function holderRunAt(runs: readonly HolderRun[] | undefined, hour: number): HolderRun | null {
  if (!runs || !runs.length) return null;
  let best: HolderRun | null = null;
  let monotonic = true;
  for (let i = 1; i < runs.length; i++) {
    if (runs[i]!.firstHour <= runs[i - 1]!.lastHour) {
      monotonic = false;
      break;
    }
  }
  if (monotonic) {
    let low = 0;
    let high = runs.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (runs[mid]!.firstHour <= hour) low = mid + 1;
      else high = mid;
    }
    const candidate = runs[low - 1];
    return candidate && candidate.firstHour <= hour && hour <= candidate.lastHour ? candidate : null;
  }
  for (let i = 0; i < runs.length; i++) {
    const run = runs[i]!;
    if (run.firstHour <= hour && hour <= run.lastHour) {
      if (
        !best
        || (run.countryId !== undefined && best.countryId !== undefined
          ? beatsSlotHolder(
              { countryId: run.countryId, firstHour: run.firstHour, tag: run.tag },
              { countryId: best.countryId, firstHour: best.firstHour, tag: best.tag },
            )
          : false)
      ) {
        best = run;
      }
    }
  }
  return best;
}

export function holderTagAt(runs: readonly HolderRun[] | undefined, hour: number): string | null {
  return holderRunAt(runs, hour)?.tag ?? null;
}

export function buildChangeIndex(edges: Iterable<number>): readonly number[] {
  const unique = new Set<number>();
  for (const edge of edges) {
    if (Number.isFinite(edge)) unique.add(edge);
  }
  return Object.freeze([...unique].sort((a, b) => a - b));
}

export function nextChangeAfter(bounds: readonly number[] | undefined, hour: number): number {
  if (!bounds || !bounds.length) return Infinity;
  let low = 0;
  let high = bounds.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (bounds[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  return bounds[low] ?? Infinity;
}

const holdersIndexCache = new WeakMap<object, readonly number[]>();
const rangesIndexCache = new WeakMap<object, readonly number[]>();
const windowRangesIndexCache = new WeakMap<object, Map<number, readonly number[]>>();
const membershipIndexCache = new WeakMap<object, readonly number[]>();

const mergedRangesCache = new WeakMap<object, readonly (readonly [number, number])[]>();

function freezeRanges(ranges: HourRanges): void {
  for (let i = 0; i < ranges.length; i++) {
    if (!Object.isFrozen(ranges[i])) Object.freeze(ranges[i]);
  }
  if (!Object.isFrozen(ranges)) Object.freeze(ranges);
}

function indexedRanges(ranges: HourRanges): readonly (readonly [number, number])[] {
  let merged = mergedRangesCache.get(ranges);
  if (merged) return merged;
  freezeRanges(ranges);
  const valid: [number, number][] = [];
  for (let i = 0; i < ranges.length; i++) {
    const [first, last] = ranges[i]!;
    if (first <= last) valid.push([first, last]);
  }
  valid.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const out: (readonly [number, number])[] = [];
  for (let i = 0; i < valid.length; i++) {
    const [first, last] = valid[i]!;
    const prev = out[out.length - 1] as [number, number] | undefined;
    if (prev && prev[1] + 1 >= first) {
      if (last > prev[1]) prev[1] = last;
    } else {
      out.push([first, last]);
    }
  }
  for (let i = 0; i < out.length; i++) Object.freeze(out[i]);
  merged = Object.freeze(out);
  mergedRangesCache.set(ranges, merged);
  return merged;
}

export function nextHoldersChange(holders: Readonly<Record<string, readonly HolderRun[]>> | null | undefined, hour: number): number {
  if (!holders) return Infinity;
  let bounds = holdersIndexCache.get(holders);
  if (!bounds) {
    const edges: number[] = [];
    for (const key in holders) {
      const runs = holders[key]!;
      for (let i = 0; i < runs.length; i++) {
        const run = runs[i]!;
        edges.push(run.firstHour, run.lastHour + 1);
        Object.freeze(run);
      }
      Object.freeze(runs);
    }
    Object.freeze(holders);
    bounds = buildChangeIndex(edges);
    holdersIndexCache.set(holders, bounds);
  }
  return nextChangeAfter(bounds, hour);
}

export function nextRangesChange(ranges: HourRanges | null | undefined, hour: number): number {
  if (!ranges || !ranges.length) return Infinity;
  let bounds = rangesIndexCache.get(ranges);
  if (!bounds) {
    const edges: number[] = [];
    for (let i = 0; i < ranges.length; i++) {
      const [first, last] = ranges[i]!;
      edges.push(first, last + 1);
    }
    freezeRanges(ranges);
    bounds = buildChangeIndex(edges);
    rangesIndexCache.set(ranges, bounds);
  }
  return nextChangeAfter(bounds, hour);
}

export function nextWindowRangesChange(ranges: HourRanges | null | undefined, windowHours: number, hour: number): number {
  if (!ranges || !ranges.length) return Infinity;
  let byWindow = windowRangesIndexCache.get(ranges);
  if (!byWindow) {
    freezeRanges(ranges);
    byWindow = new Map();
    windowRangesIndexCache.set(ranges, byWindow);
  }
  let bounds = byWindow.get(windowHours);
  if (!bounds) {
    const edges: number[] = [];
    for (let i = 0; i < ranges.length; i++) {
      const [first, last] = ranges[i]!;
      edges.push(first, last + windowHours);
    }
    bounds = buildChangeIndex(edges);
    byWindow.set(windowHours, bounds);
  }
  return nextChangeAfter(bounds, hour);
}

export function nextMembershipChange(membership: Readonly<Record<string, HourRanges>> | null | undefined, hour: number): number {
  if (!membership) return Infinity;
  let bounds = membershipIndexCache.get(membership);
  if (!bounds) {
    const edges: number[] = [];
    for (const key in membership) {
      const ranges = membership[key]!;
      for (let i = 0; i < ranges.length; i++) {
        const [first, last] = ranges[i]!;
        edges.push(first, last + 1);
      }
      freezeRanges(ranges);
    }
    Object.freeze(membership);
    bounds = buildChangeIndex(edges);
    membershipIndexCache.set(membership, bounds);
  }
  return nextChangeAfter(bounds, hour);
}

export function overlapsHourRanges(ranges: HourRanges | null | undefined, firstHour: number, lastHour: number): boolean {
  if (!ranges || !ranges.length || firstHour > lastHour) return false;
  const merged = indexedRanges(ranges);
  if (!merged.length) return false;
  let low = 0;
  let high = merged.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (merged[mid]![1] < firstHour) low = mid + 1;
    else high = mid;
  }
  return low < merged.length && merged[low]![0] <= lastHour;
}

export function withoutHourRanges(ranges: HourRanges, cut: HourRanges): HourRanges {
  if (!cut.length) return ranges;
  const out: (readonly [number, number])[] = [];
  for (const [first, last] of ranges) {
    let at = first;
    for (const [lo, hi] of cut) {
      if (hi < at || lo > last) continue;
      if (lo > at) out.push([at, lo - 1]);
      at = Math.max(at, hi + 1);
    }
    if (at <= last) out.push([at, last]);
  }
  return out;
}

export function inHourRanges(ranges: HourRanges | null | undefined, hour: number): boolean {
  return overlapsHourRanges(ranges, hour, hour);
}

export interface FamilyFactionRun {
  readonly id: string;
  readonly tag: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly names: readonly { readonly firstHour: number; readonly lastHour: number; readonly name: string | null }[];
}

const factionBoundsCache = new WeakMap<object, readonly number[]>();

export function nextFactionsChange(
  runs: readonly FamilyFactionRun[] | null | undefined,
  hour: number,
  unobserved?: HourRanges | null,
  incomplete?: HourRanges | null,
): number {
  let soonest = Infinity;
  if (runs && runs.length) {
    let bounds = factionBoundsCache.get(runs);
    if (!bounds) {
      const edges: number[] = [];
      const seenFactions = new Set<string>();
      for (let i = 0; i < runs.length; i++) {
        const run = runs[i]!;
        edges.push(run.firstHour, run.lastHour + 1);
        if (!seenFactions.has(run.id)) {
          seenFactions.add(run.id);
          for (let j = 0; j < run.names.length; j++) {
            const nameRun = run.names[j]!;
            edges.push(nameRun.firstHour, nameRun.lastHour + 1);
            Object.freeze(nameRun);
          }
          Object.freeze(run.names);
        }
        Object.freeze(run);
      }
      Object.freeze(runs);
      bounds = buildChangeIndex(edges);
      factionBoundsCache.set(runs, bounds);
    }
    soonest = Math.min(soonest, nextChangeAfter(bounds, hour));
  }
  if (unobserved?.length) soonest = Math.min(soonest, nextRangesChange(unobserved, hour));
  if (incomplete?.length) soonest = Math.min(soonest, nextRangesChange(incomplete, hour));
  return soonest;
}
