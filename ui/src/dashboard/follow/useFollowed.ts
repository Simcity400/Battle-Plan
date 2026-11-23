import { decode } from '@msgpack/msgpack';
import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { FollowedWidgetKind } from './store.ts';
import type { DashboardClient, DashboardSource } from '../client';
import type { PlaybackController } from '../playback';
import { baseClient } from '../prepare.ts';
import { pageClock } from '../widgetClock.ts';
import { playbackRate } from '../pulled.ts';
import { canonical } from './canonical.ts';
import { pipelineFor, pipelineGeneration, subscribeClockPipeline, type FollowQuery, type MountOptions, type Pipeline, type PipelineHost } from './pipeline.ts';
import type { Channel } from './facts';
import type { FollowedPanel } from './message';
import type { WidgetView } from './page';
import type { ModelField, LiveMap } from '../../draw/views/fields';
import type { Live, LiveValues } from '../../charts/live';
import { observeOnScreen, type OnScreen } from './onScreen.ts';

export const laneCount = () => Math.max(1, Math.min(4, (globalThis.navigator?.hardwareConcurrency || 2) - 2));

const weakCpu = () => Number((globalThis as { __weakCpu?: number }).__weakCpu) || 1;

export function lanesHost(client: DashboardClient): PipelineHost {
  const lanes = Array.from({ length: laneCount() }, () => new Worker(new URL('../../draw/lane.worker.ts', import.meta.url), { type: 'module' }));
  return {
    lanes,
    open: ports => client.openFollow ? client.openFollow(ports) : Promise.reject(new Error('This connection has no channel for pushed answers')),
    channel: () => new MessageChannel(),
    frameRate: playbackRate,
    rate: weakCpu,
    close: () => { for (const lane of lanes) lane.terminate(); },
  };
}

export function usePipeline(clock: PlaybackController | null, client: DashboardClient | null): Pipeline | null {
  return useMemo(() => {
    if (!clock || !client) return null;
    const base = baseClient(client);
    return pipelineFor(pageClock(clock), base, () => lanesHost(base));
  }, [clock, client]);
}

export function usePipelineGeneration(clock: PlaybackController | null): number {
  const page = clock ? pageClock(clock) : null;
  const read = useCallback(() => page ? pipelineGeneration(page) : 0, [page]);
  const subscribe = useCallback((changed: () => void) => page ? subscribeClockPipeline(page, changed) : () => {}, [page]);
  return useSyncExternalStore(subscribe, read, read);
}

export function useFollowedWidget(pipeline: Pipeline | null, key: string | null, options: MountOptions): number | null {
  const [held, setHeld] = useState<{ key: string; id: number } | null>(null);
  const contextKind = useContext(FollowedWidgetKind);
  const timing = useRef(options);
  timing.current = options.kind === undefined && contextKind !== null ? { ...options, kind: contextKind } : options;
  useLayoutEffect(() => {
    if (key === null || !pipeline) { setHeld(null); return; }
    setHeld({ key, id: pipeline.acquire(key, timing.current) });
    return () => { pipeline.drop(key); };
  }, [pipeline, key]);
  const id = heldFor(held, key);
  const { offset, pin, bounds } = options;
  const boundsKey = bounds === undefined ? 'kept' : bounds ? `${bounds[0]}:${bounds[1]}` : '';
  useLayoutEffect(() => {
    if (id !== null) pipeline?.retime(id, { offset, pin, ...bounds !== undefined ? { bounds } : {} });
  }, [pipeline, id, offset, pin, boundsKey]);
  return id;
}

export function useSurfaceLabel(element: Element | null, on: boolean, placement: number | null, label = 'page') {
  useLayoutEffect(() => {
    if (!on || placement === null || !(element instanceof HTMLElement) || element.dataset.surface) return;
    element.dataset.surface = label; element.dataset.placement = String(placement);
    return () => { delete element.dataset.surface; delete element.dataset.placement; };
  }, [element, on, placement, label]);
}

