import type { TimelineProjection } from './model.ts';

const registry = new Map<string, TimelineProjection<any>>();

export function registerTimelineProjection<TOptions>(projection: TimelineProjection<TOptions>): TimelineProjection<TOptions> {
  registry.set(projection.kind, projection as TimelineProjection<any>);
  return projection;
}

export function timelineProjection(kind: string): TimelineProjection<any> | undefined {
  return registry.get(kind);
}

export function timelineProjections(): readonly TimelineProjection<any>[] {
  return [...registry.values()];
}
