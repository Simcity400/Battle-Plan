import { DEFAULT_VIEW, registerFamilyProjection } from '../registry.ts';
import type { HeatMatrixProjection } from './matrixModel.ts';

export function registerHeatMatrixProjection<TOptions>(
  projection: HeatMatrixProjection<TOptions>,
  view = DEFAULT_VIEW,
): HeatMatrixProjection<TOptions> {
  return registerFamilyProjection(projection, view, 'heat-matrix') as HeatMatrixProjection<TOptions>;
}
