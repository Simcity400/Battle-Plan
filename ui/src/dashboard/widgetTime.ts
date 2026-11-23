import { createContext, createElement, useCallback, useContext, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { ReactNode } from 'react';
import type { BoundTrack, WidgetSpec } from './model';
import type { PlaybackController } from './playback';
import { pageClock } from './widgetClock.ts';
import { axisHour, coverageAt, coverageSpan, regionFollows, regionHour, regionKey, regionOutside, regionTrack, SPEC, type AxisHour, OUTSIDE_MESSAGE, type Outside, type Region, type RegionAction, type TrackHour } from './regionTime.ts';

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

export type AxisSpan = readonly [low: number, high: number];

interface Watch { span: (axis: number) => AxisSpan; changed: () => void; stamp: number; live: boolean }
interface Edge { watch: Watch; stamp: number; at: number }

class EdgeHeap {
  readonly items: Edge[] = [];
  private readonly before: (a: Edge, b: Edge) => boolean;
  constructor(before: (a: Edge, b: Edge) => boolean) { this.before = before; }
  push(edge: Edge): void {
    const items = this.items;
    let at = items.push(edge) - 1;
    while (at > 0) { const up = (at - 1) >> 1; if (!this.before(items[at], items[up])) break; [items[at], items[up]] = [items[up], items[at]]; at = up; }
  }
  pop(): void {
    const items = this.items, last = items.pop();
    if (!items.length || !last) return;
    items[0] = last;
    for (let at = 0;;) {
      const left = at * 2 + 1, right = left + 1;
      let best = at;
      if (left < items.length && this.before(items[left], items[best])) best = left;
      if (right < items.length && this.before(items[right], items[best])) best = right;
      if (best === at) return;
      [items[at], items[best]] = [items[best], items[at]]; at = best;
    }
  }
}

interface Boundaries { highs: EdgeHeap; lows: EdgeHeap; live: number; sequence: number; stop: () => void }
const boundaries = new WeakMap<PlaybackController, Boundaries>();
const stale = (edge: Edge) => !edge.watch.live || edge.stamp !== edge.watch.stamp;

function place(shared: Boundaries, watch: Watch, axis: number): void {
  const [low, high] = watch.span(axis);
  watch.stamp++;
  if (high < Infinity) shared.highs.push({ watch, stamp: watch.stamp, at: high });
  if (low > -Infinity) shared.lows.push({ watch, stamp: watch.stamp, at: low });
}

function compact(shared: Boundaries): void {
  for (const heap of [shared.highs, shared.lows]) {
    if (heap.items.length <= 4 * shared.live + 16) continue;
    const kept = heap.items.filter(edge => !stale(edge));
    heap.items.length = 0;
    for (const edge of kept) heap.push(edge);
  }
}

export function watchBoundary(clock: PlaybackController, span: (axis: number) => AxisSpan, changed: () => void): () => void {
  const page = pageClock(clock);
  let shared = boundaries.get(page);
  if (!shared) {
    const made: Boundaries = { highs: new EdgeHeap((a, b) => a.at < b.at), lows: new EdgeHeap((a, b) => a.at > b.at), live: 0,
      sequence: page.getSnapshot().sequence, stop: () => {} };
    const check = () => {
      const { sequence } = page.getSnapshot();
      if (sequence === made.sequence) return;
      made.sequence = sequence;
      const axis = page.getHour(), crossed = new Set<Watch>();
      for (const [heap, past] of [[made.highs, (at: number) => at <= axis], [made.lows, (at: number) => at > axis]] as const) {
        for (let top = heap.items[0]; top && (stale(top) || past(top.at)); top = heap.items[0]) {
          heap.pop();
          if (!stale(top)) { top.watch.stamp++; crossed.add(top.watch); }
        }
      }
      for (const watch of crossed) place(made, watch, axis);
      compact(made);
      for (const watch of crossed) if (watch.live) watch.changed();
    };
    const stopFrame = page.subscribeFrame(check), stopAnchor = page.subscribe(check);
    made.stop = () => { stopFrame(); stopAnchor(); };
    boundaries.set(page, shared = made);
  }
  const held = shared;
  const watch: Watch = { span, changed, stamp: 0, live: true };
  held.live++;
  place(held, watch, page.getHour());
  return () => {
    if (!watch.live) return;
    watch.live = false;
    if (--held.live === 0 && boundaries.get(page) === held) { held.stop(); boundaries.delete(page); }
  };
}

export function followSpan(track: Pick<BoundTrack, 'axisOffset' | 'branch'>, epochs?: readonly number[]): (axis: number) => AxisSpan {
  const offset = track.axisOffset;
  return axis => {
    const hour = Math.floor(axis) + offset;
    let [low, high] = coverageSpan(track, hour);
    if (epochs?.length && coverageAt(track, hour).hour !== null) {
      const epoch = epochOf(epochs, hour);
      if (epoch > 0) low = Math.max(low, epochs[epoch - 1]);
      if (epoch < epochs.length) high = Math.min(high, epochs[epoch] - 1);
    }
    return [low - offset, high + 1 - offset];
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
  const moves = on && !!track && regionFollows(region);
  const watch = useCallback((changed: () => void) => moves && track ? watchBoundary(clock, followSpan(track), changed) : () => {}, [moves, track, clock]);
  const key = useSyncExternalStore(moves ? watch : unwatched, now, now);
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
  const track = regionTrack(region, runtime.tracks);
  const follows = regionFollows(region);
  const clock = runtime.clock;
  const generation = useRef(0);
  const held = useRef<{ generation: number; hourAt: unknown; epochs: unknown; epoch: number; value: TrackHour | null } | null>(null);
  const subscribe = useCallback((changed: () => void) => {
    if (!on || !follows) return () => {};
    const notify = () => { generation.current++; changed(); };
    const stopAnchor = clock.subscribe(notify);
    const stopBoundary = track ? watchBoundary(clock, followSpan(track, epochs), notify) : () => {};
    return () => { stopAnchor(); stopBoundary(); };
  }, [on, follows, clock, track, epochs]);
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
