import { entryAt } from '../../dashboard/reads.ts';
import type { WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { labelObservationAt } from '../wars/warsData.ts';

export const WAR_NOT_STARTED = 'The selected war has not started at this date.';

export interface WarChipState {
  readonly at: number | null;
  readonly war: WarTimeline | null;
  readonly observation: WarObservation | null;
  readonly prestart: boolean;
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
  valid: [-Infinity, Infinity] as const,
});

export function warChipStateAt(wars: WarsReply | undefined, saved: string | null, at: number | null): WarChipState {
  if (!wars || at === null || !Number.isFinite(at)) return PENDING_CHIP;
  const hour = Math.floor(at);
  const explicit = saved ? wars.wars.find(item => item.id === saved) ?? null : null;
  if (explicit) {
    if (hour < explicit.firstHour) {
      return { at: hour, war: null, observation: null, prestart: true, valid: [-Infinity, explicit.firstHour - 1] };
    }
    if (hour > explicit.lastHour) {
      const observation = explicit.observations.at(-1) ?? null;
      return {
        at: hour,
        war: explicit,
        observation,
        prestart: false,
        valid: [explicit.lastHour + 1, Infinity],
      };
    }
    const observation = labelObservationAt(explicit, hour);
    let lo = explicit.firstHour;
    let hi = explicit.lastHour;
    if (observation) {
      lo = observation.firstHour;
      hi = observation.lastHour;
    } else {
      for (const item of explicit.observations) {
        if (item.lastHour < hour) lo = Math.max(lo, item.lastHour + 1);
        else if (item.firstHour > hour) hi = Math.min(hi, item.firstHour - 1);
      }
    }
    return {
      at: hour,
      war: explicit,
      observation,
      prestart: false,
      valid: [lo, hi],
    };
  }
  const war = chosenWar(wars, null, hour);
  const observation = war ? labelObservationAt(war, hour) : null;
  let lo = -Infinity;
  let hi = Infinity;
  const bound = (edge: number) => {
    if (edge <= hour) {
      if (edge > lo) lo = edge;
    } else if (edge - 1 < hi) {
      hi = edge - 1;
    }
  };
  for (const item of wars.wars) {
    bound(item.lastHour + 1);
    for (const obs of item.observations) {
      bound(obs.firstHour);
      bound(obs.lastHour + 1);
    }
  }
  return {
    at: hour,
    war,
    observation,
    prestart: false,
    valid: [lo, hi],
  };
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

const menuStartsCache = new WeakMap<WarsReply, readonly number[]>();

function menuStartsOf(wars: WarsReply): readonly number[] {
  let starts = menuStartsCache.get(wars);
  if (!starts) {
    const raw: number[] = [];
    for (const war of wars.wars) for (const obs of war.observations) raw.push(obs.firstHour);
    raw.sort((a, b) => a - b);
    const unique: number[] = [];
    for (const hour of raw) if (unique[unique.length - 1] !== hour) unique.push(hour);
    starts = unique;
    menuStartsCache.set(wars, starts);
  }
  return starts;
}

export function warMenuSpanAt(wars: WarsReply | undefined, at: number): readonly [number, number] {
  if (!wars || !Number.isFinite(at)) return [-Infinity, Infinity];
  const starts = menuStartsOf(wars);
  if (!starts.length) return [-Infinity, Infinity];
  const hour = Math.floor(at);
  let low = 0;
  let high = starts.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (starts[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  return [low > 0 ? starts[low - 1]! : -Infinity, low < starts.length ? starts[low]! - 1 : Infinity];
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
