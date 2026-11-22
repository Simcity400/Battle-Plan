import { createContext, createElement, useCallback, useContext, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';
import type { BoundTrack, WidgetSpec } from './model';
import type { PlaybackController } from './playback';
import { pageClock } from './widgetClock.ts';
import { axisHour, regionDetail, regionFollows, regionHour, regionKey, regionTrack, SPEC, type Region, type RegionAction, type TrackHour } from './regionTime.ts';

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

export interface ReadCoverage { outside: boolean; outsideDetail?: string }

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

export function useOutsideTime(clock: PlaybackController, bounded: boolean, hourAt: (clockHour: number) => number | null): boolean {
  const outsideNow = useCallback(() => bounded && hourAt(clock.getHour()) === null, [bounded, hourAt, clock]);
  const watchClock = useCallback((changed: () => void) => bounded ? watchBoundary(clock, outsideNow, changed) : () => {}, [bounded, clock, outsideNow]);
  return useSyncExternalStore(watchClock, outsideNow, outsideNow);
}

type Timed = { clock: PlaybackController; tracks: BoundTrack[] };

export function useRegion(widget: WidgetSpec, action: RegionAction = SPEC): Region {
  const hour = action.kind === 'at' ? action.hour : null;
  return useMemo(() => ({ widget, action }), [widget, action.kind, hour]);
}

export function useRegionHourAt(runtime: Timed, region: Region): (clockHour: number) => TrackHour | null {
  const track = regionTrack(region, runtime.tracks);
  return useCallback((clockHour: number) => regionHour(region, track, runtime.tracks, clockHour as never), [region, track, runtime.tracks]);
}

export function useRegionCoverage(runtime: Timed, region: Region): ReadCoverage {
  const hourAt = useRegionHourAt(runtime, region);
  const outside = useOutsideTime(runtime.clock, !!regionTrack(region, runtime.tracks), hourAt);
  const detail = outside ? regionDetail(region, null) : undefined;
  return useMemo(() => ({ outside, outsideDetail: detail }), [outside, detail]);
}

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
    const stopBoundary = watchBoundary(clock, () => epochOf(epochs, hourAt(clock.getHour())), notify);
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

export interface RegionScope { key: string; heads: ActiveHeads; outside: boolean; detail: string | undefined }
export const RegionContext = createContext<RegionScope | null>(null);

export function useRegionScope(runtime: Timed, region: Region): RegionScope {
  const { outside, outsideDetail } = useRegionCoverage(runtime, region);
  const [heads] = useState(() => new ActiveHeads());
  const key = regionKey(region);
  return useMemo(() => ({ key, heads, outside, detail: outsideDetail }), [key, heads, outside, outsideDetail]);
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

export function useOutsideRecording(read?: ReadCoverage): { outside: boolean; detail: string | undefined } {
  const scope = useContext(RegionContext);
  const reading = useSyncExternalStore(scope?.heads.subscribe ?? unsubscribed, scope?.heads.any ?? none, scope?.heads.any ?? none);
  if (read) return { outside: read.outside, detail: read.outsideDetail };
  return { outside: !!scope?.outside && reading, detail: scope?.detail };
}
