import { countryFromKey } from '../../dashboard/countryKey.ts';
import { validateForceReply } from '../../charts/race/forceRace.ts';
import type { ForceParams } from '../../charts/race/forceRace';
import type { OptionValue } from '../../dashboard/options';
import type { ForceSizeReply } from '../../generated/dashboard';

export function validateAirForceReply(data: unknown): ForceSizeReply {
  return validateForceReply(data);
}

/** Country choices are opaque tags; top N asks the product to rank the declared measure. */
export function airForceRequest(branchId: string, resolved: { tags: readonly string[]; top?: number },
  options: Readonly<Record<string, OptionValue>>): ForceParams {
  return { branchId, countries: resolved.top == null ? resolved.tags.map(countryFromKey) : [], top: resolved.top ?? null,
    measure: options.measure === 'wings' ? 'wings' : 'planes', ownership: options.ownership === 'effective' ? 'effective' : 'roster' };
}

/** The declared totals a line of chosen countries reads. */
export function airForceTotals(options: Readonly<Record<string, OptionValue>>) {
  const owner = options.ownership === 'effective' ? 'effective' : 'roster';
  return { kind: 'air-force-totals', measure: `${owner}${options.measure === 'wings' ? 'Wings' : 'Planes'}`, decimals: 0 };
}
