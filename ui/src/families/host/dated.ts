import type { FamilyStaticSources } from './adapter.ts';

export interface BoundDated {
  readonly h: number;
  readonly c: number;
}

export function boundDated(
  staticSources: FamilyStaticSources | null,
  hasSubscriptions: boolean,
  datedByBounds: boolean,
  bounds: readonly [number, number] | null,
): BoundDated | undefined {
  if (staticSources?.follows && staticSources.provenInterval) {
    return { h: staticSources.provenInterval[0], c: staticSources.provenInterval[1] };
  }
  if (hasSubscriptions) return undefined;
  if (staticSources) {
    return staticSources.provenInterval
      ? { h: staticSources.provenInterval[0], c: staticSources.provenInterval[1] }
      : { h: staticSources.hour, c: staticSources.hour };
  }
  return datedByBounds && bounds ? { h: bounds[0], c: bounds[1] } : undefined;
}
