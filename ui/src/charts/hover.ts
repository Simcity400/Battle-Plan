import { keyStops } from './frame.ts';
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

/** Where a pointer hover sits (`at`, the crosshair's hour) and the hour it is about
 * (`describes`: its tooltip, Jump to and pick). `hour` is the hour under the pointer,
 * already kept in view and not after `until`; null shows nothing.
 *
 * - Over the plot, the chart's `snap` may move both onto a nearby mark (an event
 *   tick), but only where the pointer itself could go: in view and not after `until`.
 * - Over a strip, the plot's marks do not apply. The strip's `describe` names what the
 *   item drawn under the pointer is about, such as a bar's end where its amounts were read,
 *   which may lie outside the view when the bar is cut by a zoom. The hover is about
 *   that hour while the crosshair stays at the pointer, provided it is inside `extent`
 *   and not after `until` (a bar ending later is not drawn). */
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

/** A keyboard stop as the pointer would resolve it: where the crosshair sits (`at`,
 * in view) and the hour it is about, such as a strip bar's read past the view's end. */
export interface KeyTarget { at: number; describes: number }

/** A keyboard stop with an identity: two can share an hour (a read, and a strip bar
 * the view cuts off there), and each must be reachable. */
export interface KeyStop extends KeyTarget { id: string }

/** The stops the keyboard steps through between `first` and `last` (the view's
 * start and its last hour shown): each of a chart's key hours (as `keyStops` keeps
 * them, a value carried into the view included), about itself, then each strip
 * target in view that is not the same stop, in hour order; at one hour the chart's
 * own stop comes first. Null when the chart has neither, so the keys step a day.
 * Linear in the stops plus a sort: a campaign's thousands of daily reads cost
 * nothing noticeable per key press. */
export function keyStopList(keyHours: readonly number[] | undefined, targets: readonly KeyTarget[], first: number, last: number): KeyStop[] | null {
  if (!keyHours && !targets.length) return null;
  if (last < first) return [];
  const own = keyStops(keyHours ?? [], first, last).map(hour => ({ id: `key@${hour}`, at: hour, describes: hour }));
  const ownHours = new Set(own.map(stop => stop.at));
  const strip = targets
    .filter(target => target.at >= first && target.at <= last)
    .filter(target => !(target.describes === target.at && ownHours.has(target.at)))
    .map(target => ({ id: `strip@${target.at}>${target.describes}`, ...target }));
  // A stable sort keeps the chart's own stop before a strip stop at the same hour.
  return [...own, ...strip].sort((a, b) => a.at - b.at);
}

/** `keyStopList` for a chart, built again only when its inputs change: the key
 * hours and the strip (by identity), the view and the last hour drawn. Each key
 * press asks for it, and between presses nothing changes. */
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

/** The stop a key moves the keyboard hover to: Home and End the first and last, the
 * first press the last; an arrow the next or previous stop, one at a time even
 * within an hour. A hover without a stop of these (the pointer's, or one the view
 * has moved past) steps on from its hour. Null without stops. */
export function keyedStop(key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', current: { id?: string; at: number } | null,
  stops: readonly KeyStop[]): KeyStop | null {
  if (!stops.length) return null;
  if (key === 'Home') return stops[0];
  if (key === 'End' || current == null) return stops.at(-1)!;
  const index = current.id == null ? -1 : stops.findIndex(stop => stop.id === current.id);
  if (index >= 0) return stops[key === 'ArrowRight' ? Math.min(stops.length - 1, index + 1) : Math.max(0, index - 1)];
  // At either end the hover stays on the nearest stop shown.
  if (key === 'ArrowRight') return stops.find(stop => stop.at > current.at) ?? stops.filter(stop => stop.at <= current.at).at(-1)!;
  return stops.filter(stop => stop.at < current.at).at(-1) ?? stops.find(stop => stop.at >= current.at)!;
}

/** The hour the keys act on (the context-menu key, Enter): the hour the hover shows,
 * else the keyboard's hour, else the playhead. */
export function actionHour(shown: number | null, state: HoverState<unknown>, playhead: number | null): number | null {
  return shown ?? state.keyed ?? playhead;
}
