import type { HeatLanesProjection } from './lanesModel.ts';

const registry = new Map<string, HeatLanesProjection<unknown>>();

export function registerHeatLanesProjection<TOptions>(projection: HeatLanesProjection<TOptions>): void {
  registry.set(projection.kind, projection as HeatLanesProjection<unknown>);
}

export function heatLanesProjection(kind: string): HeatLanesProjection<unknown> | undefined {
  return registry.get(kind);
}

export function registeredHeatLanesKinds(): readonly string[] {
  return [...registry.keys()];
}
