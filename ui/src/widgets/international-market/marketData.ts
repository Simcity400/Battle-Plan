import type { ContractHistory, MarketHistoryReply, OfferHistory } from '../../generated/dashboard';
import { fixedText, runAt, valueAt } from '../../dashboard/economyHistory.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { FIXED5, holderTagAt, poolAt, variantAt } from '../../dashboard/marketHistory.ts';
import { priceLevel } from '../trade-flows/marketData.ts';

export type MarketTab = 'contracts' | 'requests' | 'inventory';

export interface MarketItemAt { variant: string; name: string | null; amount: bigint }

/** A request or accepted contract at the hour shown. */
export interface ContractRow {
  key: string;
  contract: ContractHistory;
  seller: string | null;
  buyer: string | null;
  equipment: MarketItemAt[];
  prices: MarketItemAt[];
  levels: { variant: string; level: number }[];
  /** The daily purchase contract's convoys assigned and total, collected CIC and
   * completed deliveries; null where not recorded at its read. */
  assigned: bigint | null; convoys: bigint | null; collected: bigint | null; deliveries: bigint | null;
}

/** One listed variant of a seller's inventory. */
export interface OfferRow {
  key: string;
  offer: OfferHistory;
  seller: string | null;
  variant: string;
  name: string | null;
  amount: bigint;
  level: number | null;
}

const items = (reply: MarketHistoryReply, contract: ContractHistory, pool: number, hour: number): MarketItemAt[] =>
  poolAt(contract.items, pool, hour).map(item => ({ ...item, name: variantAt(reply, item.variant, hour) }));

/** The requests or contracts standing at `hour` that the matcher keeps (either party), and how
 * many others it cannot rule out: a party with no tag then may be a country it keeps. */
export function contractRows(reply: MarketHistoryReply, hour: number, matches: (tag: string, hour: number) => boolean): { rows: ContractRow[]; unattributed: number } {
  const rows: ContractRow[] = [];
  let unattributed = 0;
  for (const contract of reply.contracts) {
    // Its speed is read every hour it stands; its buyer only while the buyer has an identity.
    if (!runAt(contract.speed, hour)) continue;
    const seller = holderTagAt(reply, contract.seller, hour), buyer = holderTagAt(reply, valueAt(contract.buyer, hour), hour);
    if (!(seller !== null && matches(seller, hour) || buyer !== null && matches(buyer, hour))) {
      if (seller === null || buyer === null) unattributed++;
      continue;
    }
    const at = (index: number) => { const value = valueAt(contract.purchase[index] ?? [], hour); return value === null ? null : BigInt(value); };
    rows.push({ key: `${contract.seller}:${contract.id}`, contract, seller, buyer,
      equipment: items(reply, contract, 0, hour), prices: items(reply, contract, 1, hour),
      levels: poolAt(contract.levels, 2, hour).map(item => ({ variant: item.variant, level: Number(item.amount) })),
      assigned: at(0), convoys: at(1), collected: at(2), deliveries: at(3) });
  }
  return { rows, unattributed };
}

/** Every listed variant of the matching sellers at `hour`, in their native order. */
export function offerRows(reply: MarketHistoryReply, hour: number, matches: (tag: string, hour: number) => boolean): OfferRow[] {
  const rows: OfferRow[] = [];
  for (const offer of reply.offers) {
    const seller = holderTagAt(reply, offer.seller, hour);
    if (seller === null || !matches(seller, hour)) continue;
    const levels = new Map(poolAt(offer.levels, 2, hour).map(item => [item.variant, Number(item.amount)]));
    for (const item of poolAt(offer.inventory, 0, hour)) rows.push({ key: `${offer.seller}:${item.variant}`, offer, seller, variant: item.variant,
      name: variantAt(reply, item.variant, hour), amount: item.amount, level: levels.get(item.variant) ?? null });
  }
  return rows;
}

export const units = (raw: bigint | null) => raw === null ? '—' : fixedText(raw, FIXED5);
export const cic = (raw: bigint | null) => raw === null ? '—' : `${fixedText(raw, FIXED5)} CIC`;
/** Table cells round to two decimals; tooltips keep `units` and `cic` exact. */
export const unitsLabel = (raw: bigint | null) => raw === null ? '—' : formatAmount(raw, 5);
export const cicLabel = (raw: bigint | null) => raw === null ? '—' : `${formatAmount(raw, 5)} CIC`;
export const levelText = (level: number | null) => level === null ? '—' : priceLevel(level);
export const firstWithMore = (items: readonly MarketItemAt[]) => items.length > 1 ? `+${items.length - 1}` : null;