export function useElement(target: { readonly current: Element | null }): Element | null {
  const [element, setElement] = useState<Element | null>(null);
  useLayoutEffect(() => { if (target.current !== element) setElement(target.current); });
  return element;
}

export function useOnScreen(target: Element | null, onScreen: OnScreen = observeOnScreen): boolean {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!target?.isConnected) { setVisible(false); return; }
    return onScreen(target, setVisible);
  }, [target, onScreen]);
  return visible;
}

export function useFollowedSub(pipeline: Pipeline | null, widget: number | null, follow: { source: DashboardSource; branch: string; offset: number } | null,
  query: FollowQuery | null, channel: Channel, product?: string): number | undefined {
  const [sub, setSub] = useState<number | undefined>(undefined);
  const key = query && follow ? canonical(query) : null;
  const held = useRef({ query, follow });
  held.current = { query, follow };
  useLayoutEffect(() => {
    const { query, follow } = held.current;
    if (!pipeline || widget === null || key === null || !query || !follow) { setSub(undefined); return; }
    const token = {};
    setSub(pipeline.subscribe(widget, token, follow.source, follow.branch, follow.offset, query, channel, product));
    return () => pipeline.unsubscribe(widget, token);
  }, [pipeline, widget, key, channel, product, follow?.source.id, follow?.source.revision, follow?.branch, follow?.offset]);
  return sub;
}

const viewKey = (view: WidgetView | null) => view
  ? `${view.mode}|${view.reason ?? ''}|${view.structure ?? ''}|${view.dim}|${String(view.tooltip ?? '')}`
  : 'none';

export type Reading =
  | { state: 'pending' }
  | { state: 'outside' }
  | { state: 'refused'; reason: string }
  | { state: 'covered'; hour: number; dim: boolean };

export function readingOf(view: WidgetView | null | undefined): Reading {
  if (!view || view.mode === 'reading') return { state: 'pending' };
  if (view.mode === 'outside') return { state: 'outside' };
  if (view.mode === 'failed' || view.mode === 'refused') return { state: 'refused', reason: view.reason ?? 'The followed reading stopped without a reason' };
  return { state: 'covered', hour: view.painted ?? view.h!, dim: view.dim };
}

export function datedReading(reading: Reading, dated: { read(): number | null }): Reading {
  return reading.state !== 'covered' ? reading : { state: 'covered', dim: reading.dim, get hour() { return dated.read() ?? reading.hour; } };
}

export function componentReading(view: WidgetView | null | undefined, sub: number | undefined): Reading {
  const refused = sub === undefined || view?.mode !== 'picture' ? undefined : view.fields?.[`refused:${sub}`];
  return typeof refused === 'string' ? { state: 'refused', reason: refused } : readingOf(view);
}

export function joinReadings(readings: readonly (Reading | null)[]): Reading {
  const held = readings.filter((reading): reading is Reading => reading !== null);
  return held.find(reading => reading.state === 'refused') ?? held.find(reading => reading.state === 'outside') ?? held.find(reading => reading.state === 'pending')
    ?? held.reduce<Reading>((joined, reading) => joined.state !== 'covered' || reading.state !== 'covered' ? reading
      : { state: 'covered', hour: Math.max(joined.hour, reading.hour), dim: joined.dim || reading.dim },
    held[0] ?? { state: 'pending' });
}

export const branchBounds = (source: DashboardSource, branch: unknown): readonly [number, number] | undefined => {
  const held = source.metadata.branches?.find(each => String(each.id) === String(branch));
  return held ? [held.firstHour, held.lastHour] : undefined;
};

