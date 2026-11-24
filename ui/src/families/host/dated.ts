import type { FamilyStaticSources } from './adapter.ts';

export interface BoundDated {
  readonly h: number;
  readonly c: number;
}

export function boundDated(
  staticSources: FamilyStaticSources | null,
  proof: readonly [lo: number, hi: number] | null,
  hasSubscriptions: boolean,
  datedByBounds: boolean,
  bounds: readonly [number, number] | null,
): BoundDated | undefined {
  if (staticSources?.follows && proof) return { h: proof[0], c: proof[1] };
  if (hasSubscriptions) return undefined;
  if (staticSources) return proof ? { h: proof[0], c: proof[1] } : { h: staticSources.hour, c: staticSources.hour };
  return datedByBounds && bounds ? { h: bounds[0], c: bounds[1] } : undefined;
}
