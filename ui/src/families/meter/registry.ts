import type { MeterProjection } from './model.ts';

const projections = new Map<string, MeterProjection<unknown>>();

export function registerMeterProjection<TOptions>(projection: MeterProjection<TOptions>): void {
  projections.set(projection.kind, projection as MeterProjection<unknown>);
}

export function meterProjection(kind: string): MeterProjection<unknown> | undefined {
  return projections.get(kind);
}
