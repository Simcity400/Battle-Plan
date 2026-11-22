import { resolveWindow, type BoundTrack, type WidgetSpec } from './model.ts';
import { recordedHour } from './regionTime.ts';

export function settledHour(hours: readonly number[], hour: number): number {
  let low = 0, high = hours.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (hours[middle] <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? hour : hours[found];
}

export function trackHourOf(widget: WidgetSpec, track: BoundTrack | undefined, tracks: BoundTrack[], clockHour: number): number | null {
  return recordedHour(track, track ? resolveWindow(widget, track, tracks, clockHour)?.lastHour : undefined);
}
