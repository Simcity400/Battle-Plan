import { DEFAULT_VIEW, ensureFamilyProjection } from '../registry.ts';
import type { BreakdownModel, BreakdownProjection } from './model.ts';

export function ensureBreakdownProjection(kind: string, onReady: () => void): BreakdownProjection<any> | 'loading' | 'unknown' {
  return ensureFamilyProjection<BreakdownModel, any>('breakdown', kind, DEFAULT_VIEW, onReady) as BreakdownProjection<any> | 'loading' | 'unknown';
}
