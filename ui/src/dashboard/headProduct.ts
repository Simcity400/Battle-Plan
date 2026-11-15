import { useCallback, useContext, useMemo, useRef, useState } from 'react';
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
import { canonical } from './follow/store.ts';
import { followedCells, repliedCells, sectionOf } from './follow/liveCells.ts';
import type { Live, LiveValues } from '../charts/live';
import { PreparingPage } from './preparing.ts';

/** How a rows-at-an-hour product is read for a widget: `follows` while it follows playback and nothing is hovered
 * (a pushed panel), else asked once for `hourAt` the clock's hour: the pinned hour, a range's end or the hovered
 * hour, null outside the recorded branch. */
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

/** Where a rows-at-an-hour product's data comes from: pushed once the service follows it and its first panel has
 * landed; until then, and wherever the service does not follow it, asked by the fallback's clock, so a seek before the
 * first push shows the hour sought, never a reply held from an earlier hour. Following but seen off screen, read not
 * at all (`idle`: nothing of it shows, and it is followed again when it comes back). Not following, or not watched,
 * asked. */
export function headSource(following: boolean, status: 'off' | FollowEntry['status'], landed: boolean, hidden = false): 'pushed' | 'fallback' | 'asked' | 'idle' {
  if (following && hidden) return 'idle';
  if (!following || status === 'off') return 'asked';
  return status === 'followed' && landed ? 'pushed' : 'fallback';
}

export interface HeadProductOptions<T> {
  /** The widget is on the page and shown. */
  active: boolean;
  validate: (data: unknown) => T;
  /** What renders: a followed widget re-renders only when this changes (which rows, not their numbers), at most once a
   * slot of the page's shared structure pace while playing. Without it, every reply that differs renders, as paced. */
  structure?: (reply: T) => string;
  /** Writes a followed reply's changing numbers into the page in the animation frame, never by a render. */
  paint?: (reply: T) => void;
  /** An hour the reader points at: asked once, whatever the widget follows. */
  hoverHour?: number | null;
}

export interface HeadProduct<T> extends ProductState<T> {
  /** The element the product is shown in: followed only while it is on screen. */
  root: (element: Element | null) => void;
  /** How `data` was read: pushed at the head, asked for its hour, asked by the fallback's clock because the service
   * does not follow it (`reason` says why), or not at all (no params, or the hour is outside the branch). */
  read: 'followed' | 'asked' | 'fallback' | 'off';
  reason?: string;
  /** The track hour `data` was asked for; null while followed, where the head moves on without a render. */
  hour: number | null;
  /** The hour the shown structure is of: the push that made it while followed, else the hour its shown reply was asked
   * for (an earlier reply shown while the newest loads is of its own hour); null before any. */
  at: number | null;
  /** `at` read now: a panel landed again unchanged at another hour (a seek) moves it with no render, telling `live`. */
  atNow: () => number | null;
  /** The live cells the product names (`liveCells.ts`), keyed `liveKey(row, column)`: followed at the head and painted
   * each frame with no render, or the asked reply's values. One source whichever way the reply was read. */
  live: Live<LiveValues<number>>;
  /** The hour the values `live` gives now are of, for `useShownHour`: their followed answers' through hour, or the hour
   * asked; null while none is shown. Read where they are painted. */
  shownAt: () => number | null;
  /** The newest reply, read where it is painted: pushed, the last panel landed, whether or not its `structure` rendered
   * (`live` tells when one lands); else `data`. */
  current: () => T | undefined;
}

/** Why each product was read by the fallback's clock, for the designer's list; the perf log has each follow's too. */
export const headFallbacks = new Map<string, string>();

const NO_PAINT = () => {};
const MOVING = ['hour'] as const;

/** The hour a followed read's shown reply was asked for (`useFollowedProduct`'s `moment`, its moving params). */
export function momentHour(moment: string | undefined): number | null {
  if (!moment) return null;
  const [hour] = JSON.parse(moment) as unknown[];
  return typeof hour === 'number' ? hour : null;
}

/** What a reply is of: its history (source, revision, branch), product and params; null where nothing is read. */
export function headSubject(track: Pick<BoundTrack, 'source' | 'branch'> | undefined, product: string, params: Readonly<Record<string, unknown>> | null): string | null {
  return track && params ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, product, canonical(params)]) : null;
}

/** The last reply asked, with the hour it is of, and the client and subject it was asked of. */
export interface Retained<T> { client: unknown; subject: string; data: T; hour: number | null }

/** What is kept to show while a newer reply is on the way: `asked` once it lands, else `held` while the client and
 * subject stand, nothing once either changes. */
export function retain<T>(held: Retained<T> | null, client: unknown, subject: string | null, asked: T | undefined, hour: number | null): Retained<T> | null {
  if (subject === null) return null;
  if (asked !== undefined) return { client, subject, data: asked, hour };
  return held && held.client === client && held.subject === subject ? held : null;
}

