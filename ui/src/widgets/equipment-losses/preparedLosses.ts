import { asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { LossesParams, LossesReply, LossSource } from '../../generated/dashboard';
import type { TimeChartProps } from '../../charts/TimeChart';
import { loadProduct } from '../../dashboard/products.ts';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { useHeldFailure } from '../../dashboard/heldFailures.ts';
import { useViewRead } from '../../dashboard/viewRead.ts';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries.ts';
import { validateLossesReply } from './lossesData.ts';
import type { LossWindow } from './lossesData';
import { preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import type { PreparedPrefixFields } from '../../dashboard/preparedPrefix';
import { stretchSince } from '../../dashboard/stretch.ts';

export interface PreparedLosses extends LossesReply {
  preparedReads: number[];
  preparedHeadHour: number;
  preparedDetailHour: number;
  preparedFrom: number;
  preparedTo: number;
  preparedWidth: number;
  /** The first hour of each stretch after hours the recording lost, where every total restarts. */
  restarts: number[];
  /** The prepared prefix the reply answers: the preparation coverage it reads
   * through and the requested hours still being prepared. */
  prefix?: PreparedPrefixFields | null;
}
const hour = (value: unknown): value is number => Number.isInteger(value) && (value as number) >= -2147483648 && (value as number) <= 2147483647;
/** The reply's root `coverage` and `pending` are its prepared prefix, which the
 * query loop follows; its land coverage spans travel as `landCoverage`. */
export function validatePreparedLosses(value: unknown): PreparedLosses {
  if (!value || typeof value !== 'object' || !('landCoverage' in value)) throw new Error('The prepared equipment-loss reads are malformed.');
  const prefix = preparedPrefix(value);
  const { landCoverage, coverage: _prepared, pending: _pending, ...rest } = value as Record<string, unknown>;
  const reply = validateLossesReply({ ...rest, coverage: landCoverage }) as PreparedLosses;
  if (!Array.isArray(reply.preparedReads) || !reply.preparedReads.every((read, index) => hour(read) && (index === 0 || reply.preparedReads[index - 1] < read))
    || !hour(reply.preparedHeadHour) || !hour(reply.preparedDetailHour)
    || !reply.preparedReads.includes(reply.preparedHeadHour) || !reply.preparedReads.includes(reply.preparedDetailHour)
    || !Number.isFinite(reply.preparedFrom) || !Number.isFinite(reply.preparedTo)
    || reply.preparedFrom > reply.preparedTo
    || !Number.isInteger(reply.preparedWidth) || reply.preparedWidth < 1
    || !Array.isArray(reply.restarts) || !reply.restarts.every((restart, index) => hour(restart) && (index === 0 || reply.restarts[index - 1] < restart))) {
    throw new Error('The prepared equipment-loss reads are malformed.');
  }
  reply.prefix = prefix;
  return reply;
}
export const preparedLossHour = (reply: PreparedLosses, hour: number) => reply.preparedReads.includes(hour);

/** The window a reply answers for `view`: the requested hour, or the reply's
 * head hour when it read another; and before preparation reaches the window,
 * through the prepared prefix's last hour, since every loss is a cumulative
 * signed total. Null when the prefix has not reached a range's start. */
export function preparedLossWindow(reply: PreparedLosses, view: LossWindow): LossWindow | null {
  const hour = preparedLossHour(reply, view.hour) ? view.hour : reply.preparedHeadHour;
  const prefix = reply.prefix;
  if (!prefix?.pending.length) return stretchWindow(reply, { ...view, hour });
  const last = prefix.coverage.lastHour;
  if (view.from != null && view.from > last) return null;
  return stretchWindow(reply, { ...view, hour: Math.min(hour, last), end: Math.min(view.end, last) });
}

/** `window` counted within the stretch its hour lies in: totals restart after hours the
 * recording lost, so one reaching back before them counts from its stretch's first hour,
 * `since`, and never across them. */
export function stretchWindow(reply: PreparedLosses, window: LossWindow): LossWindow {
  const since = stretchSince(reply.restarts, window.hour);
  if (since === null || window.from != null && window.from >= since) return { ...window, since: null };
  return { ...window, from: null, since };
}

/** Shared product routing holds one viewport and its reached exact dates. */
export function usePreparedLosses(client: DashboardClient, track: BoundTrack | undefined, selection: LossesParams | null,
  view: LossWindow | null, detailHour: number, active: boolean, open: boolean) {
  const key = JSON.stringify([track?.source.id, track?.source.revision, selection, view?.first, view?.end, view?.from]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const alive = useRef(owner); alive.current = owner;
  const [viewport, setViewport] = useState<{ owner: typeof owner; from: number; to: number; width: number }>();
  const [hover, setHover] = useState<{ owner: typeof owner; hour: number | null }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const [error, setError] = useState<{ owner: typeof owner; failure: Error }>();
  const navigation = useRef<AbortController | null>(null), picks = useRef(new Set<AbortController>());
  useEffect(() => () => { navigation.current?.abort(); for (const controller of picks.current) controller.abort(); picks.current.clear(); }, [owner, active]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    setViewport(before => before?.owner === owner && before.from === next.from && before.to === next.to && before.width === next.width
      ? before : { owner, ...next });
  }, [owner]);
  const onHover = useCallback((hour: number | null) => {
    const reached = hour === null ? null : Math.floor(hour);
    setHover(before => before?.owner === owner && before.hour === reached ? before : { owner, hour: reached });
  }, [owner]);
  const reachedHover = hover?.owner === owner ? hover.hour : null;
  const geometry = open && viewport?.owner === owner ? viewport : undefined;
  const to = view ? Math.min(view.end, geometry?.to ?? view.hour) : 0;
  const from = view ? Math.min(to, Math.max(view.first, geometry?.from ?? view.hour)) : 0;
  const reads = view ? [...new Set([view.first, view.hour, view.end, detailHour,
    ...(view.from == null || view.from <= -2147483648 ? [] : [view.from - 1]),
    ...(reachedHover === null || reachedHover < view.first || reachedHover > view.end ? [] : [reachedHover])])].sort((a, b) => a - b) : [];
  const request = track && selection && view ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-losses',
    params: { selection, from, to, width: geometry?.width ?? 1, reads, headHour: view.hour, detailHour } } : null;
  // One read at a time while the chart's view and the reached hours move (`useViewRead`);
  // `preparedLossWindow` places a reply asked for an older head hour.
  const content = useContentRevision(client);
  const asked = request?.params;
  const read = useViewRead<NonNullable<typeof asked>, PreparedLosses>(request ? JSON.stringify([key, content]) : null, asked,
    JSON.stringify([asked?.from, asked?.to, asked?.width, asked?.reads, asked?.headHour, asked?.detailHour]), active,
    (at, signal, publish, restart) => loadProduct(client, { ...request!, params: at }, signal, validatePreparedLosses, track?.source, restart, publish), client);
  const product = { ...read, error: useHeldFailure(read.error, read.data !== undefined, read.retry) };
  const exactAt = useCallback(async (hour: number) => {
    if (!active || !track || !request || !view) return null;
    const controller = new AbortController(); picks.current.add(controller);
    try {
      const reply = await loadProduct(client, { ...request, params: { ...request.params,
        reads: [...new Set([...request.params.reads, Math.floor(hour)])].sort((a, b) => a - b) } },
      controller.signal, validatePreparedLosses, track.source);
      return controller.signal.aborted || alive.current !== owner ? null : reply;
    } catch (failure) {
      if (!controller.signal.aborted && alive.current === owner) setError({ owner, failure: asFailure(failure) });
      return null;
    } finally { picks.current.delete(controller); }
  }, [owner, active, request, view]);
  const navigate = useCallback((key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', origin: number | null,
    x: readonly [number, number], until: number | null, leaves: readonly (readonly [LossSource, number | null])[]) => {
    if (!active || !track || !selection || !view) return;
    const lastHour = Math.floor(Math.min(x[1], until ?? view.end, view.end));
    const firstHour = Math.ceil(Math.max(x[0], view.first));
    if (firstHour > lastHour) return;
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    void preparedSeriesQuery(client, track.source, 'series-next-change', { branchId: selection.branchId,
      family: { kind: 'equipment-losses', selection }, leaves, key, from: origin, firstHour, lastHour }, controller.signal).then(value => {
      const reply = value as { hour: unknown };
      if (reply.hour !== null && (typeof reply.hour !== 'number' || !Number.isFinite(reply.hour))) throw new Error('The prepared equipment-loss navigation is malformed.');
      if (!controller.signal.aborted && alive.current === owner) setKeyboard(before => ({ owner, hour: reply.hour as number | null, revision: (before?.revision ?? 0) + 1 }));
    }).catch(failure => { if (!controller.signal.aborted && alive.current === owner) setError({ owner, failure: asFailure(failure) }); });
  }, [owner, active, track, selection, view]);
  return { ...product, request, onSummaryView, onHover, exactAt, navigate,
    retry: () => { setError(undefined); product.retry(); },
    error: product.error ?? (error?.owner === owner ? error.failure : undefined),
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined };
}
