import { ensureFamilyProjection } from '../registry.ts';
import type { BreakdownModel, BreakdownProjection } from './model.ts';

export function ensureBreakdownProjection(
  kind: string,
  view: string,
  onReady: () => void,
): BreakdownProjection<any> | 'loading' | 'unknown' {
  return ensureFamilyProjection<BreakdownModel, any>('breakdown', kind, view, onReady) as BreakdownProjection<any> | 'loading' | 'unknown';
}
