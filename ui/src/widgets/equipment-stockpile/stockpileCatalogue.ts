import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { EquipmentStockpileReply, StockpileCatalogueReply, StockVariant, TypeLogistics } from './types';
import type { Span } from '../../charts/series';
import type { StockGroup } from './stockpileData';
import type { StockpileChoicePeak, StockpileHolder } from '../../generated/dashboard';

export interface StockpilePrepared {
  hour: number;
  reads: readonly number[]; logisticsReads: readonly number[];
  stockRecorded: boolean; linesRecorded: boolean; balanceRecorded: boolean; aiRecorded: boolean;
  nonzeroLogistics: readonly string[];
  choices: Readonly<Record<string, StockpileChoicePeak | undefined>>;
  designs: Readonly<Record<string, readonly string[]>>;
}
export type PreparedStockpileReply = EquipmentStockpileReply & { prepared?: StockpilePrepared };
export interface StockpileView { from: number; to: number; width: number }

export const stockpileGroupKey = (group: StockGroup) => JSON.stringify([group.kind, group.kind === 'variant' ? group.variants[0]?.id : group.key]);

export const stockpileHolders = (spans: ReadonlyMap<string, readonly Span[]>): StockpileHolder[] =>
  [...spans].map(([tag, within]) => ({ country: countryFromKey(tag), within: within.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour })) }));

export const holderAt = (holders: readonly StockpileHolder[], hour: number): string | null => {
  const found = holders.find(holder => !holder.within || holder.within.some(span => span.firstHour <= hour && hour <= span.lastHour));
  return found ? countryKey(found.country) : null;
};

export function stockpileView(catalogue: StockpileCatalogueReply, at: EquipmentStockpileReply | undefined, hour: number, tag = at?.tag ?? (catalogue.countries[0] === undefined ? '' : countryKey(catalogue.countries[0]))): PreparedStockpileReply {
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
  return view;
}
