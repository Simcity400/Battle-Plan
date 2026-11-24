import type { MeterProjection } from './model.ts';

const projections = new Map<string, MeterProjection<unknown>>();

export function registerMeterProjection<TOptions>(projection: MeterProjection<TOptions>): MeterProjection<TOptions> {
  projections.set(projection.kind, projection as MeterProjection<unknown>);
  return projection;
}

export function meterProjection(kind: string): MeterProjection<unknown> | undefined {
  return projections.get(kind);
}
