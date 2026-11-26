import { entryAt } from '../../dashboard/reads.ts';
import type { WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { labelObservationAt, observationAt } from '../wars/warsData.ts';

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

export function chosenWar(reply: WarsReply, saved: string | null, at: number): WarTimeline | null {
  if (saved) {
    const war = reply.wars.find(item => item.id === saved);
    if (war) return war.firstHour <= at ? war : null;
  }
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
  const explicit = saved ? wars.wars.find(item => item.id === saved) ?? null : null;
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

interface SweepEvent {
  readonly hour: number;
  readonly unobserved: number;
  readonly active: number;
  readonly started: number;
  readonly gap: number;
}

function eligibilitySegmentsOf(wars: WarsReply, saved: string | null): readonly EligibilitySegment[] {
  const explicit = saved ? wars.wars.find(item => item.id === saved) ?? null : null;
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
      events.push({ hour: span.firstHour, unobserved: 1, active: 0, started: 0, gap: 0 });
      events.push({ hour: span.lastHour + 1, unobserved: -1, active: 0, started: 0, gap: 0 });
    }
    for (const war of scope) {
      const obs = war.observations;
      if (!obs.length) continue;
      events.push({ hour: obs[0]!.firstHour, unobserved: 0, active: 0, started: 1, gap: 0 });
      for (let i = 0; i < obs.length; i++) {
        const cur = obs[i]!;
        events.push({ hour: cur.firstHour, unobserved: 0, active: 1, started: 0, gap: 0 });
        events.push({ hour: cur.lastHour + 1, unobserved: 0, active: -1, started: 0, gap: 0 });
        const next = obs[i + 1];
        if (next && cur.lastHour + 1 < next.firstHour) {
          events.push({ hour: cur.lastHour + 1, unobserved: 0, active: 0, started: 0, gap: 1 });
          events.push({ hour: next.firstHour, unobserved: 0, active: 0, started: 0, gap: -1 });
        }
      }
      const tail = obs[obs.length - 1]!;
      if (tail.lastHour < war.lastHour) {
        events.push({ hour: tail.lastHour + 1, unobserved: 0, active: 0, started: 0, gap: 1 });
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

    let unobservedCount = 0;
    let activeCount = 0;
    let startedCount = 0;
    let gapCount = 0;
    let cursor = -Infinity;

    const modeAt = (sample: number): WarEligibilityMode => {
      if (unobservedCount > 0 || startedCount === 0) return 'none';
      if (activeCount > 0) return 'live';
      if (gapCount === 0) return 'ended';
      const war = chosenWar(wars, explicit ? explicit.id : null, sample);
      if (!war) return 'none';
      return observationAt(war, readHourFor(war, sample)) !== null ? 'ended' : 'none';
    };

    for (let i = 0; i < events.length;) {
      const hour = events[i]!.hour;
      if (cursor < hour) {
        const sample = Number.isFinite(cursor) ? cursor : hour - 1;
        push(cursor, hour - 1, modeAt(sample));
        cursor = hour;
      }
      while (i < events.length && events[i]!.hour === hour) {
        const ev = events[i]!;
        unobservedCount += ev.unobserved;
        activeCount += ev.active;
        startedCount += ev.started;
        gapCount += ev.gap;
        i++;
      }
    }
    push(cursor, Infinity, modeAt(Number.isFinite(cursor) ? cursor : 0));

    segments = built;
    byKey.set(cacheKey, segments);
  }
  return segments;
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
