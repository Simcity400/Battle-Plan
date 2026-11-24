import { ensureFamilyProjection } from '../registry.ts';
import type { CardModel, CardProjection } from './model.ts';

export function ensureCardProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): CardProjection<any> | 'loading' | 'unknown' {
  return ensureFamilyProjection<CardModel, any>('card', kind, view, onReady) as CardProjection<any> | 'loading' | 'unknown';
}
