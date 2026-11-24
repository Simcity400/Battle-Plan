import { ensureFamilyProjection } from '../registry.ts';
import type { HeatMatrixModel, HeatMatrixProjection } from './matrixModel.ts';

export function ensureHeatMatrixProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): HeatMatrixProjection<any> | 'loading' | 'unknown' {
  return ensureFamilyProjection<HeatMatrixModel, any>('heat-matrix', kind, view, onReady) as HeatMatrixProjection<any> | 'loading' | 'unknown';
}
