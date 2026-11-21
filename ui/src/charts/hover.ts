import { keyStops } from './frame.ts';
import type { Range } from './view.ts';

export interface HoverState<P, K = number> {
  pointer: P | null;
  keyed: K | null;
  menuKey: boolean;
}

export type HoverEvent<P, K = number> =
  | { type: 'pointer'; point: P }
  | { type: 'pointer-leave' }
  | { type: 'key'; hour: K | null }
  | { type: 'key-clear' }
  | { type: 'menu-key' }
  | { type: 'pointer-down' }
  | { type: 'contextmenu' };

export const NO_HOVER: HoverState<never, never> = { pointer: null, keyed: null, menuKey: false };

export function reduceHover<P, K = number>(state: HoverState<P, K>, event: HoverEvent<P, K>): HoverState<P, K> {
  switch (event.type) {
    case 'pointer': return { ...state, pointer: event.point, keyed: null };
    case 'pointer-leave': return state.pointer == null ? state : { ...state, pointer: null };
    case 'key': return { ...state, pointer: null, keyed: event.hour };
    case 'key-clear': return state.keyed == null ? state : { ...state, keyed: null };
    case 'menu-key': return { ...state, menuKey: true };
    case 'pointer-down':
    case 'contextmenu': return state.menuKey ? { ...state, menuKey: false } : state;
  }
}

export const pointerMenu = (state: Pick<HoverState<unknown>, 'menuKey'>) => !state.menuKey;

export function hoverAt<P, A extends { hour: number }>(state: HoverState<P>, x: Range, until: number | null,
  pointed: (point: P) => A | null): ((A | { hour: number }) & { by: 'pointer' | 'keyboard' }) | null {
  if (state.pointer != null) {
    const at = pointed(state.pointer);
    return at ? { ...at, by: 'pointer' } : null;
  }
  const key = state.keyed;
  return key != null && key >= x[0] && key <= Math.min(x[1], until ?? Infinity) ? { hour: key, by: 'keyboard' } : null;
}

export function pointerTarget(zone: 'plot' | 'strip', hour: number | null, view: Range, until: number | null, extent: Range,
  hooks: { snap?: (hour: number) => number; describe?: (hour: number) => number } = {}): { at: number; describes: number } | null {
  if (hour == null) return null;
  if (zone === 'plot') {
    const snapped = hooks.snap?.(hour) ?? hour;
    const inView = snapped >= view[0] && snapped <= Math.min(view[1], until ?? Infinity);
    const at = inView ? snapped : hour;
    return { at, describes: at };
  }
  const about = hooks.describe?.(hour) ?? hour;
  const known = about >= extent[0] && about <= Math.min(extent[1], until ?? Infinity);
  return { at: hour, describes: known ? about : hour };
}

export interface KeyTarget { at: number; describes: number }

export interface KeyStop extends KeyTarget { id: string }

export function keyStopList(keyHours: readonly number[] | undefined, targets: readonly KeyTarget[], first: number, last: number): KeyStop[] | null {
  if (!keyHours && !targets.length) return null;
  if (last < first) return [];
  const own = keyStops(keyHours ?? [], first, last).map(hour => ({ id: `key@${hour}`, at: hour, describes: hour }));
  const ownHours = new Set(own.map(stop => stop.at));
  const strip = targets
    .filter(target => target.at >= first && target.at <= last)
    .filter(target => !(target.describes === target.at && ownHours.has(target.at)))
    .map(target => ({ id: `strip@${target.at}>${target.describes}`, ...target }));
  return [...own, ...strip].sort((a, b) => a.at - b.at);
}

export function keyStopMemo() {
  let cached: { keyHours: unknown; strip: unknown; view: Range; until: number | null; stops: KeyStop[] | null } | null = null;
  return (keyHours: readonly number[] | undefined, strip: object | undefined, targets: () => readonly KeyTarget[],
    view: Range, until: number | null): KeyStop[] | null => {
    if (cached && cached.keyHours === keyHours && cached.strip === strip && cached.view[0] === view[0] && cached.view[1] === view[1] && cached.until === until) {
      return cached.stops;
    }
    const stops = keyStopList(keyHours, targets(), view[0], Math.min(view[1], until ?? view[1]));
    cached = { keyHours, strip, view: [view[0], view[1]], until, stops };
    return stops;
  };
}

export function keyedStop(key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', current: { id?: string; at: number } | null,
  stops: readonly KeyStop[]): KeyStop | null {
  if (!stops.length) return null;
  if (key === 'Home') return stops[0];
  if (key === 'End' || current == null) return stops.at(-1)!;
  const index = current.id == null ? -1 : stops.findIndex(stop => stop.id === current.id);
  if (index >= 0) return stops[key === 'ArrowRight' ? Math.min(stops.length - 1, index + 1) : Math.max(0, index - 1)];
  if (key === 'ArrowRight') return stops.find(stop => stop.at > current.at) ?? stops.filter(stop => stop.at <= current.at).at(-1)!;
  return stops.filter(stop => stop.at < current.at).at(-1) ?? stops.find(stop => stop.at >= current.at)!;
}

export function actionHour(shown: number | null, state: HoverState<unknown>, playhead: number | null): number | null {
  return shown ?? state.keyed ?? playhead;
}
