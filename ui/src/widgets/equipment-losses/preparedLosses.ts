import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { LossesParams, LossesReply, LossSource } from '../../generated/dashboard';
import type { TimeChartProps } from '../../charts/TimeChart';
import { loadProduct, useProduct } from '../../dashboard/products.ts';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries.ts';
import { validateLossesReply } from './lossesData.ts';
import type { LossWindow } from './lossesData';

export interface PreparedLosses extends LossesReply {
  preparedReads: number[];
  preparedHeadHour: number;
  preparedDetailHour: number;
  preparedFrom: number;
  preparedTo: number;
  preparedWidth: number;
}
const hour = (value: unknown): value is number => Number.isInteger(value) && (value as number) >= -2147483648 && (value as number) <= 2147483647;
export function validatePreparedLosses(value: unknown): PreparedLosses {
  const reply = validateLossesReply(value) as PreparedLosses;
  if (!Array.isArray(reply.preparedReads) || !reply.preparedReads.every((read, index) => hour(read) && (index === 0 || reply.preparedReads[index - 1] < read))
    || !hour(reply.preparedHeadHour) || !hour(reply.preparedDetailHour)
    || !reply.preparedReads.includes(reply.preparedHeadHour) || !reply.preparedReads.includes(reply.preparedDetailHour)
    || !Number.isFinite(reply.preparedFrom) || !Number.isFinite(reply.preparedTo)
    || reply.preparedFrom > reply.preparedTo
    || !Number.isInteger(reply.preparedWidth) || reply.preparedWidth < 1) {
    throw new Error('The prepared equipment-loss reads are malformed.');
  }
  return reply;
}
export const preparedLossHour = (reply: PreparedLosses, hour: number) => reply.preparedReads.includes(hour);

/** Shared product routing holds one viewport and its reached exact dates. */
export function usePreparedLosses(client: DashboardClient, track: BoundTrack | undefined, selection: LossesParams | null,
  view: LossWindow | null, detailHour: number, active: boolean, open: boolean) {
  const key = JSON.stringify([track?.source.id, track?.source.revision, selection, view?.first, view?.end, view?.from]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const alive = useRef(owner); alive.current = owner;
  const [viewport, setViewport] = useState<{ owner: typeof owner; from: number; to: number; width: number }>();
  const [hover, setHover] = useState<{ owner: typeof owner; hour: number | null }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const [error, setError] = useState<{ owner: typeof owner; message: string }>();
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
  const product = useProduct<PreparedLosses>(client, request, active, validatePreparedLosses, track?.source);
  const exactAt = useCallback(async (hour: number) => {
    if (!active || !track || !request || !view) return null;
    const controller = new AbortController(); picks.current.add(controller);
    try {
      const reply = await loadProduct(client, { ...request, params: { ...request.params,
        reads: [...new Set([...request.params.reads, Math.floor(hour)])].sort((a, b) => a - b) } },
      controller.signal, validatePreparedLosses, track.source);
      return controller.signal.aborted || alive.current !== owner ? null : reply;
    } catch (failure) {
      if (!controller.signal.aborted && alive.current === owner) setError({ owner, message: String(failure) });
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
    }).catch(failure => { if (!controller.signal.aborted && alive.current === owner) setError({ owner, message: String(failure) }); });
  }, [owner, active, track, selection, view]);
  return { ...product, request, onSummaryView, onHover, exactAt, navigate,
    retry: () => { setError(undefined); product.retry(); },
    error: product.error ?? (error?.owner === owner ? error.message : undefined),
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined };
}
