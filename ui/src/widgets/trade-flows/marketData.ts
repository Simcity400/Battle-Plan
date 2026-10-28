import type { MarketLendLeaseReply } from '../../generated/dashboard';
import type { ProductRequest } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
export function marketRequest(track: Pick<BoundTrack, 'source' | 'branch'>, hour: number, tags: readonly string[]): ProductRequest {
  return { sourceId: track.source.id, revision: track.source.revision, product: 'market-lendlease', params: { branchId: track.branch.id, hour: Math.floor(hour), tags: [...tags].sort() } };
}
export function rawAmount(raw: string, denominator = 100000n, places = 5): string {
  const value = BigInt(raw), magnitude = value < 0n ? -value : value;
  const whole = magnitude / denominator;
  const fraction = ((magnitude % denominator) * 10n ** BigInt(places) / denominator).toString().padStart(places, '0').replace(/0+$/, '');
  return `${value < 0n ? '-' : ''}${whole}${fraction ? `.${fraction}` : ''}`;
}
export function priceLevel(level: number): string { return ['Low', 'Normal', 'High'][level] ?? 'Price level unavailable'; }
export function validateMarketReply(value: unknown): MarketLendLeaseReply {
  const record = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
  const integer = (v: unknown) => typeof v === 'number' && Number.isSafeInteger(v);
  const raw = (v: unknown) => typeof v === 'string' && /^-?\d+$/.test(v);
  const text = (v: unknown) => v === null || typeof v === 'string';
  const list = (v: unknown, valid: (row: Record<string, unknown>) => boolean): boolean => v === null || Array.isArray(v) && v.every(row => record(row) && valid(row));
  const item = (row: Record<string, unknown>) => typeof row.variant === 'string' && text(row.name) && raw(row.amountRaw);
  const items = (v: unknown) => Array.isArray(v) && list(v, item);
  const levels = (v: unknown) => Array.isArray(v) && list(v, row => typeof row.variant === 'string' && text(row.name) && integer(row.level));
  const grants = (v: unknown) => Array.isArray(v) && list(v, row => raw(row.cicRaw) && text(row.archetype) && text(row.trigger) && Array.isArray(row.targets) && row.targets.every(v => typeof v === 'string'));
  if (!record(value) || typeof value.branchId !== 'string' || !integer(value.hour) || !Array.isArray(value.warnings) || !value.warnings.every(v => typeof v === 'string')
    || !list(value.offers, row => integer(row.country) && text(row.tag) && items(row.inventory) && levels(row.prices) && grants(row.subsidies) && raw(row.subsidyTotalRaw) && typeof row.autoSendMarketAccess === 'boolean')
    || !list(value.contracts, row => typeof row.id === 'string' && text(row.seller) && text(row.buyer) && typeof row.request === 'boolean' && integer(row.speed) && items(row.equipment) && items(row.prices) && levels(row.levels) && grants(row.subsidies))
    || !list(value.agreements, row => typeof row.id === 'string' && text(row.giver) && text(row.receiver) && raw(row.revision) && integer(row.activeSince) && integer(row.lastDelivery) && ['monthly', 'percentage', 'once', 'delivered', 'sunk'].every(key => items(row[key])) && ['fuelDailyRaw', 'fuelPercentageRaw', 'fuelSentRaw', 'fuelSunkRaw', 'lastFuelDeliveredRaw'].every(key => raw(row[key])))
    || !list(value.deliveries, row => raw(row.id) && raw(row.occurrence) && typeof row.agreement === 'string' && integer(row.hour) && integer(row.kind) && text(row.giver) && text(row.receiver) && raw(row.lostRaw) && raw(row.totalRaw) && raw(row.lostTotalRaw) && record(row.item) && item(row.item))) throw new Error('Invalid market and lend-lease reply');
  return value as unknown as MarketLendLeaseReply;
}
