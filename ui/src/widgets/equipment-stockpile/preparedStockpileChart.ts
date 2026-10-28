import type { ChartSummary, ChartBucket } from '../../charts/chartSummary.ts';
import { clipToSpans, runAt } from '../../charts/series.ts';
import type { Span, Run } from '../../charts/series';
import type { EquipmentStockpileReply, AmountRun } from '../../generated/dashboard';
import { balanceAt, chartRuns, shareOf, stockAt, shareModeOf } from './stockpileData.ts';
import type { Grouping, StockGroup } from './stockpileData';
import { stockpilePartHistory } from './stockpileDrill.ts';

export interface StockpileOperand {
  field: string; id: string; key: string; values: ChartSummary; presence: ChartSummary;
  samples: readonly (readonly (string | null)[])[];
  rowSamples: readonly (readonly (number | null)[])[];
}
export interface StockpilePrepared {
  from: number; to: number;
  stockRecorded: boolean; linesRecorded: boolean; balanceRecorded: boolean; aiRecorded: boolean;
  operands: readonly StockpileOperand[]; coverage: readonly ChartSummary[]; nonzeroLogistics: readonly string[];
  groups?: readonly { key: string; operands: readonly StockpileOperand[] }[];
  choices: Readonly<Record<string, { raw: string; stockDrawable: boolean; shareDrawable: boolean }>>;
  designs: Readonly<Record<string, readonly string[]>>;
}
export type PreparedStockpileReply = EquipmentStockpileReply & { prepared?: StockpilePrepared };
export interface StockpileView { from: number; to: number; width: number }

export const stockpileGroupKey = (group: StockGroup) => JSON.stringify([group.kind, group.kind === 'variant' ? group.variants[0]?.id : group.key]);

/** Integer group sums are sampled before float conversion, so correlated model
 * extrema never become an invented type peak. */
export function stockpileAmountSummary(reply: PreparedStockpileReply, group: StockGroup, field: 'stock' | 'need'): ChartSummary | undefined {
  const row = reply.prepared?.groups?.find(row => row.key === stockpileGroupKey(group))?.operands.find(row => row.field === field);
  if (!row) return undefined;
  const scale = 10 ** reply.decimals;
  return { ...row.values, buckets: row.values.buckets.map(bucket => bucket && ({ ...bucket,
    firstValue: bucket.firstValue / scale, lastValue: bucket.lastValue / scale, minValue: bucket.minValue / scale, maxValue: bucket.maxValue / scale })),
    ...(row.values.runs ? { runs: row.values.runs.map(run => ({ ...run, value: run.value / scale })) } : {}) };
}

/** The group's share of need as the backend paired it from same-hour balance and need. */
function groupShareSummary(reply: PreparedStockpileReply, group: StockGroup): ChartSummary | undefined {
  const share = reply.prepared?.groups?.find(row => row.key === stockpileGroupKey(group))?.operands.find(row => row.field === 'share');
  return share && { ...share.values, runs: undefined };
}

export function groupIn(reply: EquipmentStockpileReply, group: StockGroup): StockGroup {
  const variants = new Map(reply.variants.map(variant => [variant.id, variant]));
  return { ...group, variants: group.variants.flatMap(variant => variants.get(variant.id) ?? []) };
}

/** Date reads carry whole canonical intervals. Merge each interval once without
 * joining native coverage breaks; then evaluate only the requested known range. */
export function mergeStockpileReads(replies: readonly PreparedStockpileReply[]): PreparedStockpileReply {
  const base = replies.at(-1)!;
  const spans = <T extends Span>(runs: readonly T[]) => {
    const byFirst = new Map<number, T>();
    for (const run of runs) {
      const previous = byFirst.get(run.firstHour);
      if (!previous || previous.lastHour < run.lastHour) byFirst.set(run.firstHour, run);
    }
    return [...byFirst.values()].sort((a, b) => a.firstHour - b.firstHour);
  };
  const amounts = (pick: (reply: PreparedStockpileReply) => readonly AmountRun[]) => spans(replies.flatMap(pick));
  return { ...base,
    variants: base.variants.map(variant => {
      const rows = replies.flatMap(reply => reply.variants.find(row => row.id === variant.id) ?? []);
      const names = new Map(rows.flatMap(row => row.names.map(name => [name.firstHour, name] as const)));
      return { ...variant, names: [...names.values()].sort((a, b) => a.firstHour - b.firstHour),
        stock: spans(rows.flatMap(row => row.stock)), daily: spans(rows.flatMap(row => row.daily)), dailyIc: spans(rows.flatMap(row => row.dailyIc)),
        unitCostLow: spans(rows.flatMap(row => row.unitCostLow)), unitCostHigh: spans(rows.flatMap(row => row.unitCostHigh)) };
    }),
    logistics: base.logistics.map(item => ({ ...item,
      stock: amounts(reply => reply.logistics.find(row => row.key === item.key)?.stock ?? []),
      balance: amounts(reply => reply.logistics.find(row => row.key === item.key)?.balance ?? []),
      available: amounts(reply => reply.logistics.find(row => row.key === item.key)?.available ?? []) })),
    stockHours: spans(replies.flatMap(reply => reply.stockHours)), lineHours: spans(replies.flatMap(reply => reply.lineHours)),
    balanceHours: spans(replies.flatMap(reply => reply.balanceHours)), aiHours: spans(replies.flatMap(reply => reply.aiHours)),
    reads: [...new Set(replies.flatMap(reply => reply.reads))].sort((a, b) => a - b),
    lineReads: [...new Set(replies.flatMap(reply => reply.lineReads))].sort((a, b) => a - b),
    logisticsReads: [...new Set(replies.flatMap(reply => reply.logisticsReads))].sort((a, b) => a - b),
  };
}

