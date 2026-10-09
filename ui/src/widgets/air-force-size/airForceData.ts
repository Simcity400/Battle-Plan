import type { AirForceParams } from '../../generated/dashboard';
import { validateNativePartsRaceReply } from '../../charts/race/partsRaceDrill.ts';
import type { NativePartsRaceReply } from '../../charts/race/partsRaceDrill';
import type { OptionValue } from '../../dashboard/options';

export function validateAirForceReply(data: unknown): NativePartsRaceReply {
  return validateNativePartsRaceReply(data);
}

/** Country choices are opaque tags; top N asks the product to rank the native measure. */
export function airForceRequest(branchId: string, resolved: { tags: readonly string[]; top?: number },
  options: Readonly<Record<string, OptionValue>>): AirForceParams {
  return { branchId, tags: resolved.top == null ? [...resolved.tags] : [], top: resolved.top ?? null,
    measure: options.measure === 'wings' ? 'wings' : 'planes', ownership: options.ownership === 'effective' ? 'effective' : 'roster' };
}
