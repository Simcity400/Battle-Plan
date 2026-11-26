import { entryAt } from '../../dashboard/reads.ts';
import type { WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { labelObservationAt } from '../wars/warsData.ts';

export const WAR_NOT_STARTED = 'The selected war has not started at this date.';
export const WAR_UNAVAILABLE = 'War not recorded at this time.';

export interface WarChipState {
  readonly at: number | null;
  readonly war: WarTimeline | null;
  readonly observation: WarObservation | null;
  readonly prestart: boolean;
  readonly unavailable: boolean;
  readonly valid: readonly [number, number];
}

const warsByIdCache = new WeakMap<WarsReply, ReadonlyMap<string, WarTimeline>>();

export function warById(wars: WarsReply, saved: string | null): WarTimeline | null {
  if (!saved) return null;
  let byId = warsByIdCache.get(wars);
  if (!byId) {
    const map = new Map<string, WarTimeline>();
    for (const war of wars.wars) map.set(war.id, war);
    byId = map;
    warsByIdCache.set(wars, byId);
  }
  return byId.get(saved) ?? null;
}

export function chosenWar(reply: WarsReply, saved: string | null, at: number): WarTimeline | null {
  const explicit = warById(reply, saved);
  if (explicit) return explicit.firstHour <= at ? explicit : null;
  let best: { war: WarTimeline; active: boolean; count: number; last: number } | null = null;
  for (const war of reply.wars) {
    const observation = entryAt(war.observations, at);
    if (!observation) continue;
    const active = at <= observation.lastHour;
    const last = Math.min(at, observation.lastHour);
    const count = new Set([...observation.viewpointSide, ...observation.opposingSide]).size;
    const better = !best || (active !== best.active ? active : count !== best.count ? count > best.count
      : last !== best.last ? last > best.last : Number(war.id) < Number(best.war.id));
    if (better) best = { war, active, count, last };
  }
  return best?.war ?? null;
}

export const readHourFor = (war: WarTimeline, at: number) => Math.min(Math.floor(at), war.lastHour);

const PENDING_CHIP: WarChipState = Object.freeze({
  at: null,
  war: null,
  observation: null,
  prestart: false,
  unavailable: false,
  valid: [-Infinity, Infinity] as const,
});

function sortedUnique(raw: number[]): readonly number[] {
  raw.sort((a, b) => a - b);
  const unique: number[] = [];
  for (const value of raw) if (unique[unique.length - 1] !== value) unique.push(value);
  return unique;
}

function upperEdgeIndex(edges: readonly number[], hour: number): number {
  let low = 0;
  let high = edges.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (edges[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  return low;
}

function spanFromEdges(edges: readonly number[], hour: number): readonly [number, number] {
  if (!edges.length) return [-Infinity, Infinity];
  const idx = upperEdgeIndex(edges, hour);
  return [idx > 0 ? edges[idx - 1]! : -Infinity, idx < edges.length ? edges[idx]! - 1 : Infinity];
}

const warEdgesCache = new WeakMap<WarTimeline, readonly number[]>();
const warGappedEdgesCache = new WeakMap<WarsReply, WeakMap<WarTimeline, readonly number[]>>();
const chipEdgesCache = new WeakMap<WarsReply, readonly number[]>();
const menuStartsCache = new WeakMap<WarsReply, readonly number[]>();

interface EligibilitySegment {
  readonly from: number;
  readonly until: number;
  readonly mode: WarEligibilityMode;
}

const eligibilityCache = new WeakMap<WarsReply, Map<string, readonly EligibilitySegment[]>>();

export function unobservedAt(wars: WarsReply, hour: number): boolean {
  const spans = wars.unobserved;
  if (!spans?.length) return false;
  const span = entryAt(spans, hour);
  return span !== null && hour <= span.lastHour;
}

function warEdgesOf(war: WarTimeline, wars?: WarsReply): readonly number[] {
  const gaps = wars?.unobserved;
  if (!gaps?.length || !wars) {
    let edges = warEdgesCache.get(war);
    if (!edges) {
      const raw: number[] = [war.firstHour, war.lastHour + 1];
      for (const obs of war.observations) {
        raw.push(obs.firstHour, obs.lastHour + 1);
      }
      edges = sortedUnique(raw);
      warEdgesCache.set(war, edges);
    }
    return edges;
  }
  let byWar = warGappedEdgesCache.get(wars);
  if (!byWar) warGappedEdgesCache.set(wars, byWar = new WeakMap());
  let edges = byWar.get(war);
  if (!edges) {
    const raw: number[] = [war.firstHour, war.lastHour + 1];
    for (const obs of war.observations) raw.push(obs.firstHour, obs.lastHour + 1);
    for (const span of gaps) raw.push(span.firstHour, span.lastHour + 1);
    edges = sortedUnique(raw);
    byWar.set(war, edges);
  }
  return edges;
}

function chipEdgesOf(wars: WarsReply): readonly number[] {
  let edges = chipEdgesCache.get(wars);
  if (!edges) {
    const raw: number[] = [];
    for (const item of wars.wars) {
      raw.push(item.lastHour + 1);
      for (const obs of item.observations) {
        raw.push(obs.firstHour, obs.lastHour + 1);
      }
    }
    for (const span of wars.unobserved ?? []) {
      raw.push(span.firstHour, span.lastHour + 1);
    }
    edges = sortedUnique(raw);
    chipEdgesCache.set(wars, edges);
  }
  return edges;
}

function menuStartsOf(wars: WarsReply): readonly number[] {
  let starts = menuStartsCache.get(wars);
  if (!starts) {
    const raw: number[] = [];
    for (const war of wars.wars) for (const obs of war.observations) raw.push(obs.firstHour);
    starts = sortedUnique(raw);
    menuStartsCache.set(wars, starts);
  }
  return starts;
}

export function warChipStateAt(wars: WarsReply | undefined, saved: string | null, at: number | null): WarChipState {
  if (!wars || at === null || !Number.isFinite(at)) return PENDING_CHIP;
  const hour = Math.floor(at);
  const explicit = warById(wars, saved);
  if (explicit) {
    const valid = spanFromEdges(warEdgesOf(explicit, wars), hour);
    if (unobservedAt(wars, hour)) {
      return { at: hour, war: null, observation: null, prestart: false, unavailable: true, valid };
    }
    if (hour < explicit.firstHour) {
      return { at: hour, war: null, observation: null, prestart: true, unavailable: false, valid };
    }
    if (hour > explicit.lastHour) {
      return {
        at: hour,
        war: explicit,
        observation: explicit.observations.at(-1) ?? null,
        prestart: false,
        unavailable: false,
        valid,
      };
    }
    const observation = labelObservationAt(explicit, hour);
    return {
      at: hour,
      war: explicit,
      observation,
      prestart: false,
      unavailable: observation === null,
      valid,
    };
  }
  const valid = spanFromEdges(chipEdgesOf(wars), hour);
  if (unobservedAt(wars, hour)) {
    return { at: hour, war: null, observation: null, prestart: false, unavailable: true, valid };
  }
  const war = chosenWar(wars, null, hour);
  const observation = war ? labelObservationAt(war, hour) : null;
  return {
    at: hour,
    war,
    observation,
    prestart: false,
    unavailable: war !== null && observation === null,
    valid,
  };
}

export type WarEligibilityMode = 'none' | 'live' | 'ended';

interface InactiveRank {
  readonly warIdx: number;
  readonly version: number;
  readonly count: number;
  readonly last: number;
  readonly id: number;
  readonly inGap: boolean;
}

function betterInactive(a: InactiveRank, b: InactiveRank): boolean {
  if (a.count !== b.count) return a.count > b.count;
  if (a.last !== b.last) return a.last > b.last;
  return a.id < b.id;
}

function heapPush(heap: InactiveRank[], item: InactiveRank): void {
  let at = heap.push(item) - 1;
  while (at > 0) {
    const up = (at - 1) >> 1;
    if (!betterInactive(heap[at]!, heap[up]!)) break;
    [heap[at], heap[up]] = [heap[up]!, heap[at]!];
    at = up;
  }
}

function heapPop(heap: InactiveRank[]): void {
  const last = heap.pop();
  if (!heap.length || !last) return;
  heap[0] = last;
  for (let at = 0;;) {
    const left = at * 2 + 1;
    const right = left + 1;
    let best = at;
    if (left < heap.length && betterInactive(heap[left]!, heap[best]!)) best = left;
    if (right < heap.length && betterInactive(heap[right]!, heap[best]!)) best = right;
    if (best === at) return;
    [heap[at], heap[best]] = [heap[best]!, heap[at]!];
    at = best;
  }
}

interface SweepEvent {
  readonly hour: number;
  readonly unobserved: number;
  readonly active: number;
  readonly warIdx: number;
  readonly rank?: Omit<InactiveRank, 'warIdx' | 'version'>;
}

function eligibilitySegmentsOf(wars: WarsReply, saved: string | null): readonly EligibilitySegment[] {
  const explicit = warById(wars, saved);
  const cacheKey = explicit ? explicit.id : '';
  let byKey = eligibilityCache.get(wars);
  if (!byKey) {
    byKey = new Map();
    eligibilityCache.set(wars, byKey);
  }
  let segments = byKey.get(cacheKey);
  if (!segments) {
    const scope = explicit ? [explicit] : wars.wars;
    const events: SweepEvent[] = [];
    for (const span of wars.unobserved ?? []) {
      events.push({ hour: span.firstHour, unobserved: 1, active: 0, warIdx: -1 });
      events.push({ hour: span.lastHour + 1, unobserved: -1, active: 0, warIdx: -1 });
    }
    for (let warIdx = 0; warIdx < scope.length; warIdx++) {
      const war = scope[warIdx]!;
      const obs = war.observations;
      const id = Number(war.id);
      for (let i = 0; i < obs.length; i++) {
        const cur = obs[i]!;
        const count = new Set([...cur.viewpointSide, ...cur.opposingSide]).size;
        const isLast = i === obs.length - 1;
        const inGap = !isLast || cur.lastHour < war.lastHour;
        events.push({ hour: cur.firstHour, unobserved: 0, active: 1, warIdx });
        events.push({
          hour: cur.lastHour + 1,
          unobserved: 0,
          active: -1,
          warIdx,
          rank: { count, last: cur.lastHour, id, inGap },
        });
      }
    }
    events.sort((a, b) => a.hour - b.hour);

    const built: EligibilitySegment[] = [];
    const push = (from: number, until: number, mode: WarEligibilityMode) => {
      if (from > until) return;
      const last = built[built.length - 1];
      if (last && last.mode === mode && last.until + 1 === from) {
        built[built.length - 1] = { from: last.from, until, mode };
      } else {
        built.push({ from, until, mode });
      }
    };

    const version = new Int32Array(scope.length);
    const inactive: InactiveRank[] = [];
    let unobservedCount = 0;
    let activeCount = 0;
    let cursor = -Infinity;

    const currentMode = (): WarEligibilityMode => {
      if (unobservedCount > 0) return 'none';
      if (activeCount > 0) return 'live';
      while (inactive.length && inactive[0]!.version !== version[inactive[0]!.warIdx]) heapPop(inactive);
      const top = inactive[0];
      if (!top) return 'none';
      return top.inGap ? 'none' : 'ended';
    };

    for (let i = 0; i < events.length;) {
      const hour = events[i]!.hour;
      if (cursor < hour) {
        push(cursor, hour - 1, currentMode());
        cursor = hour;
      }
      while (i < events.length && events[i]!.hour === hour) {
        const ev = events[i]!;
        unobservedCount += ev.unobserved;
        activeCount += ev.active;
        if (ev.warIdx >= 0) {
          const nextVer = ++version[ev.warIdx]!;
          if (ev.rank) heapPush(inactive, { warIdx: ev.warIdx, version: nextVer, ...ev.rank });
        }
        i++;
      }
    }
    push(cursor, Infinity, currentMode());

    segments = built;
    byKey.set(cacheKey, segments);
  }
  return segments;
}

export function warNextChangeAt(wars: WarsReply | undefined, saved: string | null, at: number): number {
  if (!wars || !wars.available || !Number.isFinite(at)) return Infinity;
  const hour = Math.floor(at);
  const explicit = warById(wars, saved);
  const until = spanFromEdges(explicit ? warEdgesOf(explicit, wars) : chipEdgesOf(wars), hour)[1];
  return Number.isFinite(until) ? until + 1 : Infinity;
}

export function warEligibilityAt(
  wars: WarsReply | undefined,
  saved: string | null,
  at: number,
): { readonly mode: WarEligibilityMode; readonly valid: readonly [number, number] } {
  if (!wars || !wars.available || !wars.wars.length || !Number.isFinite(at)) {
    return { mode: 'none', valid: [-Infinity, Infinity] };
  }
  const hour = Math.floor(at);
  const segments = eligibilitySegmentsOf(wars, saved);
  let low = 0;
  let high = segments.length - 1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    const seg = segments[mid]!;
    if (hour < seg.from) high = mid - 1;
    else if (hour > seg.until) low = mid + 1;
    else return { mode: seg.mode, valid: [seg.from, seg.until] };
  }
  return { mode: 'none', valid: [-Infinity, Infinity] };
}

export function createWarChipTracker(
  wars: WarsReply | undefined,
  saved: string | null,
  shownHour: (clockHour: number) => number | null,
  offset: number,
  compute: (wars: WarsReply | undefined, saved: string | null, at: number | null) => WarChipState = warChipStateAt,
) {
  let cached: WarChipState | null = null;
  const read = (clockHour: number): WarChipState => {
    if (!wars) return cached ??= compute(undefined, saved, null);
    const raw = shownHour(clockHour);
    const at = raw !== null && Number.isFinite(raw) ? Math.floor(raw) : null;
    if (cached) {
      if (at === null && cached.at === null) return cached;
      if (at !== null && cached.at !== null && at >= cached.valid[0] && at <= cached.valid[1]) return cached;
    }
    return cached = compute(wars, saved, at);
  };
  const span = (axis: number): readonly [number, number] => {
    const state = read(axis);
    return [state.valid[0] - offset, state.valid[1] + 1 - offset];
  };
  return { read, span };
}

export function warChipLeaderHour(
  observation: Pick<WarObservation, 'lastHour'> | null,
  clockHour: number,
  shownHour: (clockHour: number) => number | null,
): number | null {
  if (!observation) return null;
  const shown = shownHour(clockHour);
  return shown === null || !Number.isFinite(shown) ? null : Math.min(Math.floor(shown), observation.lastHour);
}

export function warMenuSpanAt(wars: WarsReply | undefined, at: number): readonly [number, number] {
  if (!wars || !Number.isFinite(at)) return [-Infinity, Infinity];
  return spanFromEdges(menuStartsOf(wars), Math.floor(at));
}

export function createWarMenuTracker(
  wars: WarsReply | undefined,
  shownHour: (clockHour: number) => number | null,
  offset: number,
) {
  let cached: { readonly at: number; readonly valid: readonly [number, number] } | null = null;
  const resolve = (clockHour: number) => {
    const raw = shownHour(clockHour);
    const hour = raw !== null && Number.isFinite(raw) ? Math.floor(raw) : -Infinity;
    if (cached && hour >= cached.valid[0] && hour <= cached.valid[1]) return cached;
    const valid = warMenuSpanAt(wars, hour);
    return cached = { at: valid[0], valid };
  };
  const read = (clockHour: number): number => resolve(clockHour).at;
  const span = (axis: number): readonly [number, number] => {
    const state = resolve(axis);
    return [state.valid[0] - offset, state.valid[1] + 1 - offset];
  };
  return { read, span };
}

export const warName = (observation: WarObservation | null) => observation?.name.known ? observation.name.text : 'Name not recorded';

export function warChoicesAt(wars: WarsReply | undefined, at: number, saved: string | null = null) {
  const choices: { id: string; firstHour: number | null; name: string }[] = [];
  for (const war of wars?.wars ?? []) {
    const observation = entryAt(war.observations, at);
    if (observation) choices.push({ id: war.id, firstHour: war.firstHour, name: warName(observation) });
    else if (war.id === saved) choices.push({ id: war.id, firstHour: null, name: WAR_NOT_STARTED });
  }
  return choices;
}
