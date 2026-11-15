// The requests widgets issue, built from the widget and its track alone.
import type { BoundTrack, HourWindow, WidgetSpec } from './model';

type Track = BoundTrack | undefined;
export function clockRequest(widget: WidgetSpec, track: Track, window: HourWindow | null) {
  if (!track || !window) return null;
  const firstHour = widget.time.mode !== 'range'
    ? track.branch.firstHour : Math.max(track.branch.firstHour, Math.floor(window.firstHour));
  const lastHour = Math.min(track.branch.lastHour, Math.floor(window.lastHour));
  return firstHour <= lastHour ? { sourceId: track.source.id, revision: track.source.revision, product: 'campaign-clock',
    params: { branchId: track.branch.id, firstHour, lastHour } } : null;
}

/** The shown hours inside the branch. */
export const windowExtent = (track: BoundTrack, window: HourWindow): [number, number] =>
  [Math.max(track.branch.firstHour, window.firstHour), Math.min(track.branch.lastHour, window.lastHour)];

/** The hour a moment widget shows. */
export const shownHour = (track: Track, window: HourWindow | null | undefined) => Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
