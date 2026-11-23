import { EMPTY_VIEW } from '../view.ts';
import type { ViewState } from '../view.ts';

export function viewForExtent(view: ViewState, previous: string, next: string, preserveZoom = false): ViewState {
  return !previous || previous === next || preserveZoom ? view : EMPTY_VIEW;
}

export function labelMoment(playhead: number | null, extentEnd: number, labelHour?: number): number {
  return labelHour ?? playhead ?? extentEnd;
}

export function liveHeaded(extent: readonly [number, number], head: number, growTo?: number): { extent: readonly [number, number]; head: number } {
  if (growTo === undefined) return { extent, head: Math.max(extent[0], Math.min(extent[1], head)) };
  const at = Math.max(extent[0], Math.min(growTo, head));
  return { extent: at > extent[1] ? [extent[0], at] : extent, head: at };
}
