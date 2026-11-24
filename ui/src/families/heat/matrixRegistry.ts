import type { HeatMatrixProjection } from './matrixModel.ts';

const registry = new Map<string, HeatMatrixProjection<unknown>>();

export function registerHeatMatrixProjection<TOptions>(projection: HeatMatrixProjection<TOptions>): void {
  registry.set(projection.kind, projection as HeatMatrixProjection<unknown>);
}

export function heatMatrixProjection(kind: string): HeatMatrixProjection<unknown> | undefined {
  return registry.get(kind);
}

export function registeredHeatMatrixKinds(): readonly string[] {
  return [...registry.keys()];
}
