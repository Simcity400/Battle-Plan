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

/** A declared series' family kind. */
export type MeasureFamily = string;

/** A declared series' chart of `tags` over the chart's view, from tiles, which a view following the head asks one
 * per 256 bins of travel (hourly views too, their bins an hour), with its drops as marks per bin when `withMarks`;
 * and the tags' exact values at the hours a reader reaches (the head where it is not followed, and the hover). Never
 * an hour of the view's moving edge: while playing that is a new read, and a render, every frame. The lines are
 * read when painted (`lineOf`, `marksOf`, `live`; `revision` names the lines held; `held`, the first lines, names them) and heard with `subscribe`: `live` (a view
 * following the head), a tile landing renders nothing.
 * `within` draws each tag only over its spans. */
export function useMeasureSeries(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined,
  family: MeasureFamily, measure: string, tags: readonly string[], headHour: number | null, hoverHour: number | null, active: boolean,
  withMarks: boolean, within: ReadonlyMap<string, readonly HourSpan[]> | undefined, lastHour: number, live: boolean) {
  const enabled = !!source && !!branchId && tags.length > 0;
  const ownerKey = JSON.stringify([source?.id, source?.revision, branchId, family, measure, tags, within && [...within]]);
  const owner = useMemo(() => ({ client, key: ownerKey }), [client, ownerKey]);
  const alive = useRef(owner); alive.current = owner;
  const [failure, setFailure] = useState<{ owner: typeof owner; error: Failure }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const navigation = useRef<AbortController | null>(null);
  const tiles = useTiles(client, enabled ? { source: source!, branchId: branchId!, family: { kind: family, measure }, tags, within, lastHour, marks: withMarks } : null, active, live);
  const tileView = tiles.onView;
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    if (view.to < view.from) return;
    tileView(view);
  }, [tileView]);
  const request = (product: string, params: Record<string, unknown>) => ({ sourceId: source!.id, revision: source!.revision, product,
    params: { branchId, family: { kind: family, measure }, ...params } });
  const content = useContentRevision(client);
  const readLines = tiles.read;
  const lineOf = useCallback((tag: string): ChartSummary | undefined => readLines()?.get(tag)?.summary, [readLines]);
  // The marks of the lines held, made again only when new lines land.
  const marked = useRef<{ lines: unknown; marks: Map<string, MarkSummary | null> } | null>(null);
  const marksOf = useCallback((tag: string): MarkSummary | null => {
    const lines = readLines();
    if (marked.current?.lines !== lines) marked.current = { lines, marks: new Map([...lines ?? []].map(([held, line]) => [held, line.marks])) };
    return marked.current.marks.get(tag) ?? null;
  }, [readLines]);
  // The places of the values: a state read's, else the tiles'.
  const tileDecimals = tiles.lines?.values().next().value?.decimals;
  // A followed head is pushed, not read: no read names it.
  const reads = tags.flatMap(tag => [...new Set([...(headHour === null ? [] : [Math.floor(headHour)]), ...(hoverHour === null ? [] : [Math.floor(hoverHour)])])]
    .map(hour => ({ tag, hour })));
  // One state read at a time while playback and the hover move: the last states stay shown while the newest is read.
  const stateRead = useViewRead<typeof reads, MeasureStates>(enabled && reads.length ? JSON.stringify([ownerKey, content]) : null,
    reads, JSON.stringify(reads), active, (at, signal, publish, restart) => loadProduct(client, request('series-state', { reads: at }),
      signal, validateMeasureStates, source, restart, publish), client);
  useEffect(() => () => navigation.current?.abort(), [owner, active]);
  const onNavigateHour = useCallback((key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null) => {
    if (!enabled || !active) return;
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    void loadTransientProduct<{ hour: unknown }>(client, request('series-next-change', { tags, key, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) }),
      controller.signal).then(reply => {
      if (controller.signal.aborted || alive.current !== owner) return;
      if (reply.hour !== null && (typeof reply.hour !== 'number' || !Number.isFinite(reply.hour))) throw new Error('The keyboard reply is malformed.');
      setKeyboard(previous => ({ owner, hour: reply.hour as number | null, revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && alive.current === owner) setFailure({ owner, error: asFailure(error) }); });
  }, [owner, active, enabled]);
  const lines = useMemo(() => reachedLines(tags, stateRead.data), [owner, stateRead.data]);
  const retry = () => { setFailure(undefined); tiles.retry(); stateRead.retry(); };
  // A read's failure belongs to the reads it asked for; newer ones are still loading.
  const stateError = stateRead.errorKey === JSON.stringify(reads) ? stateRead.error : undefined;
  const tilesError = tiles.error ? asFailure(tiles.error) : undefined;
  const shown = tiles.lines && tiles.lines.size > 0 ? true : undefined;
  const error = useHeldFailure((failure?.owner === owner ? failure.error : undefined) ?? stateError ?? tilesError,
    stateRead.data !== undefined || shown !== undefined, retry);
  return { enabled, owner, decimals: stateRead.data?.decimals ?? tileDecimals ?? null, lines, states: stateRead.data,
    summaries: shown, held: tiles.lines, live: tiles.live, lineOf, marksOf, revision: readLines, subscribe: tiles.subscribe, head: tiles.head, onSummaryView, onNavigateHour,
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    hourLoaded: (tag: string, hour: number) => stateRead.data?.states.some(state => state.tag === tag && state.hour === hour) ?? false,
    error, retry, notRecorded: tiles.unrecorded };
}
