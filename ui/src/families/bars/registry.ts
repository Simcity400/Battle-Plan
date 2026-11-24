import { DEFAULT_VIEW, registerFamilyProjection } from '../registry.ts';
import type { BarsProjection } from './model.ts';

export function registerBarsProjection<TOptions>(
  projection: BarsProjection<TOptions>,
  view = DEFAULT_VIEW,
): BarsProjection<TOptions> {
  return registerFamilyProjection(projection, view) as BarsProjection<TOptions>;
}
