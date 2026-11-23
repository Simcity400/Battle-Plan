import type { GatedHour } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { ProductRequest } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
export { priceLevel, validateMarketReply } from '../../dashboard/market.ts';
import { INTERACTIVE_PRODUCTS } from '../../generated/dashboard.ts';
import { productRequest } from '../../dashboard/request.ts';

export function marketLendLeaseWaits(products: readonly string[] = INTERACTIVE_PRODUCTS): string | null {
  return products.includes('market-lendlease') ? null : 'The market and lend-lease view comes with the market widgets, which this build does not serve yet.';
}
export function marketRequest(track: Pick<BoundTrack, 'source' | 'branch'>, hour: GatedHour, tags: readonly string[]): ProductRequest {
  return productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'market-lendlease', params: { branchId: track.branch.id, hour, countries: ([...tags].sort()).map(countryFromKey)} });
}
export function rawAmount(raw: string, denominator = 100000n, places = 5): string {
  const value = BigInt(raw), magnitude = value < 0n ? -value : value;
  const whole = magnitude / denominator;
  const fraction = ((magnitude % denominator) * 10n ** BigInt(places) / denominator).toString().padStart(places, '0').replace(/0+$/, '');
  return `${value < 0n ? '-' : ''}${whole}${fraction ? `.${fraction}` : ''}`;
}
