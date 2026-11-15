import { useIdentityHour } from './atHead.ts';
import { looksRead } from './countryLooks.ts';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { trackHourOf } from './settledHour.ts';

/** The hour a widget names `tags` at (their names, flags and colours): while its rows are pushed at the head, the last
 * hour at or before the head at which one of their identities changed, so a push, which comes at a new hour each
 * time, asks for no name and renders nothing of its own; elsewhere `hour`, the hour it shows. `exact` as
 * `looksRead` gives it. */
export function usePushedLooksHour(runtime: WidgetRuntime, widget: WidgetSpec, track: BoundTrack | undefined, tags: readonly string[], pushed: boolean,
  hour: number): { hour: number; exact: boolean } {
  const identityHour = useIdentityHour(runtime.client, track, tags, runtime.clock, pushed,
    clockHour => track ? trackHourOf(widget, track, runtime.tracks, clockHour) : null);
  const read = looksRead(pushed, identityHour, Math.floor(hour));
  return { hour: read.hour, exact: read.exact };
}
