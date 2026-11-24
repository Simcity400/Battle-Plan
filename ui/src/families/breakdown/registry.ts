import {
  DEFAULT_VIEW,
  ensureFamilyProjection,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
import type { BreakdownModel, BreakdownProjection } from './model.ts';

export function registerBreakdownProjection<TOptions>(
  projection: BreakdownProjection<TOptions>,
  view = DEFAULT_VIEW,
): BreakdownProjection<TOptions> {
  return registerFamilyProjection(
    { ...projection, family: 'breakdown' },
    view,
    'breakdown',
  ) as BreakdownProjection<TOptions>;
}

export function breakdownProjection(kind: string, view?: string): BreakdownProjection<any> | undefined {
  return (familyProjection<BreakdownModel, any>('breakdown', kind, view)
    ?? (view ? familyProjection<BreakdownModel, any>('breakdown', kind, undefined) : undefined)) as BreakdownProjection<any> | undefined;
}

export function ensureBreakdownProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): BreakdownProjection<any> | 'loading' | 'unknown' {
  const direct = ensureFamilyProjection<BreakdownModel, any>('breakdown', kind, view, onReady);
  if (direct !== 'unknown' || !view) return direct as BreakdownProjection<any> | 'loading' | 'unknown';
  return ensureFamilyProjection<BreakdownModel, any>('breakdown', kind, undefined, onReady) as BreakdownProjection<any> | 'loading' | 'unknown';
}

export function registeredBreakdownKinds(): readonly string[] {
  return [...new Set(registeredFamilyProjections('breakdown').map(entry => entry.kind))];
}
