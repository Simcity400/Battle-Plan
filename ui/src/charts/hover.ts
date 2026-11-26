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

export function actionHour(shown: number | null, state: HoverState<unknown>, playhead: number | null): number | null {
  return shown ?? state.keyed ?? playhead;
}
