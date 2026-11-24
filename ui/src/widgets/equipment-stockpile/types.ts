import type { CountryRef, Domain, HourSpan, StockType, StockVariantName, StockpileCatalogueLogistics, StockpileCatalogueVariant, StockpileChoicePeak } from '../../generated/dashboard';

export type { Domain, HourSpan, StockVariantName };

export interface AmountRun { firstHour: number; lastHour: number; amountRaw: string | null }

export interface StockVariant {
  id: string; token: string; archetype: string | null; names: StockVariantName[];
  stock: AmountRun[]; daily: AmountRun[]; dailyIc: AmountRun[]; unitCostLow: AmountRun[]; unitCostHigh: AmountRun[];
}
export interface TypeLogistics { key: string; balance: AmountRun[]; stock: AmountRun[]; available: AmountRun[] }

export interface EquipmentStockpileReply {
  branchId: string; tag: string; campaignKey: string; decimals: number;
  types: StockType[]; variants: StockVariant[];
  reads: number[]; stockHours: HourSpan[]; lineHours: HourSpan[]; lineReads: number[];
  logistics: TypeLogistics[]; logisticsReads: number[]; balanceHours: HourSpan[]; aiHours: HourSpan[];
  needBoundRaw: string; warnings: string[];
}

export interface StockpileCatalogueReply {
  branchId: string; countries: CountryRef[]; campaignKey: string; decimals: number; types: StockType[];
  variants: StockpileCatalogueVariant[]; logistics: StockpileCatalogueLogistics[];
  reads: number[]; lineReads: number[]; logisticsReads: number[];
  stockHours: HourSpan[]; lineHours: HourSpan[]; balanceHours: HourSpan[]; aiHours: HourSpan[];
  needBoundRaw: string; choices: Record<string, StockpileChoicePeak>; warnings: string[];
}
