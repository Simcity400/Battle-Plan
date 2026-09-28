import type { Range } from './view.ts';

/** Who drives a chart's hover. The pointer drives it while it moves over the chart.
 * A navigation key hands it to the keyboard and clears the pointer's hover, so the
 * hour drawn, its tooltip, what is read out and what the keys act on are one hour;
 * moving the pointer again takes it back. */
export interface HoverState<P> {
  pointer: P | null;
  keyed: number | null;
  /** The keys opened a context menu. The browser follows the menu key (or Shift+F10)
   * with its own contextmenu event, at a place of its choosing; that event must not
   * open a second menu at whatever hour lies there. */
  menuKey: boolean;
}

export type HoverEvent<P> =
  /** The pointer moved over the chart. */
  | { type: 'pointer'; point: P }
  /** The pointer left the chart. */
  | { type: 'pointer-leave' }
  /** A navigation key chose an hour (null: nothing in view to show). */
  | { type: 'key'; hour: number | null }
  /** Escape, or the chart lost focus. */
  | { type: 'key-clear' }
  /** The menu key (or Shift+F10) opened the context menu at the hour shown. */
  | { type: 'menu-key' }
  /** A pointer button went down: any context menu after it is the pointer's. */
  | { type: 'pointer-down' }
  /** The browser's contextmenu event arrived. */
  | { type: 'contextmenu' };

export const NO_HOVER: HoverState<never> = { pointer: null, keyed: null, menuKey: false };

export function reduceHover<P>(state: HoverState<P>, event: HoverEvent<P>): HoverState<P> {
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

/** Whether a contextmenu event is the pointer's own (a right-click), to open a menu
 * at the hour under it; else it only follows a menu the keys already opened. */
export const pointerMenu = (state: HoverState<unknown>) => !state.menuKey;

/** What the hover shows: whatever the pointer is over (`pointed`) while it drives,
 * else the keyboard's hour while it is in view and not after `until`. */
export function hoverAt<P, A extends { hour: number }>(state: HoverState<P>, x: Range, until: number | null,
  pointed: (point: P) => A | null): ((A | { hour: number }) & { by: 'pointer' | 'keyboard' }) | null {
  if (state.pointer != null) {
    const at = pointed(state.pointer);
    return at ? { ...at, by: 'pointer' } : null;
  }
  const key = state.keyed;
  return key != null && key >= x[0] && key <= Math.min(x[1], until ?? Infinity) ? { hour: key, by: 'keyboard' } : null;
}

/** The hour the keys act on (the context-menu key, Enter): the hour the hover shows,
 * else the keyboard's hour, else the playhead. */
export function actionHour(shown: number | null, state: HoverState<unknown>, playhead: number | null): number | null {
  return shown ?? state.keyed ?? playhead;
}
