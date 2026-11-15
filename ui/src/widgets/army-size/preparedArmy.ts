import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct, loadTransientProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useHeldFailure } from '../../dashboard/heldFailures';
import type { ArmySizeParams, ArmySizeReply, CountryIdentityRun, MissingSpan } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import { chartSummaryLines } from '../../charts/chartSummary';
import type { TimeChartProps } from '../../charts/TimeChart';
import { validateArmySizeReply } from './armySizeData';
import { countryValueAt, countryValuesModel } from '../../charts/race/countryValues';
import { liveSummaries, useTiles } from '../../dashboard/tiles/useTiles';
import { unrecordedReason } from '../../dashboard/tiles/view.ts';

type ArmyLine = { tag: string; summary: ChartSummary; identities?: CountryIdentityRun[] };

/** A read of a chart's view, one at a time per lane (`lane`: what is read): a view that moves while a read runs is
 * read once that one lands, and the newest landed reply stays shown meanwhile. It renders when a reply lands. */
function useViewReply<V, T>(lane: string | null, view: V | null, viewKey: string, active: boolean, read: (view: V, signal: AbortSignal) => Promise<T>) {
  const [state, setState] = useState<{ lane: string; viewKey: string; data?: T; error?: Failure; errorKey?: string }>();
  const [landed, setLanded] = useState(0), [attempt, setAttempt] = useState(0);
  const flight = useRef<{ lane: string; controller: AbortController } | null>(null);
  const wanted = useRef({ view, viewKey }); wanted.current = { view, viewKey };
  const reader = useRef(read); reader.current = read;
  const shown = useRef(state); shown.current = state;
  useEffect(() => {
    if (!active || lane == null || view == null || flight.current?.lane === lane) return;
    const held = shown.current;
    if (held?.lane === lane && (held.viewKey === viewKey && held.data !== undefined || held.errorKey === viewKey)) return;
    flight.current?.controller.abort();
    const controller = new AbortController(), asked = wanted.current;
    flight.current = { lane, controller };
    reader.current(asked.view as V, controller.signal).then(data => {
      if (!controller.signal.aborted) setState({ lane, viewKey: asked.viewKey, data });
    }, error => {
      if (!controller.signal.aborted) setState(before => ({ ...(before?.lane === lane ? before : { viewKey: asked.viewKey }), lane, error: asFailure(error), errorKey: asked.viewKey }));
    }).finally(() => {
      if (flight.current?.controller !== controller) return;
      flight.current = null;
      if (!controller.signal.aborted) setLanded(count => count + 1);
    });
    // The view's key is its identity; the view object is made again each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lane, viewKey, active, landed, attempt]);
  useEffect(() => () => { if (flight.current?.lane === lane) { flight.current.controller.abort(); flight.current = null; } }, [lane, active]);
  const current = state?.lane === lane ? state : undefined;
  return { data: current?.data, error: current?.errorKey === viewKey ? current.error : undefined,
    retry: () => { setState(undefined); setAttempt(count => count + 1); } };
}

/** The army chart's lines over its view. A chosen set of countries is drawn from tiles (an hourly view's bins an
 * hour), which a view following the head asks one per 256 bins of travel; a ranked view (its members change bin by
 * bin, and the summaries keep no rank yet) is read for the view, one read at a time. */
export function usePreparedArmy(client: DashboardClient, track: BoundTrack | undefined, params: ArmySizeParams | null, manifest: ArmySizeReply | undefined, active: boolean,
  live = false) {
  const key = JSON.stringify([track?.source.id, track?.source.revision, params]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const alive = useRef(owner); alive.current = owner;
  const pending = useRef(new Set<AbortController>());
  // Only the latest key's request may answer the chart's pending navigation.
  const navigation = useRef<AbortController | null>(null);
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const [navigationError, setNavigationError] = useState<{ owner: typeof owner; error: Failure }>();
  useEffect(() => () => { for (const request of pending.current) request.abort(); pending.current.clear(); }, [owner, active]);
  const [view, setView] = useState<{ key: string; from: number; to: number; width: number }>();
  const [tiled, setTiled] = useState<{ key: string; dense: boolean }>();
  // A view reported with the same density sets nothing: the chart reports one each interval while the head moves.
  const told = useRef(tiled); told.current = tiled;
  const ranked = params?.top != null;
  const dense = tiled?.key === key && tiled.dense;
  const tiles = useTiles(client, track && params && !ranked ? { source: track.source, branchId: params.branchId,
    family: { kind: 'army-size', measure: params.measure, countFor: params.countFor, top: null }, tags: params.tags, lastHour: track.branch.lastHour } : null, active, live);
  // Followed, a tile landing repaints the lines from this, never rendering the widget.
  const tileSource = tiles.live;
  const lineData = useMemo(() => !ranked && live ? liveSummaries(tileSource) : undefined, [ranked, live, tileSource]);
  const tileView = tiles.onView;
  // One summary read at a time while the chart's view moves, for the ranked views tiles do not draw.
  const content = useContentRevision(client);
  const shown = view?.key === key && ranked ? view : null;
  const incoming = useViewReply<{ from: number; to: number; width: number }, { series: (ArmyLine & { missing: MissingSpan[] })[] }>(
    track && params ? JSON.stringify([key, content]) : null, shown, JSON.stringify([shown?.from, shown?.to, shown?.width]), active,
    (at, signal) => loadProduct(client, { sourceId: track!.source.id, revision: track!.source.revision, product: 'series-summary',
      params: { branchId: params!.branchId, family: { kind: 'army-size', measure: params!.measure, countFor: params!.countFor, top: params!.top },
        tags: params!.tags, from: at.from, to: at.to, width: at.width } }, signal, value => {
      const series = chartSummaryLines<{ tag: string; summary: ChartSummary; identities?: CountryIdentityRun[]; missing: MissingSpan[] }>((value as { series?: unknown } | null)?.series,
        line => typeof line.tag === 'string' && Array.isArray(line.missing));
      if (!series) throw Error('The army summary is malformed.');
      return { ...value as object, series };
    }, track!.source));
  const fromTiles = !ranked;
  const lines = useMemo<ArmyLine[] | undefined>(() => fromTiles
    ? tiles.lines ? [...tiles.lines.values()].map(line => ({ tag: line.tag, summary: line.summary, identities: identityRuns(line.parts) })) : undefined
    : incoming.data?.series, [fromTiles, tiles.lines, incoming.data]);
  const summaries = useMemo(() => new Map(lines?.map(line => [line.tag, line.summary])), [lines]);
  const rankedUnrecorded = useMemo(() => unrecordedReason(incoming.data?.series), [incoming.data]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    const isDense = next.end - next.from + 1 > next.width;
    if (told.current?.key !== key || told.current.dense !== isDense) setTiled(told.current = { key, dense: isDense });
    if (!ranked) { tileView(next); return; }
    setView(before => before?.key === key && before.from === next.from && before.to === next.to && before.width === next.width ? before
      : { key, from: next.from, to: next.to, width: next.width });
  }, [key, ranked, tileView]);
  const exactLineTags = useCallback(async (hour: number) => {
    const window = manifest?.windows.findIndex(span => span.firstHour <= hour && hour <= span.lastHour) ?? -1;
    if (!track || !params || !active || window < 0) return null;
    const controller = new AbortController(); pending.current.add(controller);
    try {
      const reply = await loadProduct(client, { sourceId: track.source.id, revision: track.source.revision, product: 'army-size',
        params: { ...params, window } }, controller.signal, validateArmySizeReply, track.source);
      if (controller.signal.aborted || alive.current !== owner) return null;
      const model = countryValuesModel(reply);
      return model.tags.filter(tag => countryValueAt(model, tag, Math.floor(hour))?.total != null);
    } catch (error) {
      if (!controller.signal.aborted && alive.current === owner) setNavigationError({ owner, error: asFailure(error) });
      return null;
    } finally { pending.current.delete(controller); }
  }, [owner, manifest, active]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((action, from, x, until) => {
    if (!track || !params || !active) return;
    setNavigationError(undefined);
    navigation.current?.abort();
    const controller = new AbortController(); pending.current.add(controller); navigation.current = controller;
    void loadTransientProduct<{ hour: number | null }>(client, { sourceId: track.source.id, revision: track.source.revision, product: 'series-next-change', params: { branchId: params.branchId,
      family: { kind: 'army-size', measure: params.measure, countFor: params.countFor, top: params.top }, tags: params.tags,
      key: action, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) } }, controller.signal).then(value => {
      if (!controller.signal.aborted && alive.current === owner) setKeyboard(previous => ({ owner, hour: value.hour, revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && alive.current === owner) setNavigationError({ owner, error: asFailure(error) }); }).finally(() => pending.current.delete(controller));
  }, [owner, active]);
  const retry = () => { setNavigationError(undefined); incoming.retry(); tiles.retry(); };
  const navigationFailure = useHeldFailure(navigationError?.owner === owner ? navigationError.error : undefined,
    lines !== undefined || manifest !== undefined, retry);
  const tilesError = fromTiles && tiles.error ? asFailure(tiles.error) : undefined;
  return { dense, summaries, lineData, identities: useMemo(() => lines?.map(line => ({ tag: line.tag, identities: line.identities ?? [] })), [lines]), exactLineTags, onNavigateHour, keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    onSummaryView, ...incoming, data: lines === undefined ? undefined : { series: lines },
    error: (fromTiles ? tilesError : incoming.error) ?? navigationFailure, retry,
    /** Why no line holds a value in view, when none does. */
    unrecorded: fromTiles ? tiles.unrecorded : rankedUnrecorded };
}

/** A tiled line's holders as identity runs, a holder's parts on consecutive hours joined. */
function identityRuns(parts: readonly { firstHour: number; lastHour: number; slot: number; countryId: number }[]): CountryIdentityRun[] {
  const out: CountryIdentityRun[] = [];
  for (const part of [...parts].sort((a, b) => a.firstHour - b.firstHour)) {
    const held = out.at(-1);
    if (held && held.slot === part.slot && held.countryId === part.countryId && held.lastHour + 1 >= part.firstHour) held.lastHour = Math.max(held.lastHour, part.lastHour);
    else out.push({ firstHour: part.firstHour, lastHour: part.lastHour, slot: part.slot, countryId: part.countryId });
  }
  return out;
}
