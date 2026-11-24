import { equipmentOf, fixedText, priceLevel } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import type { MarketGrant, MarketItem, MarketLendLeaseReply, MarketOffer, MarketTerms } from '../../generated/dashboard';

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
export const levelText = (level: number | null) => priceLevel(level);

export interface MarketDetailLine { readonly label: string; readonly value: string; readonly marked?: boolean; readonly country?: string }
export interface MarketDetailSection { readonly title: string; readonly lines: readonly MarketDetailLine[]; readonly empty: string }
export interface MarketDetail { readonly missing: string | null; readonly sections: readonly MarketDetailSection[] }

const NAME_MISSING = 'Equipment name unavailable';

function grantLines(grants: readonly MarketGrant[]): MarketDetailLine[] {
  return grants.map(grant => ({
    label: grant.archetype ?? 'Archetype not named',
    value: `${cic(BigInt(grant.cicRaw))} · ${grant.trigger !== null ? 'Conditional eligibility' : grant.targets.length ? grant.targets.join(', ') : 'No eligible countries'}`,
  }));
}

export function marketDetail(reply: MarketLendLeaseReply, selected: string): MarketDetail {
  const equipment = equipmentOf(reply);
  const name = (item: { variant: string; name: string | null }) => item.name ?? equipment(item.variant).name ?? NAME_MISSING;
  const [kind, first, second] = selected.split(':');
  if (kind === 'offer') {
    if (reply.offers === null) return { missing: 'Market inventory was not recorded at this date.', sections: [] };
    const offer = reply.offers.find(row => String(row.country) === first);
    if (!offer) return { missing: 'This seller lists nothing on the market at this date.', sections: [] };
    const levels = new Map(offer.prices.map(level => [level.variant, level.level]));
    return { missing: null, sections: [
      { title: 'Listed inventory', empty: 'Nothing listed.', lines: offer.inventory.map(item => ({
        label: name(item), value: `${units(BigInt(item.amountRaw))} · ${levels.has(item.variant) ? levelText(levels.get(item.variant)!) : 'Price level not recorded'}`, marked: item.variant === second })) },
      { title: 'Subsidies', empty: 'No subsidies.', lines: [...grantLines(offer.subsidies), { label: 'Subsidy balance', value: cic(BigInt(offer.subsidyTotalRaw)) }] },
      { title: 'Market access', empty: '', lines: [{ label: 'Automatically send market access', value: offer.autoSendMarketAccess ? 'Yes' : 'No' }] },
    ] };
  }
  if (reply.contracts === null) return { missing: 'Market contract terms were not recorded at this date.', sections: [] };
  const terms = reply.contracts.find(row => String(row.request) === first && row.id === second);
  if (!terms) return { missing: 'This request or contract is not on the market at this date.', sections: [] };
  const price = new Map(terms.prices.map(item => [item.variant, BigInt(item.amountRaw)]));
  const level = new Map(terms.levels.map(item => [item.variant, item.level]));
  return { missing: null, sections: [
    { title: 'Terms', empty: '', lines: [
      { label: 'Seller', value: terms.seller ?? 'Country not recorded', ...(terms.seller !== null ? { country: terms.seller } : {}) },
      { label: 'Buyer', value: terms.buyer ?? 'Country not recorded', ...(terms.buyer !== null ? { country: terms.buyer } : {}) },
      { label: 'Kind', value: terms.request ? 'Request' : 'Contract' },
      { label: 'Factory assignment', value: String(terms.speed) },
    ] },
    { title: 'Equipment', empty: 'None', lines: terms.equipment.map(item => {
      const agreed = price.get(item.variant);
      const asked = level.get(item.variant);
      return { label: name(item), value: [units(BigInt(item.amountRaw)), ...(agreed !== undefined ? [cic(agreed)] : []), ...(asked !== undefined ? [levelText(asked)] : [])].join(' · ') };
    }) },
    { title: 'Subsidies', empty: 'No subsidies.', lines: grantLines(terms.subsidies) },
  ] };
}
