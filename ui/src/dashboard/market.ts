import type { LendLeaseHistoryReply, MarketLendLeaseReply } from '../generated/dashboard';
import { object } from './validation.ts';

export type MarketAt = MarketLendLeaseReply & { hour: number };

export const MARKET = 'market-lendlease';
export const LEASE_HISTORY = 'lend-lease-history';
export const FIXED5 = 100000n;
export const FUEL_QUANTITY = 32768n;
export const SHARE = 1000n;

const raw = (value: unknown) => typeof value === 'string' && /^-?\d+$/.test(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const integer = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value);
const list = (value: unknown, valid: (row: Record<string, unknown>) => boolean) => Array.isArray(value) && value.every(row => object(row) && valid(row));
const items = (value: unknown) => list(value, row => typeof row.variant === 'string' && text(row.name) && raw(row.amountRaw));
const levels = (value: unknown) => list(value, row => typeof row.variant === 'string' && text(row.name) && integer(row.level));
const grants = (value: unknown) => list(value, row => raw(row.cicRaw) && text(row.archetype) && text(row.trigger) && Array.isArray(row.targets) && row.targets.every(tag => typeof tag === 'string'));
const named = (value: Record<string, unknown>) => Array.isArray(value.variants) && value.variants.every(object) && Array.isArray(value.icons) && value.icons.every(object)
  && Array.isArray(value.warnings) && value.warnings.every(warning => typeof warning === 'string');
const optionalMissing = (value: unknown) => value === undefined || value === null || typeof value === 'string';
const leaseAmount = (value: unknown) => object(value) && (value.valueRaw === null ? typeof value.missing === 'string' : raw(value.valueRaw) && (value.missing === undefined || value.missing === null));
const leaseHour = (value: unknown) => object(value) && (value.value === null ? typeof value.missing === 'string' : integer(value.value) && (value.missing === undefined || value.missing === null));
const leaseItems = (value: unknown) => object(value) && (value.items === null ? typeof value.missing === 'string' : items(value.items) && (value.missing === undefined || value.missing === null));
export const isLease = (row: Record<string, unknown>) => typeof row.key === 'string' && typeof row.id === 'string' && text(row.giver) && text(row.receiver) && optionalMissing(row.receiverMissing)
  && leaseAmount(row.revision) && leaseHour(row.activeSince) && leaseHour(row.lastDelivery)
  && ['monthly', 'percentage', 'once', 'delivered', 'sunk'].every(key => leaseItems(row[key]))
  && ['fuelDailyRaw', 'fuelPercentageRaw', 'lastFuelDeliveredRaw'].every(key => leaseAmount(row[key]));

export function validateMarketReply(value: unknown): MarketLendLeaseReply {
  const nullable = (v: unknown, valid: (row: Record<string, unknown>) => boolean) => v === null || list(v, valid);
  if (!object(value) || typeof value.branchId !== 'string' || !named(value) || !text(value.purchasesMissing) || !integer(value.unattributed)
    || !nullable(value.offers, row => integer(row.country) && text(row.tag) && items(row.inventory) && levels(row.prices) && grants(row.subsidies) && raw(row.subsidyTotalRaw) && typeof row.autoSendMarketAccess === 'boolean')
    || !nullable(value.contracts, row => typeof row.key === 'string' && typeof row.id === 'string' && text(row.seller) && text(row.buyer) && typeof row.request === 'boolean'
      && integer(row.speed) && items(row.equipment) && items(row.prices) && levels(row.levels) && grants(row.subsidies) && typeof row.purchase === 'boolean')
    || !nullable(value.agreements, isLease)) throw new Error('Invalid market and lend-lease reply');
  return value as unknown as MarketLendLeaseReply;
}

export function validateLeaseHistory(value: unknown): LendLeaseHistoryReply {
  if (!object(value) || typeof value.branchId !== 'string' || !named(value)
    || !list(value.agreements, row => object(row.terms) && isLease(row.terms) && leaseAmount(row.fuelSentRaw) && leaseAmount(row.fuelSunkRaw) && integer(row.firstHour) && integer(row.lastHour) && integer(row.endedBy)))
    throw new Error('Invalid lend-lease history reply');
  return value as unknown as LendLeaseHistoryReply;
}

export function fixedText(raw: bigint, denominator = FIXED5): string {
  const negative = raw < 0n, magnitude = negative ? -raw : raw;
  const places = denominator.toString().length - 1;
  const whole = magnitude / denominator;
  const fraction = ((magnitude % denominator) * 10n ** BigInt(places) / denominator).toString().padStart(places, '0').replace(/0+$/, '');
  return `${negative ? '-' : ''}${whole}${fraction ? `.${fraction}` : ''}`;
}

export const priceLevel = (level: number | null): string => level === null ? '—' : ['Low', 'Normal', 'High'][level] ?? 'Price level unavailable';

export interface Equipment { name: string | null; icon: string | null; missing: string | null }
export function equipmentOf(reply: Pick<MarketLendLeaseReply, 'variants' | 'icons'>): (variant: string) => Equipment {
  const images = new Map(reply.icons.map(icon => [icon.key, icon.image]));
  const variants = new Map(reply.variants.map(variant => [variant.id, variant]));
  return variant => {
    const held = variants.get(variant);
    return { name: held?.name ?? null, icon: held?.icon ? images.get(held.icon) ?? null : null,
      missing: held ? held.nameUnknown ?? null : 'The recording does not name this design at this hour.' };
  };
}

export const partyTags = (...tags: readonly (string | null)[]): string[] | null => {
  const known = [...new Set(tags.filter((tag): tag is string => tag !== null))];
  return known.length ? known.sort() : null;
};
