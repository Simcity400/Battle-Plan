import type { EquipmentStockpileReply, StockpileCatalogueReply, StockpileChoicePeak, StockpileHolder, StockVariant, TypeLogistics } from '../../generated/dashboard';
import type { Span } from '../../charts/series';
import type { StockGroup } from './stockpileData';

/** What the widget reads of the whole branch besides an hour's values (`equipment-stockpile-catalogue`). */
export interface StockpilePrepared {
  /** The hour the values are as of: the one the shown reply answers, which a followed read can hold
   * while the next hour's loads. */
  hour: number;
  /** The holders' stock and logistics reads within their hours, for stepping through them. */
  reads: readonly number[]; logisticsReads: readonly number[];
  stockRecorded: boolean; linesRecorded: boolean; balanceRecorded: boolean; aiRecorded: boolean;
  nonzeroLogistics: readonly string[];
  choices: Readonly<Record<string, StockpileChoicePeak | undefined>>;
  designs: Readonly<Record<string, readonly string[]>>;
}
export type PreparedStockpileReply = EquipmentStockpileReply & { prepared?: StockpilePrepared };
export interface StockpileView { from: number; to: number; width: number }

/** The hour a reply's values are as of, else `hour`. */
export const servedHour = (reply: EquipmentStockpileReply | undefined, hour: number) =>
  (reply as PreparedStockpileReply | undefined)?.prepared?.hour ?? hour;

/** A group's key in the catalogue's choices and the chart's lines: `[kind, key]`, a variant by its handle. */
export const stockpileGroupKey = (group: StockGroup) => JSON.stringify([group.kind, group.kind === 'variant' ? group.variants[0]?.id : group.key]);

/** The group with its variants as `reply` holds them. */
export function groupIn(reply: EquipmentStockpileReply, group: StockGroup): StockGroup {
  const variants = new Map(reply.variants.map(variant => [variant.id, variant]));
  return { ...group, variants: group.variants.flatMap(variant => variants.get(variant.id) ?? []) };
}

/** The holders a request reads: each tag, only within its hours where they are named. */
export const stockpileHolders = (spans: ReadonlyMap<string, readonly Span[]>): StockpileHolder[] =>
  [...spans].map(([tag, within]) => ({ tag, within: within.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour })) }));

/** The holder whose hours hold `hour`: the first without named hours, else the one they hold it. */
export const holderAt = (holders: readonly StockpileHolder[], hour: number) =>
  holders.find(holder => !holder.within || holder.within.some(span => span.firstHour <= hour && hour <= span.lastHour))?.tag ?? null;

/** The widget's reply at an hour: the catalogue's variants, types and peaks with the hour's values and
 * the reads that supply them (`equipment-stockpile-at`), none where the hour's reply is not in yet. A
 * variant or type the hour names that an older catalogue does not list keeps its own entry. */
export function stockpileView(catalogue: StockpileCatalogueReply, at: EquipmentStockpileReply | undefined, hour: number, tag = at?.tag ?? catalogue.tags[0] ?? ''): PreparedStockpileReply {
  const values = new Map((at?.variants ?? []).map(variant => [variant.id, variant]));
  const listed = new Set(catalogue.variants.map(variant => variant.id));
  const variants: StockVariant[] = catalogue.variants.map(variant => {
    const held = values.get(variant.id);
    return { id: variant.id, token: variant.token, archetype: variant.archetype, names: variant.names, stock: held?.stock ?? [],
      daily: held?.daily ?? [], dailyIc: held?.dailyIc ?? [], unitCostLow: held?.unitCostLow ?? [], unitCostHigh: held?.unitCostHigh ?? [] };
  });
  for (const variant of at?.variants ?? []) if (!listed.has(variant.id)) variants.push(variant);
  const records = new Map((at?.logistics ?? []).map(item => [item.key, item]));
  const keys = new Set(catalogue.logistics.map(item => item.key));
  const logistics: TypeLogistics[] = catalogue.logistics.map(item => records.get(item.key) ?? { key: item.key, balance: [], stock: [], available: [] });
  for (const item of at?.logistics ?? []) if (!keys.has(item.key)) logistics.push(item);
  const typeKeys = new Set(catalogue.types.map(type => type.key));
  const prefix = at as (EquipmentStockpileReply & { coverage?: unknown; pending?: unknown; approximate?: unknown }) | undefined;
  const view: PreparedStockpileReply = {
    branchId: catalogue.branchId, tag, campaignKey: catalogue.campaignKey, decimals: catalogue.decimals,
    types: [...catalogue.types, ...(at?.types ?? []).filter(type => !typeKeys.has(type.key))], variants, logistics,
    reads: at?.reads ?? [], lineReads: at?.lineReads ?? [], logisticsReads: at?.logisticsReads ?? [],
    stockHours: at?.stockHours ?? [], lineHours: at?.lineHours ?? [], balanceHours: at?.balanceHours ?? [], aiHours: at?.aiHours ?? [],
    needBoundRaw: catalogue.needBoundRaw, warnings: [...new Set([...catalogue.warnings, ...(at?.warnings ?? [])])],
    prepared: { hour, reads: catalogue.reads, logisticsReads: catalogue.logisticsReads, stockRecorded: catalogue.stockHours.length > 0, linesRecorded: catalogue.lineHours.length > 0,
      balanceRecorded: catalogue.balanceHours.length > 0, aiRecorded: catalogue.aiHours.length > 0,
      nonzeroLogistics: catalogue.logistics.filter(item => item.nonzero).map(item => item.key), choices: catalogue.choices,
      designs: Object.fromEntries(catalogue.variants.map(variant => [variant.id, variant.designs])) },
  };
  // The hour's prepared prefix, which says where its values are still approximate.
  if (prefix && 'pending' in prefix) Object.assign(view, { coverage: prefix.coverage, pending: prefix.pending,
    ...(prefix.approximate === undefined ? {} : { approximate: prefix.approximate }) });
  return view;
}
