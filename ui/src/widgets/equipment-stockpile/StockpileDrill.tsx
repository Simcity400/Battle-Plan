import type { Gate } from '../../dashboard/regionGate';
import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillParts from '../../charts/drill/DrillParts';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import { dayLabel } from '../../charts/draw';
import { drawLineSummary, extentOfSummary } from '../../charts/chartSummary';
import { exactPartShare } from '../../charts/race/countryPartsDrillData';
import { runAt } from '../../charts/series';
import { fixedNumber, formatAmount, formatRate, formatSigned } from '../../charts/valueAxis';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { catalogueColours } from '../../charts/palette';
import type { EquipmentStockpileReply, StockpileCatalogueReply } from './types';
import type { StockGroup } from './stockpileData';
import { balanceMode, balanceOperandsAt, dailyAt, dailyIcAt, stockAt, variantName } from './stockpileData';
import { stockpileGroupKey } from './stockpileCatalogue';
import { useStockpileChart } from './useStockpileChart';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { StockpileHolder } from '../../generated/dashboard';

const NEED_DASH = [5, 4];
const NEED = '';

export default function StockpileDrill({ data, catalogue, holders, included, group, hour, client, gate, track, ...chart }: {
  data: EquipmentStockpileReply; catalogue: StockpileCatalogueReply | undefined; holders: readonly StockpileHolder[]; included: readonly string[];
  group: StockGroup; hour: number; client: DashboardClient; gate: Pick<Gate, 'span' | 'recorded'>; track: BoundTrack;
} & Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label' | 'data'>) {
  const tokens = useChartTokens(), viewport = useViewportHeight();
  const amount = (raw: bigint) => formatAmount(raw, data.decimals, 0);
  const total = stockAt(data, group, hour);
  const models = useMemo(() => group.variants.map((variant, index) => ({ variant, index,
    label: variantName(variant, hour), stock: stockAt(data, { variants: [variant] }, hour) })), [data, group, hour]);
  const colours = useMemo(() => catalogueColours(group.variants.map(variant => variant.id), tokens.surface), [group.variants, tokens.surface]);
  const parts = models.filter(model => model.stock !== 0n).map(model => ({ id: model.variant.id, label: model.label,
    value: model.stock == null ? null : fixedNumber(model.stock, data.decimals), raw: model.stock,
    text: model.stock == null ? 'Not recorded' : amount(model.stock), shareText: exactPartShare(model.stock, total == null ? null : Number(total)),
    color: colours.get(model.variant.id) }));
  const player = balanceMode(data, hour) === 'player';
  const lines = useMemo(() => [...group.variants.map(variant => ({ key: variant.id, group: JSON.stringify(['variant', variant.id]), measure: 'stock' as const })),
    { key: group.key, group: stockpileGroupKey(group), measure: 'stock' as const },
    ...(player ? [{ key: NEED, group: stockpileGroupKey(group), measure: 'need' as const }] : [])], [group, player]);
  const incoming = useStockpileChart(client, gate, track, catalogue, holders, included, lines, hour, chart.active);
  const logistics = player ? balanceOperandsAt(data, group, hour) : null;
  const byStock = [...models].sort((a, b) => (a.stock == null ? 1 : 0) - (b.stock == null ? 1 : 0)
    || (a.stock === b.stock ? a.label.localeCompare(b.label) : (a.stock ?? 0n) > (b.stock ?? 0n) ? -1 : 1));
  const production = byStock.flatMap(model => {
    const output = dailyAt(data, { variants: [model.variant] }, hour);
    const ic = dailyIcAt(data, { variants: [model.variant] }, hour);
    const low = runAt(model.variant.unitCostLow, hour), high = runAt(model.variant.unitCostHigh, hour);
    if (model.stock === 0n && (output == null || output === 0n) && !low) return [];
    const cost = runAt(data.lineHours, hour) == null ? 'Not recorded' : !low ? 'No production line'
      : low.amountRaw == null || high?.amountRaw == null ? 'Not known'
        : low.amountRaw === high.amountRaw ? formatRate(low.amountRaw, data.decimals)
          : `${formatRate(low.amountRaw, data.decimals)}–${formatRate(high.amountRaw, data.decimals)}`;
    return [{ id: model.variant.id, label: model.label, output: output == null ? 'Not recorded' : formatRate(output, data.decimals),
      ic: ic == null ? 'Not recorded' : formatRate(ic, data.decimals), cost }];
  });
  const series = useMemo<BreakdownSeries[]>(() => models.map(model => {
    const amountAt = (at: number) => {
      const reached = incoming.data?.prepared?.hour === at ? incoming.data : undefined;
      const variant = reached?.variants.find(row => row.id === model.variant.id);
      const value = reached && variant ? stockAt(reached, { variants: [variant] }, at) : null;
      return value == null ? 'Not recorded' : formatAmount(value, data.decimals, 0);
    };
    return { id: model.variant.id, label: model.label, color: colours.get(model.variant.id)!,
      runs: incoming.summaries.get(model.variant.id)?.runs ?? [],
      labelAt: at => variantName((incoming.data?.variants ?? data.variants).find(row => row.id === model.variant.id) ?? model.variant, at),
      amount: amountAt, missingAt: amountAt };
  }), [models, data, colours, incoming.data, incoming.summaries]);
  const totalSummary = incoming.summaries.get(group.key);
  const needed = incoming.summaries.get(NEED);
  const summaries = { parts: series.map(part => incoming.summaries.get(part.id) ?? { from: hour, to: hour, width: 1, buckets: [], runs: [] }),
    total: totalSummary?.buckets.length && totalSummary.buckets.every(bucket => bucket !== null && !bucket.hasBreak) ? totalSummary : undefined };
  const neededExtent = useCallback((x: readonly [number, number]) => needed ? extentOfSummary(needed, x[0], x[1]) : null, [needed]);
  return <div className="stockpile-models">
    {incoming.error && <WidgetState kind="error" what="the stock history" failure={incoming.error} onRetry={incoming.retry} />}
    {logistics && <p className="stockpile-facts">Available <b>{logistics.available == null ? 'not known' : amount(logistics.available)}</b>
      {' − '}needed <b>{logistics.need == null ? 'not known' : amount(logistics.need)}</b>
      {' = '}<b className={logistics.balance < 0n ? 'is-negative' : logistics.balance > 0n ? 'is-positive' : undefined}>{logistics.balance < 0n ? 'shortage ' : 'surplus '}{formatSigned(logistics.balance, data.decimals, 0)}</b>
      <span> (the game’s Logistics figures)</span></p>}
    <p className="drill-empty">In stock on {dayLabel(hour)}, by model</p>
    {parts.length ? <DrillParts parts={parts} format={value => String(value)} total={total == null ? null : fixedNumber(total, data.decimals)}
      showShares={total != null && total > 0n} positiveBars
      totalRow={{ label: 'In stock', value: total == null ? 'Not recorded' : amount(total) }} />
      : <p className="drill-empty">None in stock: {total == null ? 'not recorded' : amount(total)}.</p>}
    {production.length > 0 && <div className="stockpile-production" role="table" aria-label="Production by model">
      <div role="row" className="stockpile-production-head"><span role="columnheader">Production</span><span role="columnheader">Output / day</span>
        <span role="columnheader">IC / day</span><span role="columnheader">IC per unit</span></div>
      {production.map(row => <div key={row.id} role="row" className="stockpile-production-row">
        <span role="cell">{row.label}</span><b role="cell">{row.output}</b><b role="cell">{row.ic}</b><b role="cell" className={/[a-z]/i.test(row.cost) ? 'is-muted' : undefined}>{row.cost}</b>
      </div>)}
    </div>}
    <p className="drill-empty">Stock over time by model{player ? ' · dashed: needed' : ''}</p>
    <BreakdownChart {...chart} {...{ summaries }} series={series} selectedHour={hour} tooltipHeight={viewport}
      onSummaryView={incoming.onSummaryView} onHover={incoming.onHover}
      label={`${group.label} in stock over time, by model`} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} overlayExtent={player ? neededExtent : undefined}
      overlay={(ctx, frame, chartTokens) => {
        chart.overlay?.(ctx, frame, chartTokens);
        if (needed) drawLineSummary(ctx, frame, chartTokens, needed, chartTokens.text, true, null, { dash: NEED_DASH });
      }} />
  </div>;
}
