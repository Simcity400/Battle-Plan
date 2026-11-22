import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { resolveWindow } from './model.ts';
import { useFollowedProduct } from './products.ts';
import type { ProductState } from './products';
import type { Failure } from './queryError';
import { panelReply } from './follow/panel.ts';
import { canonical } from './follow/canonical.ts';
import { screenHolder } from './follow/onScreen.ts';
import { useWidgetKey } from './follow/useFollow.ts';
import { installedLive, installedModel, installedPanel, useFollowedSub, useFollowedWidget, useInstalled, useOnScreen, usePipeline } from './follow/useFollowed.ts';
import { repliedCells } from './follow/liveCells.ts';
import type { Live, LiveValues } from '../charts/live';
import { PreparingPage } from './preparing.ts';
import { useRecordedRows } from './widgetPerf.ts';
import { recordedPanelRows } from '../draw/panels/views.ts';
import { inFrame } from './frameUpdates.ts';
import type { PlaybackController } from './playback';
import { pageClock } from './widgetClock.ts';
import { asFailure } from './queryError.ts';
import type { Pipeline } from './follow/pipeline';

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

interface Boundary { holds: () => boolean; was: boolean; changed: () => void }
interface Boundaries { watches: Set<Boundary>; sequence: number; stop: () => void }
const boundaries = new WeakMap<PlaybackController, Boundaries>();

