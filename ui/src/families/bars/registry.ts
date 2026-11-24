import {
  DEFAULT_VIEW,
  ensureFamilyProjection,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
import type { BarsModel, BarsProjection } from './model.ts';

export function registerBarsProjection<TOptions>(
  projection: BarsProjection<TOptions>,
  view = DEFAULT_VIEW,
): BarsProjection<TOptions> {
  return registerFamilyProjection(projection, view) as BarsProjection<TOptions>;
}

export function barsProjection(kind: string, view?: string): BarsProjection<any> | undefined {
  return familyProjection<BarsModel, any>('bars', kind, view) as BarsProjection<any> | undefined;
}

export function ensureBarsProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): BarsProjection<any> | 'loading' | 'unknown' {
  return ensureFamilyProjection<BarsModel, any>('bars', kind, view, onReady) as BarsProjection<any> | 'loading' | 'unknown';
}

export function registeredBarsKinds(): string[] {
  return [...new Set(registeredFamilyProjections('bars').map(entry => entry.kind))];
}
