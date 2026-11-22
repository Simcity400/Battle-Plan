import { createContext, createElement, useCallback, useContext, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';
import type { BoundTrack, WidgetSpec } from './model';
import type { PlaybackController } from './playback';
import { pageClock } from './widgetClock.ts';
import { axisHour, regionFollows, regionHour, regionKey, regionOutside, regionTrack, SPEC, type AxisHour, OUTSIDE_MESSAGE, type Outside, type Region, type RegionAction, type TrackHour } from './regionTime.ts';

export { SETTINGS_DETAIL } from './regionTime.ts';

export class ActiveHeads {
  private readonly heads = new Set<object>();
  private readonly listeners = new Set<() => void>();

  set(head: object, active: boolean): void {
    const was = this.heads.size > 0;
    if (active) this.heads.add(head);
    else this.heads.delete(head);
    if (was === this.heads.size > 0) return;
    for (const listener of [...this.listeners]) listener();
  }

  readonly any = (): boolean => this.heads.size > 0;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
}

export interface ReadCoverage { outside: boolean; outsideMessage?: string; outsideDetail?: string }

interface Boundary { holds: () => unknown; was: unknown; changed: () => void }
interface Boundaries { watches: Set<Boundary>; sequence: number; stop: () => void }
const boundaries = new WeakMap<PlaybackController, Boundaries>();

export function watchBoundary(clock: PlaybackController, holds: () => unknown, changed: () => void): () => void {
  const page = pageClock(clock);
  let shared = boundaries.get(page);
  if (!shared) {
    const made: Boundaries = { watches: new Set(), sequence: page.getSnapshot().sequence, stop: () => {} };
    const check = () => {
      const { sequence } = page.getSnapshot();
      if (sequence === made.sequence) return;
      made.sequence = sequence;
      for (const watch of made.watches) {
        const now = watch.holds();
        if (!Object.is(now, watch.was)) { watch.was = now; watch.changed(); }
      }
    };
    const stopFrame = page.subscribeFrame(check), stopAnchor = page.subscribe(check);
    made.stop = () => { stopFrame(); stopAnchor(); };
    boundaries.set(page, shared = made);
  }
  const watch: Boundary = { holds, was: holds(), changed };
  const watches = shared.watches;
  watches.add(watch);
  return () => {
    watches.delete(watch);
    if (!watches.size && boundaries.get(page) === shared) { shared.stop(); boundaries.delete(page); }
  };
}

type Timed = { clock: PlaybackController; tracks: BoundTrack[] };

export function useRegion(widget: WidgetSpec, action: RegionAction = SPEC): Region {
  const hour = action.kind === 'at' ? action.hour : null;
  return useMemo(() => ({ widget, action }), [widget, action.kind, hour]);
}

export function useRegionHourAt(runtime: Timed, region: Region): (axis: AxisHour) => TrackHour | null {
  const track = regionTrack(region, runtime.tracks);
  return useCallback((axis: AxisHour) => regionHour(region, track, runtime.tracks, axis), [region, track, runtime.tracks]);
}

const outsideKey = (outside: Outside | null) => outside ? `${outside.message}\u0000${outside.detail ?? ''}` : '';
const unwatched = () => () => {};

export function useRegionOutside(runtime: Timed, region: Region, on = true): Outside | null {
  const track = regionTrack(region, runtime.tracks);
  const { clock, tracks } = runtime;
  const now = useCallback(() => outsideKey(regionOutside(region, track, tracks, axisHour(clock))), [region, track, tracks, clock]);
  const watch = useCallback((changed: () => void) => on && track ? watchBoundary(clock, now, changed) : () => {}, [on, track, clock, now]);
  const key = useSyncExternalStore(on && track ? watch : unwatched, now, now);
  return useMemo(() => {
    if (!key) return null;
    const [message, detail] = key.split('\u0000');
    return { message, detail: detail || undefined };
  }, [key]);
}

export function useRegionCoverage(runtime: Timed, region: Region): ReadCoverage {
  const outside = useRegionOutside(runtime, region);
  return useMemo(() => coverageOf(outside), [outside]);
}

export const coverageOf = (outside: Outside | null): ReadCoverage =>
  outside ? { outside: true, outsideMessage: outside.message, outsideDetail: outside.detail } : { outside: false };

export function epochOf(epochs: readonly number[] | undefined, hour: number | null): number {
  if (hour === null) return -1;
  if (!epochs) return 0;
  let low = 0, high = epochs.length;
  while (low < high) { const middle = (low + high) >> 1; if (epochs[middle] <= hour) low = middle + 1; else high = middle; }
  return low;
}

export function useRegionHour(runtime: Timed, region: Region, on = true, epochs?: readonly number[]): TrackHour | null {
  const hourAt = useRegionHourAt(runtime, region);
  const follows = regionFollows(region);
  const clock = runtime.clock;
  const generation = useRef(0);
  const held = useRef<{ generation: number; hourAt: unknown; epochs: unknown; epoch: number; value: TrackHour | null } | null>(null);
  const subscribe = useCallback((changed: () => void) => {
    if (!on || !follows) return () => {};
    const notify = () => { generation.current++; changed(); };
    const stopAnchor = clock.subscribe(notify);
    const stopBoundary = watchBoundary(clock, () => epochOf(epochs, hourAt(axisHour(clock))), notify);
    return () => { stopAnchor(); stopBoundary(); };
  }, [on, follows, clock, hourAt, epochs]);
  const read = () => {
    const now = hourAt(axisHour(clock));
    const epoch = epochOf(epochs, now);
    const kept = held.current;
    if (follows && kept && kept.generation === generation.current && kept.hourAt === hourAt && kept.epochs === epochs && kept.epoch === epoch) return kept.value;
    held.current = { generation: generation.current, hourAt, epochs, epoch, value: now };
    return now;
  };
  return useSyncExternalStore(subscribe, read, read);
}

export interface RegionScope { key: string; heads: ActiveHeads; outside: Outside | null }
export const RegionContext = createContext<RegionScope | null>(null);

export function useRegionScope(runtime: Timed, region: Region): RegionScope {
  const outside = useRegionOutside(runtime, region);
  const [heads] = useState(() => new ActiveHeads());
  const key = regionKey(region);
  return useMemo(() => ({ key, heads, outside }), [key, heads, outside]);
}

export function TimeRegion({ runtime, region, children }: { runtime: Timed; region: Region; children?: ReactNode }) {
  const scope = useRegionScope(runtime, region);
  return createElement(RegionContext.Provider, { value: scope }, children);
}

export function useReadsRegion(region: Region, active: boolean): void {
  const scope = useContext(RegionContext);
  const heads = scope && scope.key === regionKey(region) ? scope.heads : null;
  const [token] = useState(() => ({}));
  useLayoutEffect(() => {
    if (!heads || !active) return;
    heads.set(token, true);
    return () => heads.set(token, false);
  }, [heads, token, active]);
}

const unsubscribed = () => () => {};
const none = () => false;

export function useOutsideRecording(read?: ReadCoverage, shown = false): Outside | null {
  const scope = useContext(RegionContext);
  const reading = useSyncExternalStore(scope?.heads.subscribe ?? unsubscribed, scope?.heads.any ?? none, scope?.heads.any ?? none);
  if (read) return read.outside ? { message: read.outsideMessage ?? OUTSIDE_MESSAGE, detail: read.outsideDetail } : null;
  return scope?.outside && (reading || shown) ? scope.outside : null;
}
