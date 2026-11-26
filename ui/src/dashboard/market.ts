import type { MarketLendLeaseReply } from '../generated/dashboard';
import { object } from './validation.ts';

export type MarketAt = MarketLendLeaseReply & { hour: number };

export const MARKET = 'market-lendlease';
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
export const isLease = (row: Record<string, unknown>) => typeof row.key === 'string' && typeof row.id === 'string' && text(row.giver) && text(row.giverMissing) && text(row.receiver) && text(row.receiverMissing) && raw(row.revision)
  && integer(row.activeSince) && integer(row.lastDelivery) && ['monthly', 'percentage', 'once', 'delivered', 'sunk'].every(key => items(row[key]))
  && ['fuelDailyRaw', 'fuelPercentageRaw', 'lastFuelDeliveredRaw'].every(key => raw(row[key]));
export function validateMarketReply(value: unknown): MarketLendLeaseReply {
  const nullable = (v: unknown, valid: (row: Record<string, unknown>) => boolean) => v === null || list(v, valid);
  if (!object(value) || typeof value.branchId !== 'string' || !named(value) || !text(value.purchasesMissing) || !text(value.agreementsMissing) || !integer(value.unattributed) || !text(value.refused)
    || !nullable(value.offers, row => integer(row.country) && text(row.tag) && items(row.inventory) && levels(row.prices) && grants(row.subsidies) && raw(row.subsidyTotalRaw) && typeof row.autoSendMarketAccess === 'boolean')
    || !nullable(value.contracts, row => typeof row.key === 'string' && typeof row.id === 'string' && text(row.seller) && text(row.buyer) && typeof row.request === 'boolean'
      && integer(row.speed) && items(row.equipment) && items(row.prices) && levels(row.levels) && grants(row.subsidies) && typeof row.purchase === 'boolean')
    || !nullable(value.agreements, isLease)) throw new Error('Invalid market and lend-lease reply');
  return value as unknown as MarketLendLeaseReply;
}

function terminatingDecimalPlaces(denominator: bigint): number | null {
  if (denominator <= 0n) throw new RangeError(`fixedText: denominator must be positive, got ${denominator}`);
  let rest = denominator;
  let twos = 0;
  let fives = 0;
  while (rest % 2n === 0n) {
    rest /= 2n;
    twos++;
  }
  while (rest % 5n === 0n) {
    rest /= 5n;
    fives++;
  }
  return rest === 1n ? Math.max(twos, fives) : null;
}

export function fixedText(raw: bigint, denominator = FIXED5): string {
  const exactPlaces = terminatingDecimalPlaces(denominator);
  if (raw === 0n) return '0';
  const negative = raw < 0n;
  const magnitude = negative ? -raw : raw;
  const places = exactPlaces ?? denominator.toString().length;
  const whole = magnitude / denominator;
  const remainder = magnitude % denominator;
  const scaled = (remainder * 10n ** BigInt(places)) / denominator;
  const fraction = scaled.toString().padStart(places, '0').replace(/0+$/, '');
  return `${negative ? '-' : ''}${whole}${fraction ? `.${fraction}` : ''}`;
}

export const priceLevel = (level: number | null): string => level === null ? '—' : ['Low', 'Normal', 'High'][level] ?? 'Price level unavailable';

export interface Equipment { name: string | null; icon: string | null; missing: string | null; iconMissing: string | null }
export function equipmentOf(reply: Pick<MarketLendLeaseReply, 'variants' | 'icons'>): (variant: string) => Equipment {
  const images = new Map(reply.icons.map(icon => [icon.key, icon.image]));
  const variants = new Map(reply.variants.map(variant => [variant.id, variant]));
  return variant => {
    const held = variants.get(variant);
    const icon = held?.icon ? images.get(held.icon) ?? null : null;
    const iconMissing = icon !== null
      ? null
      : held
        ? (held.iconUnknown ?? (held.icon ? `${held.icon}: the product drew no icon for this equipment` : 'The recording names no icon for this equipment'))
        : 'The recording does not name this design at this hour.';
    return {
      name: held?.name ?? null,
      icon,
      missing: held ? held.nameUnknown ?? null : 'The recording does not name this design at this hour.',
      iconMissing,
    };
  };
}
