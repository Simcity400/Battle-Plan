import { resolveWindow, type BoundTrack, type WidgetSpec } from './model.ts';

/** The last of `hours` (ascending) at or before `hour`, `hour` itself before the first: the hour a view drawn
 * from a history that changes only at `hours` shows at `hour`, so following the head it renders only there. */
export function settledHour(hours: readonly number[], hour: number): number {
  let low = 0, high = hours.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (hours[middle] <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? hour : hours[found];
}

/** The track hour `clockHour` names for `widget` (its window's end), null outside the recorded branch. */
export function trackHourOf(widget: WidgetSpec, track: BoundTrack | undefined, tracks: BoundTrack[], clockHour: number): number | null {
  const hour = track ? resolveWindow(widget, track, tracks, clockHour)?.lastHour : undefined;
  return hour === undefined || !track || hour < track.branch.firstHour || hour > track.branch.lastHour ? null : Math.floor(hour);
}
