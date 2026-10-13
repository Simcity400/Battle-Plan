import { useMemo } from 'react';
import DrillParts from '../../charts/drill/DrillParts';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import { dayLabel, drawLine } from '../../charts/draw';
import { exactPartShare } from '../../charts/race/countryPartsDrillData';
import { runAt } from '../../charts/series';
import { fixedNumber, formatAmount, formatRate, formatSigned } from '../../charts/valueAxis';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import type { StockGroup } from './stockpileData';
import { balanceMode, balanceOperandsAt, dailyAt, dailyIcAt, stockAt, stockRuns, variantName } from './stockpileData';
import { stockpilePartHistory } from './stockpileDrill';

/** The dash Needed is drawn with, as the Fuel widget draws capacity. */
const NEED_DASH = [5, 4];

/** One type's stock at an hour, model by model: each model's share of the type (one
 * unit, so the bars compare), the game's logistics equation (available − needed =
 * surplus) where it computes one, what each model's production lines make a day, the
 * IC they put in and their cost per unit, and the stock over time by model with what
 * is needed dashed. A cost per unit is a production line's; a model held but not in
 * production has none recorded. */
export default function StockpileDrill({ data, group, hour, ...chart }: {
  data: EquipmentStockpileReply; group: StockGroup; hour: number;
} & Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label'>) {
  const tokens = useChartTokens(), viewport = useViewportHeight();
  const amount = (raw: bigint) => formatAmount(raw, data.decimals, 0);
  const total = stockAt(data, group, hour);
  const models = useMemo(() => group.variants.map((variant, index) => ({ variant, index,
    label: variantName(variant, hour), stock: stockAt(data, { variants: [variant] }, hour) })), [data, group, hour]);
  const colours = useMemo(() => new Map(models.map(model => [model.variant.id, tokens.categories[model.index % tokens.categories.length]])), [models, tokens.categories]);
  const parts = models.filter(model => model.stock !== 0n).map(model => ({ id: model.variant.id, label: model.label,
    value: model.stock == null ? null : fixedNumber(model.stock, data.decimals), raw: model.stock,
    text: model.stock == null ? 'Not recorded' : amount(model.stock), shareText: exactPartShare(model.stock, total == null ? null : Number(total)),
    color: colours.get(model.variant.id) }));
  const player = balanceMode(data, hour) === 'player';
  const logistics = player ? balanceOperandsAt(data, group, hour) : null;
  // Models in the order the bars list them: most held first, unrecorded last.
  const byStock = [...models].sort((a, b) => (a.stock == null ? 1 : 0) - (b.stock == null ? 1 : 0)
    || (a.stock === b.stock ? a.label.localeCompare(b.label) : (a.stock ?? 0n) > (b.stock ?? 0n) ? -1 : 1));
  const production = byStock.flatMap(model => {
    const output = dailyAt(data, { variants: [model.variant] }, hour);
    const ic = dailyIcAt(data, { variants: [model.variant] }, hour);
    const low = runAt(model.variant.unitCostLow, hour), high = runAt(model.variant.unitCostHigh, hour);
    if (model.stock === 0n && (output == null || output === 0n) && !low) return [];
    // No cost run where the lines were read: the model has no production line then.
    const cost = runAt(data.lineHours, hour) == null ? 'Not recorded' : !low ? 'No production line'
      : low.amountRaw == null || high?.amountRaw == null ? 'Not known'
        : low.amountRaw === high.amountRaw ? formatRate(low.amountRaw, data.decimals)
          : `${formatRate(low.amountRaw, data.decimals)}–${formatRate(high.amountRaw, data.decimals)}`;
    return [{ id: model.variant.id, label: model.label, output: output == null ? 'Not recorded' : formatRate(output, data.decimals),
      ic: ic == null ? 'Not recorded' : formatRate(ic, data.decimals), cost }];
  });
  const series = useMemo<BreakdownSeries[]>(() => models.map(model => ({ id: model.variant.id, label: model.label,
    color: colours.get(model.variant.id)!, runs: stockRuns(data, { variants: [model.variant] }),
    labelAt: at => variantName(model.variant, at),
    amount: at => { const value = stockAt(data, { variants: [model.variant] }, at); return value == null ? 'Not recorded' : formatAmount(value, data.decimals, 0); } })),
  [models, data, colours]);
  const needRuns = useMemo(() => player ? stockpilePartHistory(data, group, 'balance').series.find(part => part.id === 'need')?.runs
    .map(run => ({ ...run, value: -run.value })) ?? [] : [], [data, group, player]);
  return <div className="stockpile-models">
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
    <BreakdownChart {...chart} series={series} selectedHour={hour} tooltipHeight={viewport}
      label={`${group.label} in stock over time, by model`} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      overlay={(ctx, frame, chartTokens) => {
        chart.overlay?.(ctx, frame, chartTokens);
        if (needRuns.length) drawLine(ctx, frame, chartTokens, needRuns, chartTokens.text, { dash: NEED_DASH, held: true });
      }} />
  </div>;
}
