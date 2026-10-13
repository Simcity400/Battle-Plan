import { useMemo } from 'react';
import DrillParts from '../../charts/drill/DrillParts';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import { fixedNumber, formatAxisPercent } from '../../charts/valueAxis';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import type { StockGroup } from './stockpileData';
import { formatPercent, labelAt } from './stockpileData';
import { stockpilePartAmount, stockpilePartHistory, stockpilePartsAt } from './stockpileDrill';
import type { StockpileMode } from './stockpileDrill';

const percentOfNeed = (raw: bigint | null, needed: bigint | null | undefined) => raw != null && needed != null && needed < 0n
  ? formatPercent(Number(raw) / Number(-needed)) : 'Not known';

/** The same exact operands drive the moment rows and signed history. Daily
 * output is a rate, not a contribution to this recorded stock or balance. */
export default function StockpileDrill({ data, group, mode, percentage, hour, ...chart }: {
  data: EquipmentStockpileReply; group: StockGroup; mode: StockpileMode; percentage: boolean; hour: number;
} & Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label'>) {
  const tokens = useChartTokens(), viewport = useViewportHeight();
  const history = useMemo(() => stockpilePartHistory(data, group, mode, percentage), [data, group, mode, percentage]);
  const value = stockpilePartsAt(data, group, hour, mode);
  const needed = value.parts.find(part => part.id === 'need')?.raw;
  const series = useMemo<BreakdownSeries[]>(() => history.series.map((part, index) => ({ ...part,
    color: tokens.categories[index % tokens.categories.length], amount: at => {
      const sample = stockpilePartsAt(data, group, at, mode);
      const raw = sample.parts[index].raw;
      const amount = stockpilePartAmount(raw, data.decimals);
      return percentage ? `${amount} (${percentOfNeed(raw, sample.parts.find(part => part.id === 'need')?.raw)})` : amount;
    }, missingAt: at => stockpilePartsAt(data, group, at, mode).note ?? 'Not recorded',
  })), [history, data, group, mode, percentage, tokens.categories]);
  return <>
    <DrillParts parts={value.parts.map((part, index) => ({ id: part.id, label: part.label,
      raw: part.raw, value: part.raw == null ? null : fixedNumber(part.raw, data.decimals),
      text: stockpilePartAmount(part.raw, data.decimals),
      shareText: percentage ? percentOfNeed(part.raw, needed) : undefined, color: series[index]?.color }))}
      total={value.total == null ? null : fixedNumber(value.total, data.decimals)} signedShares format={String}
      totalRow={{ label: mode === 'stock' ? 'In stock' : 'Surplus or shortage',
        value: stockpilePartAmount(value.total, data.decimals) }} />
    {value.note && <p className="drill-empty">{value.note}</p>}
    <BreakdownChart {...chart} series={series} selectedHour={hour} tooltipHeight={viewport}
      label={`${labelAt(group, group.kind, hour)} ${mode === 'balance' ? 'stock and need' : 'stock'} over time`}
      axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} keyHours={history.hours}
      formatValue={percentage ? formatAxisPercent : undefined} />
  </>;
}
