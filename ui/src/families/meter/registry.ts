import {
  DEFAULT_VIEW,
  ensureFamilyProjection,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
import type { MeterModel, MeterProjection } from './model.ts';

export function registerMeterProjection<TOptions>(
  projection: MeterProjection<TOptions>,
  view = DEFAULT_VIEW,
): MeterProjection<TOptions> {
  return registerFamilyProjection(projection, view) as MeterProjection<TOptions>;
}

export function meterProjection(kind: string, view?: string): MeterProjection<unknown> | undefined {
  return familyProjection<MeterModel, unknown>('meters', kind, view) as MeterProjection<unknown> | undefined;
}

export function ensureMeterProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): MeterProjection<unknown> | 'loading' | 'unknown' {
  return ensureFamilyProjection<MeterModel, unknown>('meters', kind, view, onReady) as MeterProjection<unknown> | 'loading' | 'unknown';
}

export function registeredMeterKinds(): string[] {
  return [...new Set(registeredFamilyProjections('meters').map(entry => entry.kind))];
}
