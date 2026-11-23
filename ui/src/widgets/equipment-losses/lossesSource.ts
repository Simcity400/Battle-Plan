import type { Gate } from '../../dashboard/regionGate';
import { asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { TimeChartProps } from '../../charts/TimeChart';
import { loadProduct, loadTransientProduct, useProduct } from '../../dashboard/products.ts';
import { stretchSince } from '../../dashboard/stretch.ts';
import { nearest, stepHours, validateLossesReply, withEdges } from './lossesData.ts';
import type { LossWindow } from './lossesData';
import type { LossSource, LossesParams, LossesReply, NavigationKey } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';

const SERIES = { kind: 'equipment-losses', measure: 'total' };
const PART_BITS = 41n;

export function stretchWindow(reply: LossesReply, window: LossWindow): LossWindow {
  const since = stretchSince(reply.restarts, window.hour);
  if (since === null || window.from != null && window.from >= since) return { ...window, since: null };
  return { ...window, from: null, since };
}

export const exactLossHour = (reply: LossesReply, hour: number) => reply.exactHours.includes(hour);

export const leafPart = (key: string) => (BigInt(key) & ((1n << PART_BITS) - 1n)).toString();

export interface LossSelection { branchId: string; country: LossesParams['country']; landSource: LossesParams['landSource']; identity?: LossesParams['identity'] }

export function useLossHistory(client: DashboardClient, gate: Pick<Gate, 'within' | 'recorded' | 'country' | 'span'>, track: BoundTrack | undefined, selection: LossSelection | null,
  view: LossWindow | null, exact: readonly number[], active: boolean, open: boolean) {
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
  const to = view ? Math.floor(Math.min(view.end, geometry?.to ?? view.end)) : 0;
  const from = view ? Math.floor(Math.min(to, Math.max(view.first, geometry?.from ?? view.first))) : 0;
  const reads = view ? [...new Set([view.first, view.end, ...exact,
    ...(view.from == null ? [] : [view.from - 1]),
    ...(reachedHover === null || reachedHover < view.first || reachedHover > view.end ? [] : [reachedHover])])]
    .filter(hour => Number.isInteger(hour) && hour >= view.first - 1 && hour <= view.end).sort((a, b) => a - b)
    .flatMap(hour => { const read = gate.recorded(hour); return read === null ? [] : [read]; }) : [];
  const fromAt = gate.within(from), toAt = gate.within(to);
  const request = track && selection && view && fromAt !== null && toAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'equipment-losses', params: { ...selection, from: fromAt, to: toAt, width: Math.max(1, Math.round(geometry?.width ?? 64)), reads } }) : null;
  const read = useProduct(client, request, active, validateLossesReply, track?.source);
  const exactAt = useCallback(async (hour: number) => {
    if (!active || !track || !request) return null;
    const controller = new AbortController(); picks.current.add(controller);
    try {
      const asked = gate.recorded(hour);
      const reply = await loadProduct(client, { ...request, params: { ...request.params,
        reads: [...new Set([...request.params.reads ?? [], ...(asked === null ? [] : [asked])])].sort((a, b) => a - b) } }, controller.signal, validateLossesReply, track.source);
      return controller.signal.aborted || alive.current !== owner ? null : reply;
    } catch (failure) {
      if (!controller.signal.aborted && alive.current === owner) setError({ owner, failure: asFailure(failure) });
      return null;
    } finally { picks.current.delete(controller); }
  }, [owner, active, request]);
  const navigate = useCallback((key: NavigationKey, origin: number | null, x: readonly [number, number], until: number | null,
    leaves: readonly (readonly [LossSource, string])[], shipHours: readonly number[]) => {
    if (!active || !track || !selection || !view) return;
    const lastHour = Math.floor(Math.min(x[1], until ?? view.end, view.end));
    const firstHour = Math.ceil(Math.max(x[0], view.first));
    if (firstHour > lastHour) return;
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    const ships = stepHours(shipHours, key, origin, firstHour, lastHour);
    const land = (found: number | null) => {
      const hour = withEdges(key, found, origin, firstHour);
      if (!controller.signal.aborted && alive.current === owner) setKeyboard(before => ({ owner, hour, revision: (before?.revision ?? 0) + 1 }));
    };
    if (!leaves.length) { land(ships); return; }
    const held = gate.country?.history ?? null;
    const firstAt = gate.within(firstHour), lastAt = gate.within(lastHour);
    if (firstAt === null || lastAt === null) { land(ships); return; }
    void loadTransientProduct<{ hour: unknown }>(client, productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'series-next-change',
      params: { branchId: selection.branchId, family: SERIES, lines: leaves.map(([, leaf]) => ({ country: held ? countryFromKey(held.key) : selection.country, part: leafPart(leaf) })),
        ...(held ? { within: [[countryFromKey(held.key), held.spans.flatMap(span => gate.span(span.firstHour, span.lastHour) ?? [])] as const] } : {}),
        key, from: gate.within(origin), firstHour: firstAt, lastHour: lastAt } }), controller.signal).then(reply => {
      if (reply.hour !== null && !Number.isSafeInteger(reply.hour)) throw new Error('The equipment-loss navigation is malformed.');
      land(nearest(key, reply.hour as number | null, ships));
    }).catch(failure => { if (!controller.signal.aborted && alive.current === owner) setError({ owner, failure: asFailure(failure) }); });
  }, [owner, active, track, selection, view]);
  return { ...read, request, onSummaryView, onHover, exactAt, navigate,
    retry: () => { setError(undefined); read.retry(); },
    error: read.error ?? (error?.owner === owner ? error.failure : undefined),
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined };
}
