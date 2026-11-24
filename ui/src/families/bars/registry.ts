import {
  DEFAULT_VIEW,
  ensureBoundProjection,
  type BoundProjectionKey,
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
  binding: BoundProjectionKey,
  onReady: () => void,
): BarsProjection<any> | 'loading' | 'unknown' {
  return ensureBoundProjection<BarsModel, any>(binding, onReady) as BarsProjection<any> | 'loading' | 'unknown';
}

export function registeredBarsKinds(): string[] {
  return [...new Set(registeredFamilyProjections('bars').map(entry => entry.kind))];
}