export function stockpileWindow(reply: PreparedStockpileReply, from: number, to: number): PreparedStockpileReply {
  const within = [{ firstHour: from, lastHour: to }];
  return { ...reply,
    stockHours: clipToSpans(reply.stockHours, within), lineHours: clipToSpans(reply.lineHours, within),
    balanceHours: clipToSpans(reply.balanceHours, within), aiHours: clipToSpans(reply.aiHours, within),
    variants: reply.variants.map(row => ({ ...row, stock: clipToSpans(row.stock, within), daily: clipToSpans(row.daily, within),
      dailyIc: clipToSpans(row.dailyIc, within), unitCostLow: clipToSpans(row.unitCostLow, within), unitCostHigh: clipToSpans(row.unitCostHigh, within) })),
    logistics: reply.logistics.map(row => ({ ...row, stock: clipToSpans(row.stock, within), balance: clipToSpans(row.balance, within), available: clipToSpans(row.available, within) })),
  };
}

/** Each dense sample sums exact decimal operands before the legacy ratio/scale
 * conversion. Per-series extrema are the approved crowded-chart geometry. */
function denseSample(reply: PreparedStockpileReply, bucket: number, sample: number): PreparedStockpileReply {
  const prepared = reply.prepared!;
  const operand = (field: string, key: string) => prepared.operands.find(row => row.field === field && row.key === key);
  const amount = (field: string, key: string): AmountRun[] => {
    const row = operand(field, key), presence = row?.rowSamples[bucket]?.[sample];
    return presence == null || presence === 0 ? [] : [{ firstHour: 0, lastHour: 0,
      amountRaw: presence < 0 ? null : row?.samples[bucket]?.[sample] ?? null }];
  };
  const covered = (field: number): Span[] => prepared.coverage[field]?.buckets[bucket] ? [{ firstHour: 0, lastHour: 0 }] : [];
  return { ...reply, stockHours: covered(0), lineHours: covered(1), balanceHours: covered(2), aiHours: covered(3),
    variants: reply.variants.map(row => ({ ...row, stock: amount('stock', row.id), daily: amount('daily', row.id), dailyIc: amount('daily-ic', row.id),
      unitCostLow: amount('cost-low', row.id), unitCostHigh: amount('cost-high', row.id) })),
    logistics: reply.logistics.map(row => ({ ...row, stock: amount('logistics-stock', row.key), balance: amount('balance', row.key), available: amount('available', row.key) })),
  };
}

export function stockpileSummaries(reply: PreparedStockpileReply, groups: readonly StockGroup[], grouping: Grouping,
  view: StockpileView, exact: boolean): ReadonlyMap<string, ChartSummary> {
  if (exact) return new Map(groups.map(group => [group.key, { ...view, buckets: [], runs: chartRuns(reply, groupIn(reply, group), grouping) }]));
  const prepared = reply.prepared;
  if (!prepared) return new Map();
  const share = shareModeOf(reply, grouping);
  const coverage = prepared.coverage[share ? 2 : 0];
  if (!coverage) return new Map();
  const samples = coverage.buckets.map((_, index) => [0, 1, 2, 3].map(sample => denseSample(reply, index, sample)));
  return new Map(groups.map(group => {
    const aggregate = share ? groupShareSummary(reply, group) : stockpileAmountSummary(reply, group, 'stock');
    if (aggregate) return [group.key, aggregate];
    const buckets = coverage.buckets.map((covered, index): ChartBucket | null => {
      if (!covered) return null;
      const values = samples[index].map(sample => {
        const selected = groupIn(sample, group);
        const ratio = shareOf(balanceAt(sample, selected, 0));
        const stock = stockAt(sample, selected, 0);
        return share ? ratio == null ? null : ratio * 100 : stock == null ? null : Number(stock) / 10 ** reply.decimals;
      });
      const rows = prepared.operands.filter(row => share ? group.types.includes(row.key) && ['balance', 'logistics-stock', 'available'].includes(row.field)
        : row.field === 'stock' && group.variants.some(variant => variant.id === row.key));
      const extrema = rows.flatMap(row => row.values.buckets[index] ?? []);
      const hours = [covered.firstHour, covered.lastHour,
        extrema.reduce((hour, row) => Math.max(hour, row.minHour), covered.firstHour),
        extrema.reduce((hour, row) => Math.max(hour, row.maxHour), covered.firstHour)];
      const known = values.flatMap((value, index) => value == null ? [] : [{ value, hour: hours[index] }]);
      if (!known.length) return null;
      const min = known.reduce((a, b) => a.value <= b.value ? a : b), max = known.reduce((a, b) => a.value >= b.value ? a : b);
      return { firstHour: known[0].hour, lastHour: known[1]?.hour ?? known[0].hour, firstValue: known[0].value, lastValue: values[1] ?? known.at(-1)!.value,
        lastValueHour: Math.max(covered.firstHour, ...extrema.map(row => row.lastValueHour)), minHour: min.hour, minValue: min.value,
        maxHour: max.hour, maxValue: max.value, firstBreak: covered.firstBreak || extrema.some(row => row.firstBreak),
        hasBreak: covered.hasBreak || known.length !== 4 || rows.some(row => row.values.buckets[index]?.hasBreak || (row.presence.buckets[index]?.minValue ?? 0) < 0) };
    });
    return [group.key, { ...view, buckets }];
  }));
}

