import type { EquipmentStockpileReply } from '../../generated/dashboard';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import { runAt } from '../../charts/series.ts';
import { clipToSpans } from '../../charts/series.ts';
import type { Run, Span } from '../../charts/series';
import { datedCountryToken } from '../../dashboard/datedCountry.ts';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import { BREAKDOWN } from './definition.ts';
import { balanceChangeHours, balanceOperandsAt, choiceKey, choiceRow, labelAt, stockAt, stockChoices, stockRuns } from './stockpileData.ts';
import type { Grouping, StockGroup } from './stockpileData';

export type StockpileMode = 'balance' | 'stock';
export interface StockpilePart { id: string; label: string; raw: bigint | null }

export const stockpilePartAmount = (raw: bigint | null, decimals: number) => raw == null ? 'Not recorded'
  : `${raw > 0n ? '+' : ''}${formatFixed(raw, decimals, decimals)}`;

/** Names are independent of numeric ownership; design witnesses remain identity. */
export function stockpileNumbers(reply: EquipmentStockpileReply): EquipmentStockpileReply {
  return { ...reply, warnings: [], types: reply.types.map(type => ({ ...type, name: null })),
    variants: reply.variants.map(variant => ({ ...variant, names: variant.names.map(name => ({ ...name, name: null })) })) };
}

/** Merge only the chosen holder's numeric coverage; read dates keep their supplying hours. */
export function stockpileCountryData(replies: readonly EquipmentStockpileReply[], spans: ReadonlyMap<string, readonly Span[]>, tag: string): EquipmentStockpileReply | undefined {
  const first = replies[0];
  if (!first) return undefined;
  const variants = new Map<string, EquipmentStockpileReply['variants'][number]>();
  const logistics = new Map<string, EquipmentStockpileReply['logistics'][number]>();
  const ordered = <T extends Span>(runs: T[]) => runs.sort((a, b) => a.firstHour - b.firstHour);
  for (const reply of replies) {
    const coverage = spans.get(reply.tag) ?? [];
    for (const variant of reply.variants) {
      const previous = variants.get(variant.id);
      const stock = clipToSpans(variant.stock, coverage), weekly = clipToSpans(variant.weekly, coverage);
      if (previous) { previous.stock.push(...stock); previous.weekly.push(...weekly); }
      else variants.set(variant.id, { ...variant, stock, weekly });
    }
    for (const item of reply.logistics) {
      const previous = logistics.get(item.key);
      const balance = clipToSpans(item.balance, coverage), stock = clipToSpans(item.stock, coverage), available = clipToSpans(item.available, coverage);
      if (previous) { previous.balance.push(...balance); previous.stock.push(...stock); previous.available.push(...available); }
      else logistics.set(item.key, { ...item, balance, stock, available });
    }
  }
  const coverage = (key: 'stockHours' | 'lineHours' | 'balanceHours' | 'aiHours') => ordered(replies.flatMap(reply => clipToSpans(reply[key], spans.get(reply.tag) ?? [])));
  const reads = (key: 'reads' | 'lineReads' | 'logisticsReads') => [...new Set(replies.flatMap(reply => reply[key].filter((hour, index, hours) => (spans.get(reply.tag) ?? []).some(span => hour <= span.lastHour
    && (hours[index + 1] === undefined || hours[index + 1] > span.firstHour)))))].sort((a, b) => a - b);
  return { ...first, tag, types: [...new Map(replies.flatMap(reply => reply.types.map(type => [type.key, type] as const))).values()],
    variants: [...variants.values()].map(variant => ({ ...variant, stock: ordered(variant.stock), weekly: ordered(variant.weekly) })),
    logistics: [...logistics.values()].map(item => ({ ...item, stock: ordered(item.stock), balance: ordered(item.balance), available: ordered(item.available) })),
    stockHours: coverage('stockHours'), lineHours: coverage('lineHours'), balanceHours: coverage('balanceHours'), aiHours: coverage('aiHours'),
    reads: reads('reads'), lineReads: reads('lineReads'), logisticsReads: reads('logisticsReads'), warnings: [...new Set(replies.flatMap(reply => reply.warnings))] };
}

