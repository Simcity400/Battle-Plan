import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { Range } from '../../charts/view';
import type { HourSpan } from '../../generated/dashboard';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartSummary } from '../../charts/chartSummary';
import type { MarkSummary } from '../../charts/markSummary';
import { reachedLines, validateMeasureStates } from './measureSeriesData';
import type { MeasureStates } from './measureSeriesData';
import { loadProduct, loadTransientProduct } from '../../dashboard/products';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useViewRead } from '../../dashboard/viewRead';
import { useTiles } from '../../dashboard/tiles/useTiles';
import type { CoveredBranch, GatedHour } from '../../dashboard/regionTime';
import type { SeriesGate } from '../../dashboard/keyedSeries';
import { forwardRequest, type DeclaredProduct } from '../../dashboard/request.ts';
import type { ParamsFor } from '../../dashboard/regionTime';
import { withinParams } from '../../dashboard/keyedSeriesData.ts';

export type MeasureFamily = string;

export function useMeasureSeries(client: DashboardClient, gate: SeriesGate, source: DashboardSource | undefined, branch: (CoveredBranch & { id: string }) | undefined,
  family: MeasureFamily, measure: string, tags: readonly string[], headHour: GatedHour | null, hoverHour: GatedHour | null, active: boolean,
  withMarks: boolean, within: ReadonlyMap<string, readonly HourSpan[]> | undefined, lastHour: number, live: boolean,
  readable: (tag: string, hour: number) => boolean = () => true) {
  const branchId = branch?.id;
  const enabled = !!source && !!branchId && tags.length > 0;
  const ownerKey = JSON.stringify([source?.id, source?.revision, branchId, family, measure, tags, within && [...within]]);
  const owner = useMemo(() => ({ client, key: ownerKey }), [client, ownerKey]);
  const alive = useRef(owner); alive.current = owner;
  const [failure, setFailure] = useState<{ owner: typeof owner; error: Failure }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const navigation = useRef<AbortController | null>(null);
  const tiles = useTiles(client, enabled ? { source: source!, branchId: branchId!, family: { kind: family, measure }, tags, within, span: gate.span, lastHour, marks: withMarks } : null, active, live);
  const tileView = tiles.onView;
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    if (view.to < view.from) return;
    tileView(view);
  }, [tileView]);
  const request = <K extends DeclaredProduct>(product: K, params: ParamsFor<K>) => forwardRequest(source!, product,
    { branchId, family: { kind: family, measure }, ...params } as unknown as ParamsFor<K>);
  const content = useContentRevision(client);
  const readLines = tiles.read;
  const lineOf = useCallback((tag: string): ChartSummary | undefined => readLines()?.get(tag)?.summary, [readLines]);
  const marked = useRef<{ lines: unknown; marks: Map<string, MarkSummary | null> } | null>(null);
  const marksOf = useCallback((tag: string): MarkSummary | null => {
    const lines = readLines();
    if (marked.current?.lines !== lines) marked.current = { lines, marks: new Map([...lines ?? []].map(([held, line]) => [held, line.marks])) };
    return marked.current.marks.get(tag) ?? null;
  }, [readLines]);
  const tileDecimals = tiles.lines?.values().next().value?.decimals;
  const hours = [...new Set([headHour, hoverHour].filter(hour => hour !== null))];
  const member = (tag: string, hour: number) => (!within || (within.get(tag) ?? []).some(span => span.firstHour <= hour && hour <= span.lastHour)) && readable(tag, hour);
  const reads = tags.flatMap(tag => hours.filter(hour => member(tag, hour)).map(hour => ({ country: countryFromKey(tag), hour })));
  const stateRead = useViewRead<typeof reads, MeasureStates>(enabled && reads.length ? JSON.stringify([ownerKey, content]) : null,
    reads, JSON.stringify(reads), active, (at, signal, publish, restart) => loadProduct(client, request('series-state', { reads: at }),
      signal, validateMeasureStates, source, restart, publish), client);
  useEffect(() => () => navigation.current?.abort(), [owner, active]);
  const onNavigateHour = useCallback((key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null) => {
    if (!enabled || !active) return;
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    const range = gate.span(x[0], Math.min(x[1], until ?? x[1]));
    if (!range) return;
    void loadTransientProduct<{ hour: unknown }>(client, request('series-next-change', { countries: tags.map(countryFromKey), ...withinParams(tags, within, gate.span), key, from: gate.within(from), ...range }),
      controller.signal).then(reply => {
      if (controller.signal.aborted || alive.current !== owner) return;
      if (reply.hour !== null && (typeof reply.hour !== 'number' || !Number.isFinite(reply.hour))) throw new Error('The keyboard reply is malformed.');
      setKeyboard(previous => ({ owner, hour: reply.hour as number | null, revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && alive.current === owner) setFailure({ owner, error: asFailure(error) }); });
  }, [owner, active, enabled, gate.span, gate.within]);
  const lines = useMemo(() => reachedLines(tags, stateRead.data), [owner, stateRead.data]);
  const retry = () => { setFailure(undefined); tiles.retry(); stateRead.retry(); };
  const stateError = stateRead.errorKey === JSON.stringify(reads) ? stateRead.error : undefined;
  const tilesError = tiles.error ? asFailure(tiles.error) : undefined;
  const shown = tiles.lines && tiles.lines.size > 0 ? true : undefined;
  const error = useHeldFailure((failure?.owner === owner ? failure.error : undefined) ?? stateError ?? tilesError,
    stateRead.data !== undefined || shown !== undefined, retry);
  return { enabled, owner, decimals: stateRead.data?.decimals ?? tileDecimals ?? null, lines, states: stateRead.data,
    summaries: shown, held: tiles.lines, live: tiles.live, lineOf, marksOf, revision: readLines, subscribe: tiles.subscribe, head: tiles.head, onSummaryView, onNavigateHour,
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    hourLoaded: (tag: string, hour: number) => stateRead.data?.states.some(state => countryKey(state.country) === tag && state.hour === hour) ?? false,
    error, retry, notRecorded: tiles.unrecorded };
}
