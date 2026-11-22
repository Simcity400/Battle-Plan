import { useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { useFollowedProduct } from './products.ts';
import type { ProductState } from './products';
import type { Failure } from './queryError';
import { followStore } from './follow/connection.ts';
import { panelReply } from './follow/panel.ts';
import { observeOnScreen, screenHolder, useFallbackHour, useFollow } from './follow/useFollow.ts';
import type { FollowedAnswer } from './follow/message';
import type { FollowEntry, FollowStore, Landed } from './follow/store';
import { canonical } from './follow/canonical.ts';
import { repliedCells } from './follow/liveCells.ts';
import type { Live, LiveValues } from '../charts/live';
import { PreparingPage } from './preparing.ts';
import { coverageOf, useReadsRegion, watchBoundary, type ReadCoverage } from './widgetTime.ts';
import { axisAt, axisHour, regionFollows, regionHour, type AxisHour, type GatedParams, type Region, type TrackHour } from './regionTime.ts';
import type { Gate } from './regionGate';
import { useRecordedRows } from './widgetPerf.ts';
import { recordedPanelRows } from '../draw/panels/views.ts';

export function headRead(region: Region, track: BoundTrack | undefined, tracks: readonly BoundTrack[]): { follows: boolean; hourAt: (axis: AxisHour) => TrackHour | null } {
  return { follows: regionFollows(region), hourAt: axis => regionHour(region, track, tracks, axis) };
}

export function headSource(following: boolean, status: 'off' | FollowEntry['status'], landed: boolean, hidden = false): 'pushed' | 'fallback' | 'asked' | 'idle' {
  if (following && hidden) return 'idle';
  if (!following || status === 'off') return 'asked';
  return status === 'followed' && landed ? 'pushed' : 'fallback';
}

export { watchBoundary };

export const heldPanel = (entry: FollowEntry | null): Landed | undefined => entry && !entry.panel ? entry.landed ?? undefined : undefined;

export interface HeldReading<T> { data: T; hour: number; through: number; live: Live<LiveValues<number>> }

export function withHeld<T>(head: HeadProduct<T>): HeadProduct<T> {
  const held = head.held;
  return head.data !== undefined || !held ? head
    : { ...head, data: held.data, live: held.live, at: held.hour, atNow: () => held.hour, shownAt: () => held.through };
}

function followedLive(store: FollowStore, entry: FollowEntry, read: () => FollowEntry['values']): Live<LiveValues<number>> {
  let held: FollowEntry['values'];
  let values: LiveValues<number>;
  return {
    read() {
      const next = read();
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
}

export interface HeadProductOptions<T> {
  paint?: (reply: T) => boolean | void;
  active: boolean;
  validate: (data: unknown) => T;
  structure?: (reply: T, hour: number) => string;
}

export interface HeadProduct<T> extends ProductState<T>, ReadCoverage {
  exactRead?: (hour: number) => Promise<unknown>;
  held?: HeldReading<T>;
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
export const shownAsked = (hour: number | null, paused: number | null) => paused === null || hour === paused;
type TargetHour = { exact: number | null } | { through: number };

export function retain<T>(held: Retained<T> | null, client: unknown, subject: string | null, asked: T | undefined, hour: number | null, target?: TargetHour): Retained<T> | null {
  if (subject === null) return null;
  const eligible = (at: number | null) => !target || (at !== null && ('exact' in target ? at === target.exact : at <= target.through));
  if (asked !== undefined && eligible(hour)) return { client, subject, data: asked, hour };
  return held && held.client === client && held.subject === subject && eligible(held.hour) ? held : null;
}

export function useHeadProduct<T>(runtime: WidgetRuntime, gate: Gate, product: string, wanted: GatedParams | null,
  options: HeadProductOptions<T>): HeadProduct<T> {
  const { active, validate, structure, paint } = options;
  const { region, track } = gate;
  const params = gate.open ? wanted : null;
  const follows = regionFollows(region);
  const hourAt = gate.hourAt;
  const prepared = useContext(PreparingPage);
  const following = active && follows && !prepared && !!track && !!params;
  const paramsKey = params ? canonical(params) : null;
  const [element, setElement] = useState<Element | null>(null);
  const root = useCallback((next: Element | null) => { if (next) setElement(screenHolder(next)); }, []);
  const target = useMemo(() => ({ current: element }), [element]);
  const store = useMemo(() => track ? followStore(runtime.client, track.source, runtime.clock, track.axisOffset) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const query = useMemo(() => following ? gate.follow({ ...params, panel: product, branchId: track!.branch.id }) : null,
    [following, product, paramsKey, track?.branch.id, gate.follow]);
  const latest = useRef({ validate, structure, paint });
  latest.current = { validate, structure, paint };
  const shape = useCallback((_answer: FollowedAnswer | undefined, entry: FollowEntry) => {
    const { data, error } = panelReply(entry.panel, latest.current.validate);
    if (error) return `!${error.message}`;
    if (data === undefined) return entry.landed ? '~' : entry.landed === null ? '-' : '';
    return latest.current.structure ? `=${latest.current.structure(data, entry.hour)}` : `#${entry.version}`;
  }, []);
  const painting = useCallback((entry: FollowEntry) => {
    const { data } = panelReply(entry.panel, latest.current.validate);
    if (data !== undefined) return latest.current.paint?.(data);
  }, []);
  const followed = useFollow(store, query, target, paint ? painting : null, shape, observeOnScreen);
  const outside = gate.outside !== null;
  useReadsRegion(region, active);
  const entry = followed.entry;
  const source = headSource(following, followed.status, entry?.landed !== undefined, followed.hidden);
  const idle = source === 'idle';
  const pushed = source === 'pushed';
  const panelLive = useMemo(() => store && entry ? followedLive(store, entry, () => entry.values) : null, [store, entry]);
  const heldLive = useMemo(() => store && entry ? followedLive(store, entry, () => entry.landed?.values) : null, [store, entry]);
  const fallback = source === 'fallback';
  const requestHour = useFallbackHour(runtime.clock, fallback);
  const hour = pushed || idle || !track || !params ? null : hourAt(fallback ? axisAt(requestHour) : axisHour(runtime.clock));
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
  const fromPanel = pushed ? panelReply(entry?.panel, validate) : {};
  const landed = pushed ? heldPanel(entry) : undefined;
  const heldData = landed ? panelReply(landed.panel, validate).data : undefined;
  const heldReading = useMemo(() => heldData !== undefined && landed && heldLive ? { data: heldData, hour: landed.hour, through: landed.through, live: heldLive } : undefined,
    [heldData, landed, heldLive]);
  const kept = outside || pushed ? undefined : held.current ?? undefined;
  const shown = kept && shownAsked(kept.hour, follows && !runtime.clock.getSnapshot().playing ? hourAt(axisHour(runtime.clock)) : null) ? kept : undefined;
  const data = outside ? undefined : pushed ? fromPanel.data : shown?.data;
  const dataAt = shown?.hour ?? null;
  const error: Failure | undefined = outside ? undefined : pushed ? fromPanel.error : asked.error;
  const askedData = pushed ? undefined : data;
  const askedLive = useMemo(() => repliedCells(askedData), [askedData]);
  const askedHeld = kept && !shown && !asked.error ? kept : undefined;
  const askedReading = useMemo(() => askedHeld && askedHeld.hour !== null
    ? { data: askedHeld.data, hour: askedHeld.hour, through: askedHeld.hour, live: repliedCells(askedHeld.data) } : undefined, [askedHeld]);
  const live = pushed && panelLive ? panelLive : askedLive;
  if (followed.status === 'unavailable' && followed.reason) headFallbacks.set(product, followed.reason);
  const exactRead = perfLog && track && params ? (asked: number) => {
    const at = gate.recorded(asked);
    if (at === null) return Promise.reject(new Error('This hour was not recorded.'));
    const request = { sourceId: track.source.id, revision: track.source.revision, product, params: { ...params, branchId: track.branch.id, hour: at } };
    return perfLog!.exactOnce(JSON.stringify(request), async () => (await runtime.client.product(request, new AbortController().signal)).data);
  } : undefined;
  const committedHour = entry?.hour ?? null;
  useRecordedRows(product, exactRead && pushed && data !== undefined && committedHour !== null ? {
    hour: committedHour, rows: () => recordedPanelRows(product, data),
    exact: async () => recordedPanelRows(product, await exactRead(committedHour)),
  } : undefined);
  return {
    data, error, retry: asked.retry, root, hour, live, ...coverageOf(gate.outside),
    exactRead, held: outside ? undefined : pushed ? heldReading : askedReading,
    workerFollow: pushed && runtime.client.openFollow ? entry?.id : undefined,
    at: outside ? null : pushed ? entry?.panel ? entry.hour : null : dataAt,
    atNow: outside ? () => null : pushed && entry ? () => entry.panel ? entry.hour : null : () => dataAt,
    shownAt: outside ? () => null : pushed && entry ? () => entry.panel ? entry.through : null : () => data === undefined ? null : dataAt,
    current: outside ? () => undefined : pushed && entry ? () => panelReply(entry.panel, latest.current.validate).data
      : () => follows && dataAt !== null && dataAt > runtime.clock.getHour() + (track?.axisOffset ?? 0) ? undefined : data,
    read: outside ? 'off' : pushed ? 'followed' : fallback || idle ? 'fallback' : hour === null ? 'off' : 'asked',
    ...(followed.status === 'unavailable' && followed.reason ? { reason: followed.reason } : {}),
  };
}
import { perfLog } from '../shell/perf.ts';
