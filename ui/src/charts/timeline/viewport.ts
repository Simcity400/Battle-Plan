import { AXIS_HEIGHT, ROW_HEIGHT, laneOffset } from './layout.ts';
import { backingSize } from '../surface.ts';

export interface LaneWindow { top: number; height: number; first: number; end: number }

export const timelineHeight = (count: number, rowHeight = ROW_HEIGHT) => AXIS_HEIGHT + count * rowHeight + 8;

/** An absolute view needs a width: its absolute canvas cannot size it. */
export function timelineViewStyle(height: number, viewportHeight: number, stickyAxis = true) {
  return stickyAxis ? { width: '100%', height: Math.min(height, viewportHeight) }
    : { width: '100%', position: 'absolute' as const, top: 0, height, overflow: 'visible' as const };
}

/** The canvas covers the clipped viewport and a viewport of overscan on either side.
 * Coordinates remain those of the full timeline, including its date axis. */
export function laneWindow(count: number, visibleTop: number, visibleHeight: number, rowHeight = ROW_HEIGHT): LaneWindow {
  const total = timelineHeight(count, rowHeight);
  const overscan = Math.max(2 * rowHeight, visibleHeight);
  const top = Math.floor(Math.max(0, Math.min(total - 1, visibleTop - overscan)));
  const bottom = Math.ceil(Math.min(total, Math.max(top, visibleTop + visibleHeight + overscan)));
  return {
    top, height: Math.max(1, bottom - top),
    first: Math.max(0, Math.min(count, Math.floor((top - AXIS_HEIGHT) / rowHeight))),
    end: Math.max(0, Math.min(count, Math.ceil((bottom - AXIS_HEIGHT) / rowHeight))),
  };
}

/** Large scroll jumps need painting in the scroll event, before the next frame. */
export function coversViewport(window: LaneWindow, count: number, visible: { top: number; height: number }, rowHeight = ROW_HEIGHT): boolean {
  const total = timelineHeight(count, rowHeight);
  return visible.height <= 0 || (window.top <= Math.max(0, visible.top)
    && window.top + window.height >= Math.min(total, visible.top + visible.height));
}

/** Leave unchanged styles alone during playback and pointer frames. */
export function placeLaneWindow(style: Pick<CSSStyleDeclaration, 'top' | 'height'>, window: LaneWindow, offset = 0): void {
  const top = `${window.top - offset}px`, height = `${window.height}px`;
  if (style.top !== top) style.top = top;
  if (style.height !== height) style.height = height;
}

/** Measure after placing the window; backing writes reset the drawing context. */
export function sizeLaneWindow(canvas: Pick<HTMLCanvasElement, 'style' | 'clientWidth' | 'clientHeight' | 'getBoundingClientRect' | 'width' | 'height'>,
  window: LaneWindow, offset: number, devicePixelRatio: number) {
  placeLaneWindow(canvas.style, window, offset);
  const layoutWidth = canvas.clientWidth, layoutHeight = canvas.clientHeight;
  if (!layoutWidth || !layoutHeight) return null;
  const rect = canvas.getBoundingClientRect();
  const size = backingSize({ layoutWidth, layoutHeight, rectWidth: rect.width, rectHeight: rect.height, devicePixelRatio });
  if (canvas.width !== size.width) canvas.width = size.width;
  if (canvas.height !== size.height) canvas.height = size.height;
  return { ...size, layoutWidth, layoutHeight };
}

/** Full-timeline y from the viewport canvas's local y, shared by hover and Jump to. */
export const timelineY = (localY: number, window: LaneWindow) => localY + window.top;

/** Visible part of the full-height spacer in its own pixels, including Size zoom.
 * Existing widget scroll containers keep ownership of their scrolling. */
export function visibleTimeline(element: HTMLElement): { top: number; height: number } {
  const rect = element.getBoundingClientRect();
  const scale = element.clientHeight ? rect.height / element.clientHeight : 1;
  let top = Math.max(rect.top, 0);
  let bottom = Math.min(rect.bottom, window.innerHeight);
  for (let parent = element.parentElement; parent; parent = parent.parentElement) {
    if (!/auto|scroll|hidden|clip/.test(getComputedStyle(parent).overflowY)) continue;
    const box = parent.getBoundingClientRect();
    const units = parent.offsetHeight ? box.height / parent.offsetHeight : 1;
    const edge = box.top + parent.clientTop * units;
    top = Math.max(top, edge);
    bottom = Math.min(bottom, edge + parent.clientHeight * units);
  }
  return { top: (top - rect.top) / Math.max(1e-6, scale), height: Math.max(0, bottom - top) / Math.max(1e-6, scale) };
}

/** The visible full-timeline coordinates and the sticky view's scroll offset. Both
 * axis modes use the same clipped, overscanned canvas and pointer coordinates. */
export function timelineViewport(content: HTMLElement, view: HTMLElement, count: number, rowHeight = ROW_HEIGHT,
  stickyAxis = true): { visible: { top: number; height: number }; offset: number } {
  if (!stickyAxis) return { visible: visibleTimeline(content), offset: 0 };
  const rect = view.getBoundingClientRect();
  const box = content.getBoundingClientRect();
  const toLayout = rect.height ? view.clientHeight / rect.height : 1;
  const offset = laneOffset((rect.top - box.top) * toLayout, 0, view.clientHeight, timelineHeight(count, rowHeight));
  const clipped = visibleTimeline(view);
  return { visible: { top: offset + clipped.top, height: clipped.height }, offset };
}