/** Used by edge extension to compare the exact legacy plotted values. */
const supplyingRuns = (reply: EquipmentStockpileReply, group: StockGroup, grouping: Grouping, measure: 'chart' | 'need') => measure === 'need'
  ? stockpilePartHistory(reply, groupIn(reply, group), 'balance').series.find(row => row.id === 'need')?.runs.map(run => ({ ...run, value: -run.value })) ?? []
  : chartRuns(reply, groupIn(reply, group), grouping);
export const plottedAt = (reply: EquipmentStockpileReply, group: StockGroup, grouping: Grouping, hour: number, measure: 'chart' | 'need' = 'chart'): number | null =>
  runAt(supplyingRuns(reply, group, grouping, measure), hour)?.value ?? null;

export function stockpileFields(reply: EquipmentStockpileReply, group: StockGroup, share: boolean): string[] {
  if (!share) return [...new Set([...group.variants, ...(group.kind === 'variant' ? [] : reply.variants.filter(row => row.archetype === null))])].map(row => 'stock:' + row.id);
  const keys = new Set([...group.types, ...(group.kind === 'domain' && group.key !== 'unresolved'
    ? reply.logistics.filter(row => row.key.startsWith(':') && (!group.scope.length || group.scope.includes(row.key))).map(row => row.key) : [])]);
  return ((reply as PreparedStockpileReply).prepared?.operands ?? []).filter(row => keys.has(row.key) && ['balance', 'logistics-stock', 'available'].includes(row.field)).map(row => row.field + ':' + row.id);
}

/** Extend only the two supplying intervals touched by a fine viewport. Each
 * point read replaces the last one: equal ratios never retain dense history. */
export async function exactStockpileRuns(base: PreparedStockpileReply, groups: readonly StockGroup[], grouping: Grouping,
  view: StockpileView, bounds: readonly [number, number], read: (hour: number) => Promise<PreparedStockpileReply>,
  next: (hour: number, forward: boolean, fields: readonly string[]) => Promise<number | null>, measure: 'chart' | 'need' = 'chart'): Promise<ReadonlyMap<string, readonly Run[]>> {
  const window = stockpileWindow(base, Math.floor(view.from), Math.floor(view.to));
  const share = measure === 'need' || shareModeOf(base, grouping), coverage = share ? 2 : 0;
  const result = new Map<string, Run[]>();
  for (const group of groups) {
    const fields = stockpileFields(base, group, share);
    const runs = supplyingRuns(window, group, grouping, measure).map(run => ({ ...run }));
    result.set(group.key, runs);
    for (const forward of [false, true]) {
      const edge = Math.floor(forward ? view.to : view.from);
      const target = runAt(runs, edge);
      if (!target) continue;
      let point = base, hour = edge;
      let boundary = edge, broken = true;
      for (;;) {
        const covered = runAt(point.prepared?.coverage[coverage]?.runs ?? [], hour);
        if (!covered) break;
        const intervals = (point.prepared?.operands ?? []).filter(row => fields.includes(row.field + ':' + row.id))
          .flatMap(row => runAt(row.presence.runs ?? [], hour) ?? []);
        boundary = forward ? Math.min(bounds[1], covered.lastHour, ...intervals.map(run => run.lastHour))
          : Math.max(bounds[0], covered.firstHour, ...intervals.map(run => run.firstHour));
        if (boundary === bounds[forward ? 1 : 0]) { broken = true; break; }
        const change = await next(boundary, forward, fields);
        if (change === null || (forward ? change <= boundary : change >= boundary)) { broken = covered.firstHour === boundary; break; }
        const reached = forward ? change : boundary - 1;
        const incoming = await read(reached);
        const incomingCoverage = runAt(incoming.prepared?.coverage[coverage]?.runs ?? [], reached);
        const value = plottedAt(stockpileWindow(incoming, reached, reached), group, grouping, reached, measure);
        if (value !== target.value || incomingCoverage?.firstHour !== covered.firstHour) {
          broken = value === null || incomingCoverage?.firstHour !== covered.firstHour;
          break;
        }
        point = incoming; hour = reached;
      }
      if (forward) target.lastHour = boundary;
      else { target.firstHour = boundary; target.breakBefore = broken; }
    }
  }
  return result;
}
