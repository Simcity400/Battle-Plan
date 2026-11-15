import { startTransition, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { RefObject } from 'react';
import { perfLog } from '../../shell/perf.ts';
import type { PlaybackController } from '../playback';
import { atNextSlot, pagePace, structureOpen } from '../structurePace.ts';
import type { PaceClock } from '../structurePace';
import { countWork, measuredWidget } from '../widgetPerf.ts';
import { PreparingPage } from '../preparing.ts';
import type { FollowedAnswer } from './message.ts';
import { canonical } from './store.ts';
import type { FollowEntry, FollowHandle, FollowQuery, FollowStore, Painter } from './store.ts';

/** Tells `changed` whether `element` is on screen, now and as it changes; the returned function
 * stops. */
export type OnScreen = (element: Element, changed: (visible: boolean) => void) => () => void;

/** How near the dashboard's scrolled view a widget is active: its follows are held over the same reach, so an active
 * widget never asks for its head values by the fallback because its follow is off. */
export const NEAR_SCREEN = '240px 0px';
export const DASHBOARD_SCROLL = '.dash-grid-scroll';

/** On screen as the browser sees it, or near the dashboard's view as its widgets are active there; always, where there
 * is no `IntersectionObserver`. An element with no box of its own (`display: contents`) is never seen by the browser:
 * it is on screen while any of its children is, the children watched as they come and go. */
export const observeOnScreen: OnScreen = (element, changed) => {
  if (typeof IntersectionObserver !== 'function') { changed(true); return () => {}; }
  const root = element.closest(DASHBOARD_SCROLL);
  const reach: IntersectionObserverInit | undefined = root ? { root, rootMargin: NEAR_SCREEN } : undefined;
  if (getComputedStyle(element).display !== 'contents') {
    const observer = new IntersectionObserver(entries => { for (const entry of entries) changed(entry.isIntersecting); }, reach);
    observer.observe(element);
    return () => observer.disconnect();
  }
  const seen = new Map<Element, boolean>();
  let told: boolean | null = null;
  const tell = () => {
    const visible = [...seen.values()].some(Boolean);
    if (visible !== told) { told = visible; changed(visible); }
  };
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) if (seen.has(entry.target)) seen.set(entry.target, entry.isIntersecting);
    tell();
  }, reach);
  // A child taking another's place is told by its own first sighting, so a loading state giving way keeps the follow.
  const watch = () => {
    const children = new Set(element.children);
    let added = false;
    for (const child of [...seen.keys()]) if (!children.has(child)) { observer.unobserve(child); seen.delete(child); }
    for (const child of children) if (!seen.has(child)) { seen.set(child, false); observer.observe(child); added = true; }
    if (!added && told !== null) tell();
  };
  watch();
  const children = new MutationObserver(watch);
  children.observe(element, { childList: true });
  return () => { children.disconnect(); observer.disconnect(); };
};

const noStructure = () => '';

/** A text panel's structure: its text, which moves the entry's version only when it differs. */
export const panelStructure = (_answer: FollowedAnswer | undefined, entry: FollowEntry) => entry.panel ? String(entry.version) : '';

/** A followed query as a widget renders it: whether the service follows it, and the answer's structure. */
export interface Followed {
  /** Off while it is not on screen or not asked; the widget reads the value as before unless followed. */
  status: 'off' | FollowEntry['status'];
  reason?: string;
  structure: string;
  /** The followed entry, while one is held. */
  entry: FollowEntry | null;
  /** Its element is watched and seen off screen (not merely not watched yet, nor missing): nothing of it is shown, so
   * nothing need be read for it until it comes back. */
  hidden: boolean;
}

/** Whether a widget reads its value at the head from follow, or asks for it as before: the one place the
 * request path is kept for a value follow does not serve (deleted with P11, when format 12 is the only one). */
export const followsHead = (followed: Followed) => followed.status === 'followed';

/** How often the fallback's clock moves its widget while playing. */
const FALLBACK_PACE_MS = 250;

/** The fallback's clock, beside `followsHead` and deleted with it: the hour a widget asks by for a value
 * follow does not serve. Each widget paces its own from the clock's frames, at most every
 * `FALLBACK_PACE_MS` while playing and at once when the clock stops, steps or seeks; no tick is shared
 * by the page. Off, the hour is read as rendered and nothing renders for it. */
export function useFallbackHour(clock: PlaybackController, on = true): number {
  const widget = measuredWidget();
  countWork(widget, 'render');
  const [paced, setPaced] = useState<{ clock: PlaybackController; hour: number } | null>(null);
  useEffect(() => {
    if (!on) return;
    let last = Number.NaN, told = -Infinity;
    const tell = () => {
      const { hour, playing } = clock.getSnapshot();
      if (hour === last) return;
      const now = performance.now();
      if (playing && now - told < FALLBACK_PACE_MS) return;
      last = hour; told = now;
      countWork(widget, 'hour');
      startTransition(() => setPaced(before => before?.clock === clock && before.hour === hour ? before : { clock, hour }));
    };
    tell();
    const stop = clock.subscribeFrame(tell);
    return () => { stop(); setPaced(null); };
  }, [clock, on, widget]);
  return on && paced?.clock === clock ? paced.hour : clock.getHour();
}

