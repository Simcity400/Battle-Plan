import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useState } from 'react';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartSummary } from '../../charts/chartSummary';
import type { DashboardClient } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useViewRead } from '../../dashboard/viewRead';
import { atReply, validateStockpileAt } from './stockpileAt.ts';
import { holderAt, stockpileView } from './stockpileCatalogue';
import type { StockpileView } from './stockpileCatalogue';
import { validateStockpileChart } from './stockpileData';
import type { StockpileCatalogueReply } from './types';
import type { StockpileAtReply, StockpileHolder, StockpileMeasure } from '../../generated/dashboard';

/** A chart line to read: its key in the widget, its group's key and what it draws. */
export interface StockpileLineRequest { key: string; group: string; measure: StockpileMeasure }

const NO_SUMMARIES: ReadonlyMap<string, ChartSummary> = new Map();

/** The view a chart's view is read as: its length grown to a power of two of hours, starting on a quarter of that,
 * and its width scaled to keep the pixels. A view growing or sliding with the playhead is asked again only when its
 * length doubles or it slides a quarter, never a frame at a time; every hour of `view` is inside it. */
export function askedView(view: StockpileView): StockpileView {
  const length = Math.max(1, Math.floor(view.to) - Math.floor(view.from) + 1);
  const span = 2 ** Math.ceil(Math.log2(length));
  const step = Math.max(1, span / 4);
  const from = Math.floor(Math.floor(view.from) / step) * step;
  const to = from + span + step - 1;
  return { from, to, width: Math.max(1, Math.ceil(Math.max(1, view.width) * (to - from + 1) / length)) };
}

/** The chart's lines over the view it shows (`equipment-stockpile-chart`), read as `askedView` widens it, and the
 * stockpile at the hour pointed at, over the catalogue, for tooltips and picks: asked only while one is pointed at. */
export function useStockpileChart(client: DashboardClient, track: BoundTrack, catalogue: StockpileCatalogueReply | undefined,
  holders: readonly StockpileHolder[], included: readonly string[], lines: readonly StockpileLineRequest[], head: number, active: boolean) {
  const scope = JSON.stringify([track.source.id, track.source.revision, track.branch.id, holders, included, lines]);
  const [view, setView] = useState<{ scope: string; view: StockpileView }>();
  const asked = askedView(view?.scope === scope ? view.view : { from: track.branch.firstHour, to: Math.max(track.branch.firstHour, head), width: 1 });
  const content = useContentRevision(client);
  const display = useViewRead<StockpileView, ReadonlyMap<string, ChartSummary>>(lines.length ? JSON.stringify([scope, content]) : null, asked,
    JSON.stringify([asked.from, asked.to, asked.width]), active, async (at, signal, publish, restart) => {
      const keyed = (summaries: readonly ChartSummary[]) => new Map(lines.map((line, index) => [line.key, summaries[index]]));
      return keyed(await loadProduct(client, { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile-chart',
        params: { branchId: track.branch.id, holders, included, lines: lines.map(({ group, measure }) => ({ group, measure })), from: at.from, to: at.to, width: at.width } },
      signal, value => validateStockpileChart(value, lines.length), track.source, restart, summaries => publish(keyed(summaries))));
    }, client);
  const [hover, setHover] = useState<number | null>(null);
  const tag = hover === null ? null : holderAt(holders, hover);
  const pointed = useProduct<StockpileAtReply>(client, tag && hover !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'equipment-stockpile-at', params: { branchId: track.branch.id, country: countryFromKey(tag), hour: hover } } : null, active, validateStockpileAt, track.source);
  const data = useMemo(() => catalogue && pointed.data && hover !== null
    ? stockpileView(catalogue, atReply(pointed.data, repliedCells(pointed.data).read(), hover, track.branch.id), hover) : undefined,
  [catalogue, pointed.data, hover, track.branch.id]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    setView(before => {
      const held = before?.scope === scope ? askedView(before.view) : null;
      const now = askedView(next);
      return held && held.from === now.from && held.to === now.to && held.width === now.width ? before : { scope, view: next };
    });
  }, [scope]);
  return { summaries: display.data ?? NO_SUMMARIES, data, error: display.error ?? pointed.error, onSummaryView,
    onHover: (at: number | null) => setHover(at === null ? null : Math.floor(at)),
    retry: () => { display.retry(); pointed.retry(); } };
}
