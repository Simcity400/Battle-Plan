import { formatAmount } from '../../charts/valueAxis.ts';
import { equipmentOf, fixedText, priceLevel } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import type { MarketItem, MarketOffer, MarketTerms } from '../../generated/dashboard';

export type MarketTab = 'contracts' | 'requests' | 'inventory';

export interface MarketItemAt { variant: string; name: string | null; icon: string | null; missing: string | null; amount: bigint }

export interface ContractRow {
  key: string;
  terms: MarketTerms;
  seller: string | null;
  buyer: string | null;
  equipment: MarketItemAt[];
  prices: MarketItemAt[];
  levels: { variant: string; level: number }[];
}

export interface OfferRow {
  key: string;
  offer: MarketOffer;
  seller: string | null;
  variant: string;
  name: string | null;
  icon: string | null;
  missing: string | null;
  amount: bigint;
  level: number | null;
}

const items = (equipment: ReturnType<typeof equipmentOf>, list: readonly MarketItem[]): MarketItemAt[] =>
  list.map(item => ({ ...equipment(item.variant), name: item.name ?? equipment(item.variant).name, variant: item.variant, amount: BigInt(item.amountRaw) }));

export type MarketMatch = (tag: string | null, hour: number) => boolean;

export function contractRows(reply: MarketAt, tab: 'contracts' | 'requests', matches: MarketMatch): ContractRow[] {
  const rows: ContractRow[] = [];
  const equipment = equipmentOf(reply);
  for (const terms of reply.contracts ?? []) {
    if (terms.request !== (tab === 'requests')) continue;
    const { seller, buyer } = terms;
    if (!matches(seller, reply.hour) && !matches(buyer, reply.hour)) continue;
    rows.push({ key: terms.key, terms, seller, buyer, equipment: items(equipment, terms.equipment), prices: items(equipment, terms.prices),
      levels: terms.levels.map(level => ({ variant: level.variant, level: level.level })) });
  }
  return rows;
}

export function offerRows(reply: MarketAt, matches: MarketMatch): OfferRow[] {
  const rows: OfferRow[] = [];
  const equipment = equipmentOf(reply);
  for (const offer of reply.offers ?? []) {
    if (!matches(offer.tag, reply.hour)) continue;
    const levels = new Map(offer.prices.map(level => [level.variant, level.level]));
    for (const item of offer.inventory) {
      const drawn = equipment(item.variant);
      rows.push({ key: `${offer.country}:${item.variant}`, offer, seller: offer.tag, variant: item.variant, name: item.name ?? drawn.name, icon: drawn.icon,
        missing: drawn.missing, amount: BigInt(item.amountRaw), level: levels.get(item.variant) ?? null });
    }
  }
  return rows;
}

export const units = (raw: bigint | null) => raw === null ? '—' : fixedText(raw);
export const cic = (raw: bigint | null) => raw === null ? '—' : `${fixedText(raw)} CIC`;
export const unitsLabel = (raw: bigint | null) => raw === null ? '—' : formatAmount(raw, 5);
export const cicLabel = (raw: bigint | null) => raw === null ? '—' : `${formatAmount(raw, 5)} CIC`;
export const levelText = (level: number | null) => priceLevel(level);
export const firstWithMore = (list: readonly unknown[]) => list.length > 1 ? `+${list.length - 1}` : null;

export const collectedText = (value: number) => cicLabel(BigInt(Math.trunc(value)));
