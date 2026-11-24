import {
  DEFAULT_VIEW,
  ensureFamilyProjection,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
import type { TimelineModel, TimelineProjection } from './model.ts';

export function registerTimelineProjection<TOptions>(
  projection: Omit<TimelineProjection<TOptions>, 'family'> & { readonly family?: 'timeline' },
  view = DEFAULT_VIEW,
): TimelineProjection<TOptions> {
  const full: TimelineProjection<TOptions> = { ...projection, family: 'timeline' };
  return registerFamilyProjection(full, view, 'timeline') as unknown as TimelineProjection<TOptions>;
}

export function timelineProjection(kind: string, view?: string): TimelineProjection<any> | undefined {
  return familyProjection<TimelineModel, any>('timeline', kind, view) as unknown as TimelineProjection<any> | undefined;
}

export function ensureTimelineProjection(
  kind: string,
  view: string | undefined,
  onReady: () => void,
): TimelineProjection<any> | 'loading' | 'unknown' {
  return ensureFamilyProjection<TimelineModel, any>('timeline', kind, view, onReady) as unknown as TimelineProjection<any> | 'loading' | 'unknown';
}

export function registeredTimelineKinds(): string[] {
  return [...new Set(registeredFamilyProjections('timeline').map(entry => entry.kind))];
}
