import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import type { RefObject } from 'react';
import { perfLog } from '../../shell/perf.ts';
import type { PlaybackController } from '../playback';
import { countWork, measuredWidget } from '../widgetPerf.ts';
import { PreparingPage } from '../preparing.ts';
import type { FollowedAnswer } from './message.ts';
import { canonical } from './canonical.ts';
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
  element = element.closest('article.dash-widget') ?? element;
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

/** Whether the service accepted the widget's follow. */
export const followsHead = (followed: Followed) => followed.status === 'followed';

/** The request hour at an anchor, including paused seeks. */
export function useFallbackHour(clock: PlaybackController, on = true): number {
  const widget = measuredWidget();
  countWork(widget, 'render');
  const subscribe = useCallback((changed: () => void) => on ? clock.subscribe(changed) : () => {}, [clock, on]);
  const read = useCallback(() => on ? clock.getHour() : null, [on, clock]);
  return useSyncExternalStore(subscribe, read, read) ?? clock.getHour();
}

/** A visible widget shares its query and renders when availability or row structure changes. */
export function useFollow(store: FollowStore | null, asked: FollowQuery | null, target: RefObject<Element | null>, paint: ((entry: FollowEntry) => unknown) | null,
  structureOf: (answer: FollowedAnswer | undefined, entry: FollowEntry) => string = noStructure, onScreen: OnScreen = observeOnScreen): Followed {
  const prepared = useContext(PreparingPage);
  const query = prepared ? null : asked;
  const widget = measuredWidget();
  const painting = useRef(paint); painting.current = paint;
  const shape = useRef(structureOf); shape.current = structureOf;
  const shown = useRef<{ entry: FollowEntry | null; store?: FollowStore; key?: string | null; painter: Painter | null; listeners: Set<() => void>; visible: boolean; hidden: boolean;
    show: ((visible: boolean) => void) | null }>(null);
  shown.current ??= { entry: null, painter: null, listeners: new Set(), visible: false, hidden: false, show: null };
  const box = shown.current;
  const key = query ? canonical(query) : null;
  const entry = box.store === store && box.key === key ? box.entry : null;
  const paints = paint !== null;
  const subscribe = useCallback((changed: () => void) => {
    box.listeners.add(changed);
    return () => { box.listeners.delete(changed); };
  }, [box]);
  const read = () => {
    const reading = box.store === store && box.key === key ? box.entry : null;
    const now = `${reading ? `${reading.status}\u0000${reading.reason ?? ''}\u0000${reading.id}\u0000${shape.current(reading.answer, reading)}`
      : 'off\u0000\u0000\u0000'}\u0000${box.hidden ? 'hidden' : ''}`;
    return now;
  };
  const snapshot = useSyncExternalStore(subscribe, read, () => 'off\u0000\u0000\u0000\u0000');
  const followed = useMemo<Followed>(() => {
    const [status, reason, , structure, hidden] = snapshot.split('\u0000') as [Followed['status'], string, string, string, string];
    return { status, reason: reason || undefined, structure, entry, hidden: hidden === 'hidden' };
  }, [snapshot, entry]);
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
        box.store = store;
        box.key = key;
        if (paints) painter = box.painter = store.painter(handle.entry, entry => painting.current?.(entry), true, widget);
        unwatch = store.watch(handle.entry, tell);
        tell();
      } else if (!visible && handle) leave();
    };
    box.show(box.visible);
    return () => { box.show = null; leave(); };
    // `query` is compared by its canonical text, `key`.
  }, [store, key, box, widget, paints]);
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