export function useInstalled(pipeline: Pipeline | null, widget: number | null): WidgetView | null {
  const subscribe = useMemo(() => (changed: () => void) => widget === null || !pipeline ? () => {} : pipeline.onInstall(widget, changed), [pipeline, widget]);
  const held = useRef<{ key: string; view: WidgetView | null }>({ key: 'none', view: null });
  const read = () => {
    const view = widget === null || !pipeline ? null : pipeline.view(widget);
    const key = viewKey(view);
    if (key !== held.current.key) held.current = { key, view };
    return held.current.view;
  };
  return useSyncExternalStore(subscribe, read, read);
}

export function datedStore(pipeline: Pipeline | null, widget: number | null) {
  return {
    subscribe: (changed: () => void) => {
      if (widget === null || !pipeline) return () => {};
      const installs = pipeline.onInstall(widget, changed), dates = pipeline.onDate(widget, changed);
      return () => { installs(); dates(); };
    },
    read: (): number | null => {
      const view = widget === null || !pipeline ? null : pipeline.view(widget);
      return view?.mode === 'picture' ? view.painted ?? view.h ?? null : null;
    },
  };
}

export function onInstalled(pipeline: Pipeline, widget: number, paint: () => void): () => void {
  const run = () => { if (pipeline.view(widget)?.mode === 'picture') paint(); };
  run();
  return pipeline.onInstall(widget, run);
}

export const heldFor = (held: { key: string; id: number } | null, key: string | null) => held !== null && held.key === key ? held.id : null;

export function useInstalledValue<T extends string | number | boolean | null>(subscribe: ((changed: () => void) => () => void) | undefined, select: () => T): T {
  const selected = useRef(select);
  selected.current = select;
  const listen = useMemo(() => (changed: () => void) => subscribe?.(changed) ?? (() => {}), [subscribe]);
  const read = useMemo(() => () => selected.current(), []);
  return useSyncExternalStore(listen, read, read);
}

const panels = new WeakMap<object, FollowedPanel>();

export function installedPanel(model: ModelField | undefined): FollowedPanel | undefined {
  if (!model) return undefined;
  let panel = panels.get(model);
  if (!panel) {
    if (model.refused !== null) panel = { refused: model.refused };
    else if (model.bytes) {
      const bytes = model.bytes;
      let data: unknown, read = false;
      panel = Object.defineProperty({ refused: null, bytes } as FollowedPanel, 'data', { enumerable: true, get: () => {
        if (!read) { data = decode(bytes); read = true; }
        return data;
      } });
    } else panel = { data: model.data, refused: null };
    panels.set(model, panel);
  }
  return panel;
}

const lives = new WeakMap<object, LiveValues<number>>();
const NO_LIVE: LiveValues<number> = { get: () => undefined, missing: () => 'Not read yet' };

export function liveOf(map: LiveMap | null | undefined): LiveValues<number> {
  if (!map) return NO_LIVE;
  let values = lives.get(map);
  if (!values) {
    values = {
      get: key => { const value = map.get(key); return typeof value === 'number' ? value : typeof value === 'bigint' ? Number(value) : undefined; },
      raw: key => { const value = map.get(key); return typeof value === 'bigint' ? value : typeof value === 'number' && Number.isSafeInteger(value) ? BigInt(value) : undefined; },
      missing: key => { const value = map.get(key); return typeof value === 'string' ? value : 'Not recorded'; },
    };
    lives.set(map, values);
  }
  return values;
}

export function installedLive(pipeline: Pipeline, widget: number, sub: number): Live<LiveValues<number>> {
  return {
    read: () => liveOf(pipeline.view(widget)?.fields?.[`live:${sub}`] as LiveMap | null | undefined),
    subscribe: changed => pipeline.onInstall(widget, changed),
  };
}

export const installedModel = (view: WidgetView | null, sub: number | undefined) =>
  sub === undefined ? undefined : view?.fields?.[`model:${sub}`] as ModelField | undefined;

export const installedAnswer = (view: WidgetView | null, sub: number | undefined) =>
  sub === undefined ? undefined : view?.fields?.[`answer:${sub}`] as Uint8Array | undefined;
