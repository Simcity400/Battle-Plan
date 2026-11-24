import {
  DEFAULT_VIEW,
  ensureFamilyProjection,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
import type { HeatLanesModel, HeatLanesProjection } from './lanesModel.ts';

export function registerHeatLanesProjection<TOptions>(
  projection: HeatLanesProjection<TOptions>,
  view = DEFAULT_VIEW,
): HeatLanesProjection<TOptions> {
  return registerFamilyProjection(projection, view, 'heat-lanes') as HeatLanesProjection<TOptions>;
}

export function heatLanesProjection(kind: string, view?: string): HeatLanesProjection<unknown> | undefined {
  return familyProjection<HeatLanesModel, unknown>('heat-lanes', kind, view) as HeatLanesProjection<unknown> | undefined;
}

export function ensureHeatLanesProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): HeatLanesProjection<unknown> | 'loading' | 'unknown' {
  return ensureFamilyProjection<HeatLanesModel, unknown>('heat-lanes', kind, view, onReady) as HeatLanesProjection<unknown> | 'loading' | 'unknown';
}

export function registeredHeatLanesKinds(): readonly string[] {
  return [...new Set(registeredFamilyProjections('heat-lanes').map(entry => entry.kind))];
}
