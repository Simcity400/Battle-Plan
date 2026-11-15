import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Live } from '../../charts/live.ts';
import { mapLive } from '../../charts/live.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import type { DashboardClient, DashboardSource, ProductRequest } from '../client';
import type { HourSpan } from '../../generated/dashboard';
import type { TileReply } from './tile.ts';
import { validateProductReply } from '../client.ts';
import { tileHead } from './head.ts';
import { TileView, tileCache, unrecordedReason } from './view.ts';
import { prepareGate, preparing } from '../prepare.ts';
import type { TiledLine, TileViewport } from './view.ts';

/** A declared series' tiles: the recording, history and family asked, the tags drawn (each over its `within`
 * spans when given) and the history's last hour. */
export interface TileSeries {
  source: Pick<DashboardSource, 'id' | 'revision'>;
  branchId: string;
  family: Record<string, unknown>;
  tags: readonly string[];
  within?: ReadonlyMap<string, readonly HourSpan[]>;
  lastHour: number;
  /** Each tile also carries its drops per bin, drawn as marks. */
  marks?: boolean;
}

/** The tiles' name of a series: recording, revision, history and family. */
const laneKey = (series: TileSeries) => `${series.source.id}|${series.source.revision}|${series.branchId}|${JSON.stringify(series.family)}`;

/** A chart's lines from tiles: `onView` takes the chart's view as it moves, outside React, and asks only the tiles
 * it lacks; nothing renders because the view or the head moved inside the tiles held. Without `live`, the component
 * renders when a view's lines are assembled (a new tile landed or the series changed). With `live` it renders only
 * when the first lines of a series land or its failure changes: later lines (a tile the head's travel asked) are
 * read when painted, `read()`, and `subscribe` hears them, so a chart following the head never commits for a tile.
 * `head(tag)` names a tag's line for `followAtHead`; `unrecorded`, why no line holds a value in the lines landed. */
export function useTiles(client: DashboardClient, series: TileSeries | null, active: boolean, live = false) {
  const key = series && JSON.stringify([series.source.id, series.source.revision, series.branchId, series.family, series.tags,
    series.within && [...series.within], series.lastHour, series.marks === true]);
  const [state, setState] = useState<{ key: string; lines: ReadonlyMap<string, TiledLine> | null; error: unknown }>();
  const view = useRef<TileView | null>(null);
  const viewport = useRef<TileViewport | null>(null);
  const latest = useRef(series); latest.current = series;
  const newest = useRef<{ key: string; lines: ReadonlyMap<string, TiledLine> | null }>(null);
  const listeners = useRef(new Set<() => void>()).current;
  useEffect(() => {
    const lane = latest.current;
    if (!active || !key || !lane) return;
    const tileRequest = (binHours: number, index: number, tags: readonly string[], within: Record<string, HourSpan[]> | undefined): ProductRequest =>
      ({ sourceId: lane.source.id, revision: lane.source.revision, product: 'series-tiles',
        params: { branchId: lane.branchId, family: lane.family, binHours, index, tags, ...(within ? { within } : {}), ...(lane.marks ? { withMarks: true } : {}) } });
    const ask = async (binHours: number, index: number, tags: readonly string[], within: Record<string, HourSpan[]> | undefined) => {
      const request = tileRequest(binHours, index, tags, within);
      return validateProductReply(await client.product<TileReply>(request, new AbortController().signal), request).data;
    };
    const prepared = preparing(client);
    const promote = prepared ? (...asked: Parameters<typeof tileRequest>) => prepareGate(client).promote(tileRequest(...asked)) : undefined;
    let shown: { lines: boolean; error: unknown } | null = null;
    const tiles: TileView = new TileView(tileCache(client), { key: laneKey(lane), tags: lane.tags, within: lane.within, lastHour: lane.lastHour,
      marks: lane.marks === true, ask, prepared, promote }, () => {
      newest.current = { key, lines: tiles.current };
      for (const listener of [...listeners]) listener();
      const now = { lines: tiles.current !== null, error: tiles.error };
      if (live && shown && shown.lines === now.lines && Object.is(shown.error, now.error)) return;
      shown = now;
      setState({ key, lines: tiles.current, error: tiles.error });
    });
    view.current = tiles;
    if (viewport.current) tiles.setView(viewport.current);
    return () => { tiles.close(); if (view.current === tiles) view.current = null; };
  }, [client, key, active, live, listeners]);
  const onView = useCallback((next: TileViewport) => { viewport.current = next; view.current?.setView(next); }, []);
  const retry = useCallback(() => view.current?.retry(), []);
  const read = useCallback(() => newest.current?.key === key ? newest.current.lines : null, [key]);
  const subscribe = useCallback((listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; }, [listeners]);
  const lane = series && laneKey(series);
  const head = useCallback((tag: string) => tileHead(lane ?? '', tag), [lane]);
  const newestLines = useMemo<Live<ReadonlyMap<string, TiledLine> | null>>(() => ({ read, subscribe }), [read, subscribe]);
  const current = state?.key === key ? state : undefined;
  const unrecorded = useMemo(() => unrecordedReason(current?.lines ?? null), [current?.lines]);
  return { onView, lines: current?.lines ?? null, read, subscribe, live: newestLines, head, error: current?.error ?? null, retry, unrecorded };
}

/** The newest lines' summaries by tag, made again only when new lines land: what a chart's layers read as they draw. */
export function liveSummaries(lines: Live<ReadonlyMap<string, TiledLine> | null>): Live<ReadonlyMap<string, ChartSummary> | null> {
  return mapLive(lines, held => held && new Map([...held].map(([tag, line]) => [tag, line.summary])));
}
