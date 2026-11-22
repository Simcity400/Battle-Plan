import type { BoundTrack, HourWindow, WidgetSpec } from './model';
import type { Gate } from './regionGate';
import { productRequest } from './request.ts';

type Track = BoundTrack | undefined;
export function clockRequest(gate: Pick<Gate, 'recorded' | 'span'>, widget: WidgetSpec, track: Track, window: HourWindow | null) {
  if (!track || !window) return null;
  if (widget.time.mode !== 'range' && gate.recorded(window.lastHour) === null) return null;
  const span = gate.span(widget.time.mode !== 'range' ? track.branch.firstHour : window.firstHour, window.lastHour);
  return span ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'campaign-clock',
    params: { branchId: track.branch.id, ...span } }) : null;
}

export const windowExtent = (track: BoundTrack, window: HourWindow): [number, number] =>
  [Math.max(track.branch.firstHour, window.firstHour), Math.min(track.branch.lastHour, window.lastHour)];

export const shownHour = (track: Track, window: HourWindow | null | undefined) => Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
