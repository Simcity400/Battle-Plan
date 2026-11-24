import { DEFAULT_VIEW, registerFamilyProjection } from '../registry.ts';
import type { MeterProjection } from './model.ts';

export function registerMeterProjection<TOptions>(
  projection: MeterProjection<TOptions>,
  view = DEFAULT_VIEW,
): MeterProjection<TOptions> {
  return registerFamilyProjection(projection, view) as MeterProjection<TOptions>;
}