/** The available operand is the contribution, not a later stockpile read. The
 * wide residual retains signed i64 wrapping without inventing equipment. */
export function stockpilePartsAt(reply: EquipmentStockpileReply, group: StockGroup, hour: number, mode: StockpileMode) {
  if (mode === 'stock') {
    const stock = stockAt(reply, group, hour);
    return { total: stock, note: group.kind === 'variant' ? 'Need is available by equipment type.' : null,
      parts: [{ id: 'stock', label: 'In stock', raw: stock }] satisfies StockpilePart[] };
  }
  const value = balanceOperandsAt(reply, group, hour);
  const available = value?.available ?? null, need = value?.need ?? null;
  return { total: value?.balance ?? null, note: group.kind === 'variant'
    ? 'Need is available by equipment type.' : value?.needNote ? 'Need unavailable.' : null, parts: [
    { id: 'available', label: 'Available', raw: available },
    { id: 'need', label: 'Needed', raw: need == null ? null : -need },
    { id: 'other', label: 'Other', raw: value && available != null && need != null ? value.balance - available + need : null },
  ] satisfies StockpilePart[] };
}

/** One boundary walk supplies the identical signed part set for the history. */
export function stockpilePartHistory(reply: EquipmentStockpileReply, group: StockGroup, mode: StockpileMode, percentage = false) {
  const spans = mode === 'balance' ? reply.balanceHours : reply.stockHours;
  const boundaries = new Set<number>(spans.flatMap(span => [span.firstHour, span.lastHour + 1]));
  const changes = mode === 'balance' ? balanceChangeHours(reply, group)
    : stockRuns(reply, group).flatMap(run => [run.firstHour, run.lastHour + 1]);
  for (const hour of changes) boundaries.add(hour);
  const hours = [...boundaries].sort((a, b) => a - b);
  const template = stockpilePartsAt(reply, group, spans[0]?.firstHour ?? 0, mode).parts;
  const series = template.map(part => ({ id: part.id, label: part.label, runs: [] as Run[] }));
  for (let index = 0; index + 1 < hours.length; index++) {
    const firstHour = hours[index], lastHour = hours[index + 1] - 1;
    const span = runAt(spans, firstHour);
    if (!span) continue;
    const values = stockpilePartsAt(reply, group, firstHour, mode);
    const needed = values.parts.find(part => part.id === 'need')?.raw;
    if (percentage && (needed == null || needed >= 0n)) continue;
    values.parts.forEach((part, partIndex) => {
      if (part.raw == null) return;
      const runs = series[partIndex].runs, previous = runs.at(-1);
      const value = percentage ? Number(part.raw) / Number(-needed!) * 100 : fixedNumber(part.raw, reply.decimals);
      const joined = previous && previous.lastHour + 1 === firstHour && firstHour !== span.firstHour;
      if (joined && previous.value === value) previous.lastHour = lastHour;
      else runs.push({ firstHour, lastHour, value, breakBefore: !joined });
    });
  }
  return { series, hours };
}

/** Reuse the existing campaign/design-authenticated semantic choice contract. */
export function stockpileDrillGroup(groups: readonly StockGroup[], stored: string): StockGroup | undefined {
  const key = contentKey(stored);
  return stockChoices(groups, 'equipment').lines(key) ? groups.find(group => group.key === choiceRow(key)) : undefined;
}

export function pinStockpileDrill(widget: WidgetSpec, track: Pick<BoundTrack, 'id' | 'branch'>, tag: string,
  group: StockGroup, grouping: Grouping, mode: StockpileMode, hour: number, follow: boolean, country: DatedCountrySelection): WidgetSpec | null {
  const key = choiceKey(group, hour);
  if (key == null) return null;
  const pinned = pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [tag] } },
    { ...BREAKDOWN, label: labelAt(group, grouping, hour) }, track.id);
  pinned.options = { ...pinned.options, grouping, partGroup: setToken(key), partMode: mode, stockpileCountry: datedCountryToken(country) };
  pinned.time = follow ? { mode: 'follow' } : { mode: 'pin', windows: { [track.id]: {
    firstOffset: Math.floor(hour) - track.branch.firstHour, lastOffset: Math.floor(hour) - track.branch.firstHour } } };
  return pinned;
}
