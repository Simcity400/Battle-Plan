import { EMPTY_VIEW } from '../view.ts';
import type { ViewState } from '../view.ts';

export function viewForExtent(view: ViewState, previous: string, next: string, preserveZoom = false): ViewState {
  return !previous || previous === next || preserveZoom ? view : EMPTY_VIEW;
}

export function labelMoment(playhead: number | null, extentEnd: number, labelHour?: number): number {
  return labelHour ?? playhead ?? extentEnd;
}
