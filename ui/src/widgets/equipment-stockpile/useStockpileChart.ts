import { useCallback, useMemo, useState } from 'react';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartSummary } from '../../charts/chartSummary';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useViewRead } from '../../dashboard/viewRead';
import type { EquipmentStockpileReply, StockpileCatalogueReply, StockpileHolder, StockpileMeasure } from '../../generated/dashboard';
import { validateStockpileChart, validateStockpileReply } from './stockpileData';
import { holderAt, stockpileView } from './stockpileCatalogue';
import type { StockpileView } from './stockpileCatalogue';

/** A chart line to read: its key in the widget, its group's key and what it draws. */
export interface StockpileLineRequest { key: string; group: string; measure: StockpileMeasure }

const NO_SUMMARIES: ReadonlyMap<string, ChartSummary> = new Map();

/** The chart's lines over the view it shows (`equipment-stockpile-chart`), one read at a time while the
 * view moves, and the stockpile at the hour pointed at (or `head`) over the catalogue, for tooltips and picks. */
export function useStockpileChart(client: DashboardClient, track: BoundTrack, catalogue: StockpileCatalogueReply | undefined,
  holders: readonly StockpileHolder[], included: readonly string[], lines: readonly StockpileLineRequest[], head: number, active: boolean) {
  const scope = JSON.stringify([track.source.id, track.source.revision, track.branch.id, holders, included, lines]);
  const [view, setView] = useState<{ scope: string; view: StockpileView }>();
  const viewport = view?.scope === scope ? view.view : { from: head, to: head, width: 1 };
  const content = useContentRevision(client);
  const display = useViewRead<StockpileView, ReadonlyMap<string, ChartSummary>>(lines.length ? JSON.stringify([scope, content]) : null, viewport,
    JSON.stringify([viewport.from, viewport.to, viewport.width]), active, async (at, signal, publish, restart) => {
      const keyed = (summaries: readonly ChartSummary[]) => new Map(lines.map((line, index) => [line.key, summaries[index]]));
      // While preparation goes on, each later reply replaces the lines shown.
      return keyed(await loadProduct(client, { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile-chart',
        params: { branchId: track.branch.id, holders, included, lines: lines.map(({ group, measure }) => ({ group, measure })), from: at.from, to: at.to, width: at.width } },
      signal, value => validateStockpileChart(value, lines.length), track.source, restart, summaries => publish(keyed(summaries))));
    }, client);
  const [hover, setHover] = useState<number | null>(null);
  const hour = hover ?? head;
  const tag = holderAt(holders, hour);
  const pointed = useProduct<EquipmentStockpileReply>(client, tag ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'equipment-stockpile-at', params: { branchId: track.branch.id, tag, hour } } : null, active, validateStockpileReply, track.source);
  const data = useMemo(() => catalogue && pointed.data ? stockpileView(catalogue, pointed.data, hour) : undefined, [catalogue, pointed.data, hour]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    setView(before => before?.scope === scope && before.view.from === next.from && before.view.to === next.to && before.view.width === next.width ? before : { scope, view: next });
  }, [scope]);
  return { summaries: display.data ?? NO_SUMMARIES, data, error: display.error ?? pointed.error, onSummaryView,
    onHover: (at: number | null) => setHover(at === null ? null : Math.floor(at)),
    retry: () => { display.retry(); pointed.retry(); } };
}
