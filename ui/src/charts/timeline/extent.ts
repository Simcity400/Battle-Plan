import { EMPTY_VIEW } from '../view.ts';
import type { ViewState } from '../view.ts';

/** Growing playback bounds do not replace a range the user zoomed or panned to. */
export function viewForExtent(view: ViewState, previous: string, next: string, preserveZoom = false): ViewState {
  return !previous || previous === next || preserveZoom ? view : EMPTY_VIEW;
}

/** A drawing extent may include the final hour's right edge; names use its read hour. */
export function labelMoment(playhead: number | null, extentEnd: number, labelHour?: number): number {
  return labelHour ?? playhead ?? extentEnd;
}
