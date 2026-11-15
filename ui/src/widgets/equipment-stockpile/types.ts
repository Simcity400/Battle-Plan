/** The view of an hour the stockpile widget's data functions read (a UI shape: the catalogue's designs and types with
 * the hour's values as runs); the family's replies are the contract's. */
import type { ChartSummary, Domain, HourSpan, StockType, StockVariantName, StockpileCatalogueLogistics, StockpileCatalogueVariant, StockpileChoicePeak } from '../../generated/dashboard';

export type { ChartSummary, Domain, HourSpan, StockVariantName };

/** A value held over consecutive hours: an exact decimal of the fixed point; null where read but not known. */
export interface AmountRun { firstHour: number; lastHour: number; amountRaw: string | null }

/** A design in the view of an hour: its values as runs over the hours they hold. */
export interface StockVariant {
  id: string; token: string; archetype: string | null; names: StockVariantName[];
  stock: AmountRun[]; daily: AmountRun[]; dailyIc: AmountRun[]; unitCostLow: AmountRun[]; unitCostHigh: AmountRun[];
}
export interface TypeLogistics { key: string; balance: AmountRun[]; stock: AmountRun[]; available: AmountRun[] }

/** The widget's view of one holder at an hour: the catalogue's designs and types with the hour's values. */
export interface EquipmentStockpileReply {
  branchId: string; tag: string; campaignKey: string; decimals: number;
  types: StockType[]; variants: StockVariant[];
  reads: number[]; stockHours: HourSpan[]; lineHours: HourSpan[]; lineReads: number[];
  logistics: TypeLogistics[]; logisticsReads: number[]; balanceHours: HourSpan[]; aiHours: HourSpan[];
  needBoundRaw: string; warnings: string[];
}

export interface StockpileCatalogueReply {
  branchId: string; tags: string[]; campaignKey: string; decimals: number; types: StockType[];
  variants: StockpileCatalogueVariant[]; logistics: StockpileCatalogueLogistics[];
  reads: number[]; lineReads: number[]; logisticsReads: number[];
  stockHours: HourSpan[]; lineHours: HourSpan[]; balanceHours: HourSpan[]; aiHours: HourSpan[];
  needBoundRaw: string; choices: Record<string, StockpileChoicePeak>; warnings: string[];
}
