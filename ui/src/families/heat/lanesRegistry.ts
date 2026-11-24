import { ensureFamilyProjection } from '../registry.ts';
import type { HeatLanesModel, HeatLanesProjection } from './lanesModel.ts';

export function ensureHeatLanesProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): HeatLanesProjection<any> | 'loading' | 'unknown' {
  return ensureFamilyProjection<HeatLanesModel, any>('heat-lanes', kind, view, onReady) as HeatLanesProjection<any> | 'loading' | 'unknown';
}
