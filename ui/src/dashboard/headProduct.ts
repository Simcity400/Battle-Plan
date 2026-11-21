import { useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { resolveWindow } from './model.ts';
import { useFollowedProduct } from './products.ts';
import type { ProductState } from './products';
import type { Failure } from './queryError';
import { followStore } from './follow/connection.ts';
import { panelReply } from './follow/panel.ts';
import { observeOnScreen, useFallbackHour, useFollow } from './follow/useFollow.ts';
import type { FollowedAnswer } from './follow/message';
import type { FollowEntry } from './follow/store';
import { canonical } from './follow/canonical.ts';
import { repliedCells } from './follow/liveCells.ts';
import type { Live, LiveValues } from '../charts/live';
import { PreparingPage } from './preparing.ts';
import { useRecordedRows } from './widgetPerf.ts';
import { recordedPanelRows } from '../draw/panels/views.ts';

export function headRead(widget: Pick<WidgetSpec, 'time' | 'kind' | 'trackIds'>, track: BoundTrack | undefined, tracks: BoundTrack[],
  hoverHour: number | null): { follows: boolean; hourAt: (clockHour: number) => number | null } {
  return {
    follows: widget.time.mode === 'follow' && hoverHour === null,
    hourAt: clockHour => {
      if (!track) return null;
      const hour = hoverHour ?? resolveWindow(widget as WidgetSpec, track, tracks, clockHour)?.lastHour;
      return hour === undefined || hour < track.branch.firstHour || hour > track.branch.lastHour ? null : Math.floor(hour);
    },
  };
}

export function headSource(following: boolean, status: 'off' | FollowEntry['status'], landed: boolean, hidden = false): 'pushed' | 'fallback' | 'asked' | 'idle' {
  if (following && hidden) return 'idle';
  if (!following || status === 'off') return 'asked';
  return status === 'followed' && landed ? 'pushed' : 'fallback';
}

export interface HeadProductOptions<T> {
  paint?: (reply: T) => boolean | void;
  active: boolean;
  validate: (data: unknown) => T;
  structure?: (reply: T, hour: number) => string;
  hoverHour?: number | null;
}

export interface HeadProduct<T> extends ProductState<T> {
  exactRead?: (hour: number) => Promise<unknown>;
  workerFollow?: number;
  root: (element: Element | null) => void;
  read: 'followed' | 'asked' | 'fallback' | 'off';
  reason?: string;
  hour: number | null;
  at: number | null;
  atNow: () => number | null;
  live: Live<LiveValues<number>>;
  shownAt: () => number | null;
  current: () => T | undefined;
}

export const headFallbacks = new Map<string, string>();

const MOVING = ['hour'] as const;

export function whenDue(hour: number, head: () => number, subscribe: (fn: () => void) => () => void, wake: () => void) {
  const frame = () => { if (head() >= hour) { stop(); wake(); } };
  const stop = subscribe(frame);
  frame();
  return stop;
}

export function momentHour(moment: string | undefined): number | null {
  if (!moment) return null;
  const [hour] = JSON.parse(moment) as unknown[];
  return typeof hour === 'number' ? hour : null;
}

export function headSubject(track: Pick<BoundTrack, 'source' | 'branch'> | undefined, product: string, params: Readonly<Record<string, unknown>> | null): string | null {
  return track && params ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, product, canonical(params)]) : null;
}

export interface Retained<T> { client: unknown; subject: string; data: T; hour: number | null }
type TargetHour = { exact: number | null } | { through: number };

export function retain<T>(held: Retained<T> | null, client: unknown, subject: string | null, asked: T | undefined, hour: number | null, target?: TargetHour): Retained<T> | null {
  if (subject === null) return null;
  const eligible = (at: number | null) => !target || (at !== null && ('exact' in target ? at === target.exact : at <= target.through));
  if (asked !== undefined && eligible(hour)) return { client, subject, data: asked, hour };
  return held && held.client === client && held.subject === subject && eligible(held.hour) ? held : null;
}

