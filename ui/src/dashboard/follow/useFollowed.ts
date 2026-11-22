import { decode } from '@msgpack/msgpack';
import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { DashboardClient, DashboardSource } from '../client';
import type { PlaybackController } from '../playback';
import { baseClient } from '../prepare.ts';
import { pageClock } from '../widgetClock.ts';
import { playbackRate } from '../pulled.ts';
import { canonical } from './canonical.ts';
import { pipelineFor, type FollowQuery, type MountOptions, type Pipeline, type PipelineHost } from './pipeline.ts';
import type { Channel } from './facts';
import type { FollowedPanel } from './message';
import type { WidgetView } from './page';
import type { ModelField, LiveMap } from '../../draw/views/fields';
import type { Live, LiveValues } from '../../charts/live';
import { observeOnScreen, type OnScreen } from './onScreen.ts';

export const laneCount = () => {
  const asked = Number((globalThis as { __drawLanes?: number }).__drawLanes);
  if (asked >= 1) return Math.min(4, Math.floor(asked));
  return Math.max(1, Math.min(4, (globalThis.navigator?.hardwareConcurrency || 2) - 2));
};

const weakCpu = () => Number((globalThis as { __weakCpu?: number }).__weakCpu) || 1;

export function lanesHost(client: DashboardClient): PipelineHost {
  const lanes = Array.from({ length: laneCount() }, () => new Worker(new URL('../../draw/lane.worker.ts', import.meta.url), { type: 'module' }));
  return {
    lanes,
    open: ports => client.openFollow ? client.openFollow(ports) : Promise.reject(new Error('This connection has no channel for pushed answers')),
    channel: () => new MessageChannel(),
    frameRate: playbackRate,
    rate: weakCpu,
  };
}

export function usePipeline(clock: PlaybackController | null, client: DashboardClient | null): Pipeline | null {
  return useMemo(() => {
    if (!clock || !client) return null;
    const base = baseClient(client);
    return pipelineFor(pageClock(clock), base, () => lanesHost(base));
  }, [clock, client]);
}

export function useFollowedWidget(pipeline: Pipeline | null, key: string | null, options: MountOptions): number | null {
  const [id, setId] = useState<number | null>(null);
  const timing = useRef(options);
  timing.current = options;
  useLayoutEffect(() => {
    if (key === null || !pipeline) { setId(null); return; }
    const made = pipeline.acquire(key, timing.current);
    setId(made);
    return () => { pipeline.drop(key); };
  }, [pipeline, key]);
  const { offset, pin, bounds } = options;
  const boundsKey = bounds === undefined ? 'kept' : bounds ? `${bounds[0]}:${bounds[1]}` : '';
  useLayoutEffect(() => {
    if (id !== null) pipeline?.retime(id, { offset, pin, ...bounds !== undefined ? { bounds } : {} });
  }, [pipeline, id, offset, pin, boundsKey]);
  return id;
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
  ? `${view.mode}|${view.reason ?? ''}|${view.id ?? ''}|${view.h ?? ''}|${view.dim}|${String(view.tooltip ?? '')}`
  : 'none';

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
      get: key => { const value = map.get(key); return typeof value === 'number' ? value : undefined; },
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
