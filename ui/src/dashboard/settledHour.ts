import { resolveWindow, type BoundTrack, type WidgetSpec } from './model.ts';
import { recordedHour } from './regionTime.ts';

export function trackHourOf(widget: WidgetSpec, track: BoundTrack | undefined, tracks: BoundTrack[], clockHour: number): number | null {
  return recordedHour(track, track ? resolveWindow(widget, track, tracks, clockHour)?.lastHour : undefined);
}
