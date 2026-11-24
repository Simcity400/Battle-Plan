import type { BarsProjection } from './model.ts';

const projections = new Map<string, BarsProjection<any>>();

export function registerBarsProjection<TOptions>(projection: BarsProjection<TOptions>): BarsProjection<TOptions> {
  projections.set(projection.kind, projection as BarsProjection<any>);
  return projection;
}

export function barsProjection(kind: string): BarsProjection<any> | undefined {
  return projections.get(kind);
}

export function registeredBarsKinds(): string[] {
  return [...projections.keys()];
}
