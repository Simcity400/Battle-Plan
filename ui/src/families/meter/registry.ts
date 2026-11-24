import {
  DEFAULT_VIEW,
  ensureFamilyProjection,
  registerFamilyProjection,
} from '../registry.ts';
import type { MeterModel, MeterProjection } from './model.ts';

export function registerMeterProjection<TOptions>(
  projection: MeterProjection<TOptions>,
  view = DEFAULT_VIEW,
): MeterProjection<TOptions> {
  return registerFamilyProjection(projection, view) as MeterProjection<TOptions>;
}

export function ensureMeterProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): MeterProjection<unknown> | 'loading' | 'unknown' {
  return ensureFamilyProjection<MeterModel, unknown>('meters', kind, view, onReady) as MeterProjection<unknown> | 'loading' | 'unknown';
}
