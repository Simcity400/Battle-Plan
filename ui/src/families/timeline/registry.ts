import { registerFamilyProjection } from '../registry.ts';
import type { TimelineModel, TimelineProjection } from './model.ts';

export function registerTimelineProjection<TOptions>(
  projection: Omit<TimelineProjection<TOptions>, 'family'>,
): TimelineProjection<TOptions> {
  const full: TimelineProjection<TOptions> = { ...projection, family: 'timeline' };
  registerFamilyProjection<TimelineModel, TOptions>(full);
  return full;
}
