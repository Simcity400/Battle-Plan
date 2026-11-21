import { useIdentityHour } from './atHead.ts';
import { looksRead } from './countryLooks.ts';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { trackHourOf } from './settledHour.ts';

export function usePushedLooksHour(runtime: WidgetRuntime, widget: WidgetSpec, track: BoundTrack | undefined, tags: readonly string[], pushed: boolean,
  hour: number): { hour: number; exact: boolean } {
  const identityHour = useIdentityHour(runtime.client, track, tags, runtime.clock, pushed,
    clockHour => track ? trackHourOf(widget, track, runtime.tracks, clockHour) : null);
  const read = looksRead(pushed, identityHour, Math.floor(hour));
  return { hour: read.hour, exact: read.exact };
}