/** Follows `query` while `target` is on screen, or near the dashboard's view where its widget is active (from mount;
 * never beyond that reach, never after unmount) and paints its moving parts with `paint` in the animation frame from the store. A
 * value never renders the component: only its availability or a change of `structureOf` the
 * answer does (a row came or went), and with `pace` (the playback clock) such a change renders at most once a slot of
 * the page's shared structure pace while it plays, the newest at the slot's start or at once when it stops. Another
 * element taking `target`'s place (a loading state replaced by the content it waited for) keeps the follow unless it
 * is off screen. Inside the prepare lane's hidden copy of a page nothing is followed and nothing is hidden: the widget asks
 * its head as it does unfollowed, through the lane, so the reply its first render asks when shown is held. */
export function useFollow(store: FollowStore | null, asked: FollowQuery | null, target: RefObject<Element | null>, paint: (entry: FollowEntry) => void,
  structureOf: (answer: FollowedAnswer | undefined, entry: FollowEntry) => string = noStructure, onScreen: OnScreen = observeOnScreen, given?: PaceClock): Followed {
  const prepared = useContext(PreparingPage);
  const query = prepared ? null : asked;
  // Every followed structure is paced: by the clock given, else the page's.
  const pace = given ?? pagePace() ?? undefined;
  const widget = measuredWidget();
  const painting = useRef(paint); painting.current = paint;
  const shape = useRef(structureOf); shape.current = structureOf;
  const shown = useRef<{ entry: FollowEntry | null; painter: Painter | null; listeners: Set<() => void>; visible: boolean; hidden: boolean;
    show: ((visible: boolean) => void) | null }>(null);
  shown.current ??= { entry: null, painter: null, listeners: new Set(), visible: false, hidden: false, show: null };
  const box = shown.current;
  const key = query ? canonical(query) : null;
  // With `pace`: what was rendered last, and the slot's wake-up while a newer one waits for it. While playing, anything
  // the widget renders from its follow (status, the entry, its structure, hidden) is taken only in the slot's task.
  const paced = useRef<{ snapshot: string; wake: (() => void) | null } | null>(null);
  const subscribe = useCallback((changed: () => void) => {
    box.listeners.add(changed);
    const stop = pace?.subscribe(changed);
    return () => { box.listeners.delete(changed); stop?.(); };
  }, [box, pace]);
  const read = () => {
    const entry = box.entry;
    const now = `${entry ? `${entry.status}\u0000${entry.reason ?? ''}\u0000${entry.id}\u0000${shape.current(entry.answer, entry)}`
      : 'off\u0000\u0000\u0000'}\u0000${box.hidden ? 'hidden' : ''}`;
    const held = paced.current;
    if (!pace || !held) { paced.current = { snapshot: now, wake: null }; return now; }
    if (held.snapshot === now) return now;
    if (!structureOpen(pace.getSnapshot().playing)) {
      held.wake ??= atNextSlot(() => { held.wake = null; for (const changed of box.listeners) changed(); });
      return held.snapshot;
    }
    held.wake?.();
    paced.current = { snapshot: now, wake: null };
    return now;
  };
  const snapshot = useSyncExternalStore(subscribe, read, () => 'off\u0000\u0000\u0000\u0000');
  useEffect(() => () => { paced.current?.wake?.(); }, []);
  const followed = useMemo<Followed>(() => {
    const [status, reason, , structure, hidden] = snapshot.split('\u0000') as [Followed['status'], string, string, string, string];
    return { status, reason: reason || undefined, structure, entry: box.entry, hidden: hidden === 'hidden' };
  }, [snapshot, box]);
  const structure = followed.structure;
  // The follow, held while the query stands and its element is on screen.
  useEffect(() => {
    if (!store || !query) return;
    let handle: FollowHandle | null = null, painter: Painter | null = null, unwatch: (() => void) | null = null;
    const tell = () => { for (const changed of box.listeners) changed(); };
    const leave = () => {
      painter?.dispose(); unwatch?.(); handle?.release();
      handle = null; painter = null; unwatch = null; box.painter = null;
      if (box.entry) { box.entry = null; tell(); }
    };
    box.show = visible => {
      if (visible && !handle) {
        handle = store.acquire(query);
        if (widget !== null) perfLog?.followWidget(widget, handle.entry.id);
        box.entry = handle.entry;
        painter = box.painter = store.painter(handle.entry, entry => painting.current(entry), true, widget);
        unwatch = store.watch(handle.entry, tell);
        tell();
      } else if (!visible && handle) leave();
    };
    box.show(box.visible);
    return () => { box.show = null; leave(); };
    // `query` is compared by its canonical text, `key`.
  }, [store, key, box, widget]);
  // Whether its element is on screen: a new element is watched in its place without letting the follow go.
  useEffect(() => {
    const element = target.current;
    const seen = (visible: boolean, watched = true) => {
      const hidden = watched && !visible;
      box.visible = visible;
      box.show?.(visible);
      if (box.hidden !== hidden) { box.hidden = hidden; for (const changed of box.listeners) changed(); }
    };
    if (!element || prepared) { seen(false, false); return; }
    return onScreen(element, seen);
  }, [store, key, target, onScreen, box, prepared]);
  // Rows rendered for a new structure are painted in the frame after they are committed.
  useLayoutEffect(() => { box.painter?.repaint(); }, [structure, box]);
  return followed;
}
