import type { Gate } from '../../dashboard/regionGate';
import { OUTSIDE_MESSAGE } from '../../dashboard/regionTime';
import { countryKey } from '../../dashboard/countryKey.ts';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct, loadTransientProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useHeldFailure } from '../../dashboard/heldFailures';
import type { ArmySizeParams, CountryIdentityRun, MissingSpan } from '../../generated/dashboard';
import type { ArmySizeReply } from './armySizeData';
import type { ChartSummary } from '../../charts/chartSummary';
import { chartSummaryLines } from '../../charts/chartSummary';
import type { TimeChartProps } from '../../charts/TimeChart';
import { validateArmySizeReply } from './armySizeData';
import { countryValueAt, countryValuesModel } from '../../charts/race/countryValues';
import { liveSummaries, useTiles } from '../../dashboard/tiles/useTiles';
import { unrecordedReason } from '../../dashboard/tiles/view.ts';
import { productRequest } from '../../dashboard/request.ts';

type ArmyLine = { tag: string; summary: ChartSummary; identities?: CountryIdentityRun[] };

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
  }, [lane, viewKey, active, landed, attempt]);
  useEffect(() => () => { if (flight.current?.lane === lane) { flight.current.controller.abort(); flight.current = null; } }, [lane, active]);
  const current = state?.lane === lane ? state : undefined;
  return { data: current?.data, error: current?.errorKey === viewKey ? current.error : undefined,
    retry: () => { setState(undefined); setAttempt(count => count + 1); } };
}

export function usePreparedArmy(client: DashboardClient, gate: Pick<Gate, 'span' | 'within'>, track: BoundTrack | undefined, params: ArmySizeParams | null,
  manifest: ArmySizeReply | undefined, active: boolean, live = false) {
  const key = JSON.stringify([track?.source.id, track?.source.revision, params]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const alive = useRef(owner); alive.current = owner;
  const pending = useRef(new Set<AbortController>());
  const navigation = useRef<AbortController | null>(null);
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const [navigationError, setNavigationError] = useState<{ owner: typeof owner; error: Failure }>();
  useEffect(() => () => { for (const request of pending.current) request.abort(); pending.current.clear(); }, [owner, active]);
  const [view, setView] = useState<{ key: string; from: number; to: number; width: number }>();
  const [tiled, setTiled] = useState<{ key: string; dense: boolean }>();
  const told = useRef(tiled); told.current = tiled;
  const ranked = params?.top != null;
  const dense = tiled?.key === key && tiled.dense;
  const tiles = useTiles(client, track && params && !ranked ? { source: track.source, span: gate.span, branchId: params.branchId,
    family: { kind: 'army-size', measure: params.measure, countFor: params.countFor, top: null }, tags: params.countries.map(countryKey), lastHour: track.branch.lastHour } : null, active, live);
  const tileSource = tiles.live;
  const lineData = useMemo(() => !ranked && live ? liveSummaries(tileSource) : undefined, [ranked, live, tileSource]);
  const tileView = tiles.onView;
  const content = useContentRevision(client);
  const shown = view?.key === key && ranked ? view : null;
  const incoming = useViewReply<{ from: number; to: number; width: number }, { series: (ArmyLine & { missing: MissingSpan[] })[] }>(
    track && params ? JSON.stringify([key, content]) : null, shown, JSON.stringify([shown?.from, shown?.to, shown?.width]), active,
    (at, signal) => {
      const range = gate.span(at.from, at.to);
      if (!range) return Promise.reject(new Error(OUTSIDE_MESSAGE));
      return loadProduct(client, productRequest({ sourceId: track!.source.id, revision: track!.source.revision, product: 'series-summary',
      params: { branchId: params!.branchId, family: { kind: 'army-size', measure: params!.measure, countFor: params!.countFor, top: params!.top },
        countries: params!.countries, from: range.firstHour, to: range.lastHour, width: at.width } }), signal, (value: unknown) => {
      const lines = (value as { series?: unknown } | null)?.series;
      const keyed = Array.isArray(lines) ? lines.map(line => line && typeof line === 'object' && (typeof line.country === 'string' || typeof line.country === 'number')
        ? (({ country, ...rest }) => ({ ...rest, tag: countryKey(country) }))(line) : line) : lines;
      const series = chartSummaryLines<{ tag: string; summary: ChartSummary; identities?: CountryIdentityRun[]; missing: MissingSpan[] }>(keyed,
        line => typeof line.tag === 'string' && Array.isArray(line.missing));
      if (!series) throw Error('The army summary is malformed.');
      return { ...value as object, series };
    }, track!.source);
    });
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
      const reply = await loadProduct(client, productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'army-size',
        params: { ...params, window } }), controller.signal, validateArmySizeReply, track.source);
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
    const range = gate.span(x[0], Math.min(x[1], until ?? x[1]));
    if (!range) return;
    void loadTransientProduct<{ hour: number | null }>(client, productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'series-next-change', params: { branchId: params.branchId,
      family: { kind: 'army-size', measure: params.measure, countFor: params.countFor, top: params.top }, countries: params.countries,
      key: action, from: gate.within(from), ...range } }), controller.signal).then(value => {
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
    unrecorded: fromTiles ? tiles.unrecorded : rankedUnrecorded };
}

function identityRuns(parts: readonly { firstHour: number; lastHour: number; slot: number; countryId: number }[]): CountryIdentityRun[] {
  const out: CountryIdentityRun[] = [];
  for (const part of [...parts].sort((a, b) => a.firstHour - b.firstHour)) {
    const held = out.at(-1);
    if (held && held.slot === part.slot && held.countryId === part.countryId && held.lastHour + 1 >= part.firstHour) held.lastHour = Math.max(held.lastHour, part.lastHour);
    else out.push({ firstHour: part.firstHour, lastHour: part.lastHour, slot: part.slot, countryId: part.countryId });
  }
  return out;
}