export function useHeadProduct<T>(runtime: WidgetRuntime, widget: WidgetSpec, product: string, params: Readonly<Record<string, unknown>> | null,
  options: HeadProductOptions<T>): HeadProduct<T> {
  const { active, validate, structure, paint, hoverHour = null } = options;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { follows, hourAt } = headRead(widget, track, runtime.tracks, hoverHour);
  const prepared = useContext(PreparingPage);
  const following = active && follows && !prepared && !!track && !!params;
  const paramsKey = params ? canonical(params) : null;
  const [element, setElement] = useState<Element | null>(null);
  const target = useMemo(() => ({ current: element }), [element]);
  const store = useMemo(() => track ? followStore(runtime.client, track.source, runtime.clock, track.axisOffset) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const query = useMemo(() => following ? { ...params, panel: product, branchId: track!.branch.id } : null,
    [following, product, paramsKey, track?.branch.id]);
  const latest = useRef({ validate, structure, paint });
  latest.current = { validate, structure, paint };
  const shape = useCallback((_answer: FollowedAnswer | undefined, entry: FollowEntry) => {
    const { data, error } = panelReply(entry.panel, latest.current.validate);
    if (error) return `!${error.message}`;
    if (data === undefined) return '';
    return latest.current.structure ? `=${latest.current.structure(data, entry.hour)}` : `#${entry.version}`;
  }, []);
  const painting = useCallback((entry: FollowEntry) => {
    const { data } = panelReply(entry.panel, latest.current.validate);
    if (data !== undefined) return latest.current.paint?.(data);
  }, []);
  const followed = useFollow(store, query, target, paint ? painting : null, shape, observeOnScreen);
  const entry = followed.entry;
  const source = headSource(following, followed.status, !!entry?.panel, followed.hidden);
  const idle = source === 'idle';
  const pushed = source === 'pushed';
  const panelLive = useMemo(() => {
    if (!store || !entry) return null;
    let held: typeof entry.values;
    let values: LiveValues<number>;
    return {
      read() {
        const next = entry.values;
        if (!values || held !== next) {
          held = next;
          values = {
            get(key) { const v = next?.get(key); return typeof v === 'number' ? v : undefined; },
            missing(key) { const v = next?.get(key); return typeof v === 'string' ? v : 'Not recorded'; },
          };
        }
        return values;
      },
      subscribe: (changed: () => void) => store.watchLive(entry, changed),
    };
  }, [store, entry]);
  const fallback = source === 'fallback';
  const requestHour = useFallbackHour(runtime.clock, fallback);
  const hour = pushed || idle || !track || !params ? null : hourAt(fallback ? requestHour : runtime.clock.getHour());
  const asked = useFollowedProduct<T>(runtime.client, hour === null ? null : { sourceId: track!.source.id, revision: track!.source.revision,
    product, params: { ...params, branchId: track!.branch.id, hour } }, !follows ? [] : MOVING, active, validate, track?.source);
  const askedAt = asked.data === undefined ? null : momentHour(asked.moment) ?? hour;
  const held = useRef<Retained<T> | null>(null);
  const targetHour: TargetHour = follows ? { through: runtime.clock.getHour() + (track?.axisOffset ?? 0) } : { exact: hour };
  const dueAt = 'through' in targetHour && askedAt !== null && askedAt > targetHour.through ? askedAt - (track?.axisOffset ?? 0) : null;
  const [, wake] = useState(0);
  useEffect(() => {
    if (dueAt !== null) return whenDue(dueAt, runtime.clock.getHour, runtime.clock.subscribeFrame, () => wake(n => n + 1));
  }, [dueAt, runtime.clock]);
  held.current = retain(held.current, runtime.client, headSubject(track, product, params), asked.data, askedAt, targetHour);
  const fromPanel = pushed ? panelReply(followed.entry?.panel, validate) : {};
  const kept = held.current ?? undefined;
  const data = pushed ? fromPanel.data : kept?.data;
  const dataAt = kept?.hour ?? null;
  const error: Failure | undefined = pushed ? fromPanel.error : asked.error;
  const askedData = pushed ? undefined : data;
  const askedLive = useMemo(() => repliedCells(askedData), [askedData]);
  const live = pushed && panelLive ? panelLive : askedLive;
  if (followed.status === 'unavailable' && followed.reason) headFallbacks.set(product, followed.reason);
  const exactRead = perfLog && track && params ? async (at: number) => (await runtime.client.product({
    sourceId: track.source.id, revision: track.source.revision, product, params: { ...params, branchId: track.branch.id, hour: at },
  }, new AbortController().signal)).data : undefined;
  const committedHour = entry?.hour;
  useRecordedRows(product, exactRead && pushed && data !== undefined && committedHour !== undefined ? {
    hour: committedHour, rows: () => recordedPanelRows(product, data),
    exact: async () => recordedPanelRows(product, await exactRead(committedHour)),
  } : undefined);
  return {
    data, error, retry: asked.retry, root: setElement, hour, live,
    exactRead,
    workerFollow: pushed && runtime.client.openFollow ? entry?.id : undefined,
    at: pushed ? entry?.panel ? entry.hour : null : dataAt,
    atNow: pushed && entry ? () => entry.panel ? entry.hour : null : () => dataAt,
    shownAt: pushed && store && entry ? () => entry.panel ? entry.through : null : () => data === undefined ? null : dataAt,
    current: pushed && entry ? () => panelReply(entry.panel, latest.current.validate).data
      : () => follows && dataAt !== null && dataAt > runtime.clock.getHour() + (track?.axisOffset ?? 0) ? undefined : data,
    read: pushed ? 'followed' : fallback || idle ? 'fallback' : hour === null ? 'off' : 'asked',
    ...(followed.status === 'unavailable' && followed.reason ? { reason: followed.reason } : {}),
  };
}
import { perfLog } from '../shell/perf.ts';
