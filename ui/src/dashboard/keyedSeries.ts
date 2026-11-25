import { countryFromKey } from './countryKey.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ChartSummary } from '../charts/chartSummary';
import type { MarkSummary } from '../charts/markSummary';
import type { TimeChartProps } from '../charts/TimeChart';
import type { Range } from '../charts/view';
import type { HourSpan } from '../generated/dashboard';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';
import { validateProductReply } from './client.ts';
import { useContentRevision } from './contentRevision.ts';
import { useHeldFailure } from './heldFailures.ts';
import { loadProduct, loadTransientProduct, useProducts } from './products.ts';
import { type Failure, asFailure } from './queryError.ts';
import { useViewRead } from './viewRead.ts';
import type { BoundWindow, CoveredBranch, GatedHour } from './regionTime.ts';
import type { Gate } from './regionGate';

export type SeriesGate = Pick<Gate, 'follow' | 'span' | 'within' | 'track'>;

export interface SeriesOwner { readonly key: string; readonly spans: readonly HourSpan[] | null }

export function ownerKeys(owner: SeriesOwner | null, parts: readonly string[]): { keys: SeriesKey[]; within: ReadonlyMap<string, readonly HourSpan[]> | undefined } {
  const keys = owner ? parts.map(part => ({ tag: owner.key, part })) : [];
  return { keys, within: owner?.spans ? new Map(keys.map(key => [keyId(key), owner.spans!])) : undefined };
}
import { followStore } from './follow/store.ts';
import { rowAmount } from './follow/painters.ts';
import { followFailure, followRefusal, followsHead, useFollow } from './follow/useFollow.ts';
import type { FollowEntry, FollowStore } from './follow/store';
import type { PlaybackController } from './playback';
import { followAtHead, tileHead } from './tiles/head.ts';
import type { TileReply } from './tiles/tile';
import { TileView, tileCache, unrecordedReason } from './tiles/view.ts';
import type { TiledLine, TileViewport } from './tiles/view';
import { followedRow, heldParts, keyId, keyedTileParams, keyedTileReply, reachedKeyLines, validateKeyedStates, validateSeriesParts } from './keyedSeriesData.ts';
import type { HeldParts, KeyedStates, SeriesKey, SeriesParts } from './keyedSeriesData';
import { productRequest } from './request.ts';
import { forwardRequest, type DeclaredProduct } from './request.ts';
import type { ParamsFor } from './regionTime';

export interface KeyedSeriesSpec {
  source: DashboardSource;
  branch: CoveredBranch & { id: string };
  family: Readonly<Record<string, unknown>>;
  keys: readonly SeriesKey[];
  partBits: number;
  within?: ReadonlyMap<string, readonly HourSpan[]>;
  lastHour: number;
  marks?: boolean;
}

const NO_PAINT = () => {};

const keyedLane = (spec: KeyedSeriesSpec) => `${spec.source.id}|${spec.source.revision}|${spec.branch.id}|${JSON.stringify(spec.family)}|keyed`;