export function watchBoundary(clock: PlaybackController, holds: () => boolean, changed: () => void): () => void {
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
        if (now !== watch.was) { watch.was = now; watch.changed(); }
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

export interface HeldReading<T> { data: T; hour: number; through: number; live: Live<LiveValues<number>> }

export function withHeld<T>(head: HeadProduct<T>): HeadProduct<T> {
  const held = head.held;
  return head.data !== undefined || !held ? head
    : { ...head, data: held.data, live: held.live, at: held.hour, atNow: () => held.hour, shownAt: () => held.through };
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
  structure?: string;
  outside: boolean;
  held?: HeldReading<T>;
  workerFollow?: number;
  followedWidget?: number;
  pipeline?: Pipeline | null;
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

export function whenDue(hour: number, head: () => number, subscribe: (fn: () => void) => () => void, wake: () => void) {
  const frame = () => { if (head() >= hour) { stop(); wake(); } };
  const stop = subscribe(frame);
  frame();
  return stop;
}

export function useRenderAt(hour: number | null, clock: Pick<PlaybackController, 'getHour' | 'subscribeFrame'>) {
  const [, wake] = useState(0);
  useEffect(() => {
    if (hour !== null) return whenDue(hour, clock.getHour, clock.subscribeFrame, () => inFrame(() => wake(n => n + 1)));
  }, [hour, clock]);
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
export const shownAsked = (hour: number | null, paused: number | null) => paused === null || hour === paused;
type TargetHour = { exact: number | null } | { through: number };

export function retain<T>(held: Retained<T> | null, client: unknown, subject: string | null, asked: T | undefined, hour: number | null, target?: TargetHour): Retained<T> | null {
  if (subject === null) return null;
  const eligible = (at: number | null) => !target || (at !== null && ('exact' in target ? at === target.exact : at <= target.through));
  if (asked !== undefined && eligible(hour)) return { client, subject, data: asked, hour };
  return held && held.client === client && held.subject === subject && eligible(held.hour) ? held : null;
}

export function useHeadProduct<T>(runtime: WidgetRuntime, widget: WidgetSpec, product: string, params: Readonly<Record<string, unknown>> | null,
  options: HeadProductOptions<T>): HeadProduct<T> {
  const { active, validate, paint, hoverHour = null } = options;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { follows, hourAt } = useMemo(() => headRead(widget, track, runtime.tracks, hoverHour), [widget, track, runtime.tracks, hoverHour]);
  const prepared = useContext(PreparingPage);
  const following = active && follows && !prepared && !!track && !!params;
  const paramsKey = params ? JSON.stringify(params) : null;
  const [element, setElement] = useState<Element | null>(null);
  const root = useCallback((next: Element | null) => { if (next) setElement(screenHolder(next)); }, []);
  const visible = useOnScreen(following ? element : null);
  const pipeline = usePipeline(runtime.clock, runtime.client);
  const bounds = useMemo(() => track ? [track.branch.firstHour, track.branch.lastHour] as const : null,
    [track?.branch.firstHour, track?.branch.lastHour]);
  const widgetKey = useWidgetKey();
  const pipe = useFollowedWidget(pipeline, following && track ? `${widgetKey}|${track.axisOffset}` : null,
    { offset: track?.axisOffset ?? 0, pin: null, bounds, key: `${widget.kind}|${product}|${paramsKey}` });
  const query = useMemo(() => following && visible ? { ...params, panel: product, branchId: track!.branch.id } : null,
    [following, visible, product, paramsKey, track?.branch.id]);
  const follow = useMemo(() => track ? { source: track.source, branch: track.branch.id, offset: track.axisOffset } : null, [track?.source, track?.branch.id, track?.axisOffset]);
  const sub = useFollowedSub(pipeline, pipe, follow, query, 'panel', product);
  const view = useInstalled(pipeline, pipe);
  const bounded = !!track && !!params;
  const outsideNow = useCallback(() => bounded && hourAt(runtime.clock.getHour()) === null, [bounded, hourAt, runtime.clock]);
  const watchClock = useCallback((changed: () => void) => bounded ? watchBoundary(runtime.clock, outsideNow, changed) : () => {}, [bounded, runtime.clock, outsideNow]);
  const outside = useSyncExternalStore(watchClock, outsideNow, outsideNow) || view?.mode === 'outside';
  const model = installedModel(view, sub);
  const panel = installedPanel(model);
  const pushed = following && view?.mode === 'picture' && panel !== undefined;
  const fromPanel = pushed ? panelReply(panel, validate) : {};
  const named = following && (view?.mode === 'failed' || view?.mode === 'refused') ? view.reason ?? 'The followed stream failed' : undefined;
  const followFailure = useMemo(() => named === undefined ? undefined : asFailure(new Error(named)), [named]);
  const live = useMemo(() => pipeline && pipe !== null && sub !== undefined ? installedLive(pipeline, pipe, sub) : null, [pipeline, pipe, sub]);
  const playing = useSyncExternalStore(runtime.clock.subscribe, () => runtime.clock.getSnapshot().playing, () => runtime.clock.getSnapshot().playing);
  const hour = following || !track || !params ? null : hourAt(runtime.clock.getHour());
  const pausedAt = follows && !playing ? hourAt(runtime.clock.getHour()) : null;
  const askedShows = useCallback((moment: string | undefined) => shownAsked(momentHour(moment) ?? hour, pausedAt), [hour, pausedAt]);
  const asked = useFollowedProduct<T>(runtime.client, hour === null ? null : { sourceId: track!.source.id, revision: track!.source.revision,
    product, params: { ...params, branchId: track!.branch.id, hour } }, [], active, validate, track?.source, undefined, askedShows);
  const askedAt = asked.data === undefined ? null : momentHour(asked.moment) ?? hour;
  const held = useRef<Retained<T> | null>(null);
  const subject = headSubject(track, product, params);
  held.current = following ? null : retain(held.current, runtime.client, subject, asked.data, askedAt, { exact: hour });
  const shown = held.current ?? undefined;
  const data = outside ? undefined : following ? fromPanel.data : shown?.data;
  const error: Failure | undefined = outside ? undefined : following ? followFailure ?? fromPanel.error : asked.error;
  const askedLive = useMemo(() => repliedCells(following ? undefined : data), [following, data]);
  const shownLive = following && live ? live : askedLive;
  const at = outside ? null : following ? (pushed ? view!.h ?? null : null) : shown?.hour ?? null;
  const heldReading = useMemo(() => following && view?.dim && data !== undefined && live ? { data, hour: view.h!, through: view.through!, live } : undefined,
    [following, view, data, live]);
  const latest = useRef(paint);
  latest.current = paint;
  useLayoutEffect(() => { if (data !== undefined) latest.current?.(data); }, [data]);
  const exactRead = track && params ? (at: number) => runtime.client.product({ sourceId: track.source.id, revision: track.source.revision, product,
    params: { ...params, branchId: track.branch.id, hour: at } }, new AbortController().signal).then(reply => reply.data) : undefined;
  useRecordedRows(product, pushed && data !== undefined && at !== null && exactRead ? {
    hour: at, rows: () => recordedPanelRows(product, data), exact: async () => recordedPanelRows(product, await exactRead(at)),
  } : undefined);
  const shownThrough = () => following ? (pipe === null || !pipeline ? null : pipeline.view(pipe)?.through ?? null) : data === undefined ? null : at;
  return {
    data, error, retry: asked.retry, root, hour, live: shownLive, outside,
    structure: outside || !pushed ? undefined : model?.structure,
    exactRead, held: outside ? undefined : heldReading,
    workerFollow: pushed ? sub : undefined,
    followedWidget: following ? pipe ?? undefined : undefined,
    pipeline,
    at, atNow: () => at, shownAt: shownThrough,
    current: () => data,
    read: outside ? 'off' : following ? 'followed' : hour === null ? 'off' : 'asked',
  };
}
