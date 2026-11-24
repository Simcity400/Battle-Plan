import {
  DEFAULT_VIEW,
  ensureBoundProjection,
  type BoundProjectionKey,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
import type { HeatMatrixModel, HeatMatrixProjection } from './matrixModel.ts';

export function registerHeatMatrixProjection<TOptions>(
  projection: HeatMatrixProjection<TOptions>,
  view = DEFAULT_VIEW,
): HeatMatrixProjection<TOptions> {
  return registerFamilyProjection(projection, view, 'heat-matrix') as HeatMatrixProjection<TOptions>;
}

export function heatMatrixProjection(kind: string, view?: string): HeatMatrixProjection<unknown> | undefined {
  return familyProjection<HeatMatrixModel, unknown>('heat-matrix', kind, view) as HeatMatrixProjection<unknown> | undefined;
}

export function ensureHeatMatrixProjection(
  binding: BoundProjectionKey,
  onReady: () => void,
): HeatMatrixProjection<unknown> | 'loading' | 'unknown' {
  return ensureBoundProjection<HeatMatrixModel, unknown>(binding, onReady) as HeatMatrixProjection<unknown> | 'loading' | 'unknown';
}

export function registeredHeatMatrixKinds(): readonly string[] {
  return [...new Set(registeredFamilyProjections('heat-matrix').map(entry => entry.kind))];
}