/** A rows-at-an-hour product (a table, a tree, cards) for `widget`. Following playback, it is followed as a pushed
 * panel (`{panel: product, branchId, ...params}`): no request while playing, and a render only when `structure`
 * of the reply changes, `paint` writing its numbers between. Pinned, on a range or hovering an hour, one plain
 * request for that hour (`{branchId, hour, ...params}`). Where the service does not follow it, the head is asked
 * by the fallback's clock and `reason` kept. `params` null reads nothing. */
export function useHeadProduct<T>(runtime: WidgetRuntime, widget: WidgetSpec, product: string, params: Readonly<Record<string, unknown>> | null,
  options: HeadProductOptions<T>): HeadProduct<T> {
  const { active, validate, structure, paint, hoverHour = null } = options;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { follows, hourAt } = headRead(widget, track, runtime.tracks, hoverHour);
  // A prepared copy asks what a shown widget is pushed, so the reply its fallback asks first is held when it shows.
  const prepared = useContext(PreparingPage);
  const following = active && follows && !prepared && !!track && !!params;
  const paramsKey = params ? canonical(params) : null;
  const [element, setElement] = useState<Element | null>(null);
  const target = useMemo(() => ({ current: element }), [element]);
  const store = useMemo(() => track ? followStore(runtime.client, track.source, runtime.clock, track.axisOffset) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const query = useMemo(() => following ? { ...params, panel: product, branchId: track!.branch.id } : null,
    // `params` is compared by its canonical text.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [following, product, paramsKey, track?.branch.id]);
  const latest = useRef({ validate, structure, paint });
  latest.current = { validate, structure, paint };
  const shape = useCallback((_answer: FollowedAnswer | undefined, entry: FollowEntry) => {
    const { data, error } = panelReply(entry.panel, latest.current.validate);
    if (error) return `!${error.message}`;
    if (data === undefined) return '';
    return latest.current.structure ? `=${latest.current.structure(data)}` : `#${entry.version}`;
  }, []);
  const painting = useCallback((entry: FollowEntry) => {
    const { data } = panelReply(entry.panel, latest.current.validate);
    if (data !== undefined) latest.current.paint?.(data);
  }, []);
  const followed = useFollow(store, query, target, paint ? painting : NO_PAINT, shape, observeOnScreen, runtime.clock);
  const entry = followed.entry;
  const source = headSource(following, followed.status, !!entry?.panel, followed.hidden);
  const idle = source === 'idle';
  const pushed = source === 'pushed';
  const panelLive = useMemo(() => store && entry ? followedCells({
    section: () => entry.panel && entry.panel.refused === null ? sectionOf(entry.panel.data) : null,
    group: id => store.cellGroup(id),
    subscribe: changed => store.watchLive(entry, changed),
  }) : null, [store, entry]);
  const fallback = source === 'fallback';
  const paced = useFallbackHour(runtime.clock, fallback);
  const hour = pushed || idle || !track || !params ? null : hourAt(fallback ? paced : runtime.clock.getHour());
  const asked = useFollowedProduct<T>(runtime.client, hour === null ? null : { sourceId: track!.source.id, revision: track!.source.revision,
    product, params: { ...params, branchId: track!.branch.id, hour } }, MOVING, active, validate, track?.source);
  const askedAt = asked.data === undefined ? null : momentHour(asked.moment) ?? hour;
  // The last reply asked, with the hour it is of, stays shown while the first push is on the way.
  const held = useRef<Retained<T> | null>(null);
  held.current = retain(held.current, runtime.client, headSubject(track, product, params), asked.data, askedAt);
  const fromPanel = pushed ? panelReply(followed.entry?.panel, validate) : {};
  const kept = held.current ?? undefined;
  const data: T | undefined = pushed ? fromPanel.data ?? kept?.data : asked.data ?? (fallback || idle ? kept?.data : undefined);
  const dataAt = asked.data !== undefined ? askedAt : fallback || idle ? kept?.hour ?? null : hour;
  const error: Failure | undefined = pushed ? fromPanel.error : asked.error;
  const askedData = pushed ? undefined : data;
  const askedLive = useMemo(() => repliedCells(askedData), [askedData]);
  const live = pushed && panelLive ? panelLive : askedLive;
  if (followed.status === 'unavailable' && followed.reason) headFallbacks.set(product, followed.reason);
  return {
    data, error, retry: asked.retry, root: setElement, hour, live,
    at: pushed ? entry?.panel ? entry.hour : null : dataAt,
    atNow: pushed && entry ? () => entry.panel ? entry.hour : null : () => dataAt,
    shownAt: pushed && store && entry ? () => store.liveThrough(entry) : () => data === undefined ? null : dataAt,
    current: pushed && entry ? () => panelReply(entry.panel, latest.current.validate).data ?? data : () => data,
    // Off screen while following, what it last showed is kept as the fallback keeps it, never read as outside the branch.
    read: pushed ? 'followed' : fallback || idle ? 'fallback' : hour === null ? 'off' : 'asked',
    ...(followed.status === 'unavailable' && followed.reason ? { reason: followed.reason } : {}),
  };
}
