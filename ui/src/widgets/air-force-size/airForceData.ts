import { countryFromKey } from '../../dashboard/countryKey.ts';
import { validateForceReply } from '../../charts/race/forceRace.ts';
import type { ForceParams } from '../../charts/race/forceRace';
import type { OptionValue } from '../../dashboard/options';
import type { ForceSizeReply } from '../../generated/dashboard';

export function validateAirForceReply(data: unknown): ForceSizeReply {
  return validateForceReply(data);
}

export function airForceRequest(branchId: string, resolved: { tags: readonly string[]; top?: number },
  options: Readonly<Record<string, OptionValue>>): ForceParams {
  return { branchId, countries: resolved.top == null ? resolved.tags.map(countryFromKey) : [], top: resolved.top ?? null,
    measure: options.measure === 'wings' ? 'wings' : 'planes', ownership: options.ownership === 'effective' ? 'effective' : 'roster' };
}

export function airForceTotals(options: Readonly<Record<string, OptionValue>>) {
  const owner = options.ownership === 'effective' ? 'effective' : 'roster';
  return { kind: 'air-force-totals', measure: `${owner}${options.measure === 'wings' ? 'Wings' : 'Planes'}`, decimals: 0 };
}