export function useKeyedSeries(client: DashboardClient, clock: PlaybackController, gate: SeriesGate, spec: KeyedSeriesSpec | null, headHour: GatedHour | null,
  hoverHour: GatedHour | null, following: boolean, active: boolean, root: { readonly current: Element | null }) {
  const enabled = !!spec && spec.keys.length > 0;
  const ids = useMemo(() => spec?.keys.map(keyId) ?? [], [spec?.keys]);
  const ownerKey = spec ? JSON.stringify([spec.source.id, spec.source.revision, spec.branch.id, spec.family, ids, spec.within && [...spec.within]]) : null;
  const owner = useMemo(() => ({ client, key: ownerKey }), [client, ownerKey]);
  const alive = useRef(owner); alive.current = owner;
  const latest = useRef(spec); latest.current = spec;
  const request = <K extends DeclaredProduct>(product: K, params: ParamsFor<K>): ProductRequest => forwardRequest(spec!.source, product,
    { branchId: spec!.branch.id, family: spec!.family, ...params } as unknown as ParamsFor<K>);

  const tileKey = spec ? JSON.stringify([ownerKey, spec.lastHour, spec.marks === true]) : null;
  const [tiles, setTiles] = useState<{ key: string; lines: ReadonlyMap<string, TiledLine> | null; error: unknown }>();
  const view = useRef<TileView | null>(null);
  const viewport = useRef<TileViewport | null>(null);
  const newest = useRef<{ key: string; lines: ReadonlyMap<string, TiledLine> | null } | null>(null);
  const listeners = useRef(new Set<() => void>()).current;
  useEffect(() => {
    const lane = latest.current;
    if (!active || !enabled || !tileKey || !lane) return;
    let shown: { lines: boolean; error: unknown } | null = null;
    const byId = new Map(lane.keys.map(key => [keyId(key), key]));
    const ask = async (binHours: number, index: number, asked: readonly string[], within: Record<string, HourSpan[]> | undefined) => {
      const keys = asked.map(id => byId.get(id)!);
      const tileRequest = request('series-tiles', { binHours, index, ...keyedTileParams(keys, within, gate.span), ...(lane.marks ? { withMarks: true } : {}) });
      const reply = validateProductReply(await client.product<TileReply>(tileRequest, new AbortController().signal), tileRequest).data;
      return keyedTileReply(reply as Parameters<typeof keyedTileReply>[0], keys);
    };
    const made: TileView = new TileView(tileCache(client), { key: keyedLane(lane),
      tags: lane.keys.map(keyId), within: lane.within, lastHour: lane.lastHour, marks: lane.marks === true, ask },
    () => {
      newest.current = { key: tileKey, lines: made.current };
      for (const listener of [...listeners]) listener();
      const now = { lines: made.current !== null, error: made.error };
      if (following && shown && shown.lines === now.lines && Object.is(shown.error, now.error)) return;
      shown = now;
      setTiles({ key: tileKey, lines: made.current, error: made.error });
    });
    view.current = made;
    if (viewport.current) made.setView(viewport.current);
    return () => { made.close(); if (view.current === made) view.current = null; };
  }, [client, tileKey, active, enabled, following, listeners]);
  const readLines = useCallback(() => newest.current?.key === tileKey ? newest.current.lines : null, [tileKey]);
  const summaryOf = useCallback((id: string) => readLines()?.get(id)?.summary, [readLines]);
  const subscribe = useCallback((listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; }, [listeners]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    viewport.current = next; view.current?.setView(next);
  }, []);
  const lines = tiles?.key === tileKey ? tiles.lines : null;
  const summaryByKey = useMemo(() => {
    const out = new Map<string, { summary: ChartSummary; marks: MarkSummary | null }>();
    for (const [id, line] of lines ?? []) out.set(id, { summary: line.summary, marks: line.marks });
    return out;
  }, [lines]);

  const track = gate.track;
  const store: FollowStore | null = useMemo(() => spec && track ? followStore(client, track, clock) : null, [client, clock, !!spec, track]);
  const headQuery = useMemo(() => following && active && enabled ? gate.follow({ product: 'series-state', params: { branchId: spec!.branch.id, family: spec!.family,
    reads: [...new Set(spec!.keys.map(({ tag }) => tag))].map(tag => ({ country: countryFromKey(tag) })) } }) : null,
  [following, active, enabled, ownerKey, gate.follow]);
  const followed = useFollow(store, headQuery, root, NO_PAINT);
  const headFollowed = following && followsHead(followed);
  const content = useContentRevision(client);
  const reads = (spec?.keys ?? []).flatMap(key => {
    const hours = new Set([headFollowed ? null : headHour, hoverHour].filter(hour => hour !== null));
    const spans = spec?.within?.get(keyId(key));
    return [...hours].filter(hour => !spans || spans.some(span => span.firstHour <= hour && hour <= span.lastHour))
      .map(hour => ({ country: countryFromKey(key.tag), part: key.part, hour }));
  });
  const stateRead = useViewRead<typeof reads, KeyedStates>(enabled && reads.length ? JSON.stringify([ownerKey, content]) : null,
    reads, JSON.stringify(reads), active, (at, signal, publish, restart) => loadProduct(client, request('series-state', { reads: at }),
      signal, validateKeyedStates, spec?.source, restart, publish), client);
  const decimals = stateRead.data?.decimals ?? 5;
  const followedEntry: FollowEntry | null = headFollowed ? followed.entry : null;
  const partBits = spec?.partBits ?? 0;
  useEffect(() => {
    if (!followedEntry || !spec) return;
    const lane = keyedLane(spec);
    const stops = spec.keys.map((key, at) => followAtHead(tileHead(lane, ids[at]), () => {
      const answer = followedEntry.answer;
      if (!answer) return { hour: followedEntry.hour, value: undefined };
      const row = followedRow(answer, key, partBits);
      return { hour: followedEntry.hour, value: row < 0 ? null : rowAmount(answer, row, decimals) };
    }));
    return () => stops.forEach(stop => stop());
  }, [followedEntry, ids, decimals, partBits]);

  const [failure, setFailure] = useState<{ owner: typeof owner; error: Failure }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const navigation = useRef<AbortController | null>(null);
  useEffect(() => () => navigation.current?.abort(), [owner, active]);
  const onNavigateHour = useCallback((key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null) => {
    const held = latest.current;
    if (!held || !held.keys.length || !active) return;
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    const range = gate.span(x[0], Math.min(x[1], until ?? x[1]));
    if (!range) return;
    void loadTransientProduct<{ hour: unknown }>(client, request('series-next-change', { ...keyedTileParams(held.keys, held.within && Object.fromEntries(held.within), gate.span),
      key, from: gate.within(from), ...range }), controller.signal).then(reply => {
      if (controller.signal.aborted || alive.current !== owner) return;
      if (reply.hour !== null && !Number.isSafeInteger(reply.hour)) throw new Error('The keyboard reply is malformed.');
      setKeyboard(previous => ({ owner, hour: reply.hour as number | null, revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && alive.current === owner) setFailure({ owner, error: asFailure(error) }); });
  }, [owner, active, gate.span, gate.within]);

  const unrecorded = useMemo(() => unrecordedReason(tiles?.key === tileKey ? tiles.lines : null), [tiles, tileKey]);
  const reached = useMemo(() => reachedKeyLines(spec?.keys ?? [], stateRead.data), [owner, stateRead.data]);
  const followErr = following ? followFailure(followed) : null;
  const retry = () => { followErr?.retry(); setFailure(undefined); view.current?.retry(); stateRead.retry(); };
  const stateError = stateRead.errorKey === JSON.stringify(reads) ? stateRead.error : undefined;
  const tilesError = tiles?.key === tileKey && tiles.error ? asFailure(tiles.error) : undefined;
  const error = useHeldFailure(followErr?.error ?? (failure?.owner === owner ? failure.error : undefined) ?? stateError ?? tilesError,
    stateRead.data !== undefined || summaryByKey.size > 0, retry);
  return { enabled, owner, decimals, lines: reached, states: stateRead.data, summaryByKey, summaryOf, revision: readLines, subscribe, onSummaryView, onNavigateHour,
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined, keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    followed: followedEntry && store ? { store, entry: followedEntry, row: (key: SeriesKey) => followedRow(followedEntry.answer, key, partBits) } : null,
    reason: following ? followRefusal(followed) ?? undefined : undefined,
    unrecorded, error, retry };
}

export function useSeriesParts(client: DashboardClient, gate: Pick<Gate, 'span'>, source: DashboardSource | undefined, branchId: string | undefined,
  family: Readonly<Record<string, unknown>> | null, owner: SeriesOwner | null, span: BoundWindow | null, active: boolean) {
  const key = owner?.key ?? null;
  const windows = useMemo(() => !owner || !span ? [] : owner.spans === null ? [span]
    : owner.spans.flatMap(held => gate.span(Math.max(held.firstHour, span.firstHour), Math.min(held.lastHour, span.lastHour)) ?? []),
  [owner, span?.firstHour, span?.lastHour, gate.span]);
  const requests = useMemo(() => source && branchId && family && key ? windows.map(window => productRequest({ sourceId: source.id, revision: source.revision,
    product: 'series-parts', params: { branchId, family, countries: [countryFromKey(key)], from: window.firstHour, to: window.lastHour } })) : [],
  [source, branchId, family, key, windows]);
  const validate = useCallback((value: unknown) => validateSeriesParts(value, key ?? ''), [key]);
  const read = useProducts<SeriesParts>(client, requests, active && requests.length > 0, validate, source);
  const data = useMemo<HeldParts | undefined>(() => key === null || !owner || !span ? undefined : !requests.length ? { tag: key, parts: [], missing: [], partBits: 1 }
    : read.data && { tag: key, partBits: read.data[0]?.partBits ?? 1,
      parts: heldParts(read.data.flatMap((reply, at) => reply.parts.flatMap(part => {
        const first = Math.max(part.firstHour, windows[at].firstHour), last = Math.min(part.lastHour, windows[at].lastHour);
        return first <= last ? [{ ...part, firstHour: first, lastHour: last }] : [];
      }))),
      missing: read.data.flatMap(reply => reply.missing) },
  [key, owner, span, requests.length, read.data, windows]);
  return { ...read, data };
}
