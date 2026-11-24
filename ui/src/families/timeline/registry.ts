import { ensureFamilyProjection, familyProjection, registerFamilyProjection } from '../registry.ts';
import type { TimelineModel, TimelineProjection } from './model.ts';

export function registerTimelineProjection<TOptions>(
  projection: Omit<TimelineProjection<TOptions>, 'family'>,
): TimelineProjection<TOptions> {
  const full: TimelineProjection<TOptions> = { ...projection, family: 'timeline' };
  registerFamilyProjection<TimelineModel, TOptions>(full);
  return full;
}

export function timelineProjection(kind: string): TimelineProjection<unknown> | undefined {
  return familyProjection<TimelineModel, unknown>('timeline', kind) as TimelineProjection<unknown> | undefined;
}

export function ensureTimelineProjection(kind: string, view: string | undefined, onReady: () => void): TimelineProjection<unknown> | 'loading' | 'unknown' {
  return ensureFamilyProjection<TimelineModel, unknown>('timeline', kind, view, onReady) as TimelineProjection<unknown> | 'loading' | 'unknown';
}
