/** Canvas sizing that stays sharp under the widget's Size zoom. The canvas keeps its
 * CSS layout size; its backing store follows the size it really covers on screen
 * (which includes zoom and display scaling), and drawing uses layout pixels. */

export interface SurfaceMeasure {
  /** The canvas element's layout size in CSS pixels (clientWidth/Height). */
  layoutWidth: number; layoutHeight: number;
  /** Its on-screen rectangle in CSS pixels (getBoundingClientRect), which includes zoom. */
  rectWidth: number; rectHeight: number;
  devicePixelRatio: number;
  /** Exact device pixels from ResizeObserver, when the browser reports them. */
  devicePixelWidth?: number; devicePixelHeight?: number;
}
export interface BackingSize { width: number; height: number; scaleX: number; scaleY: number }

/** Largest backing edge; keeps one widget from allocating an unbounded canvas. */
export const MAX_BACKING_EDGE = 8192;

export function backingSize(measure: SurfaceMeasure): BackingSize {
  const edge = (device: number | undefined, rect: number) => Math.max(1, Math.min(MAX_BACKING_EDGE,
    device && device > 0 ? device : Math.round(rect * measure.devicePixelRatio)));
  const width = edge(measure.devicePixelWidth, measure.rectWidth);
  const height = edge(measure.devicePixelHeight, measure.rectHeight);
  return { width, height, scaleX: width / Math.max(1, measure.layoutWidth), scaleY: height / Math.max(1, measure.layoutHeight) };
}

/** A pointer position in the canvas's layout pixels, whatever the zoom. */
export function toLocal(clientX: number, clientY: number, rect: { left: number; top: number; width: number; height: number },
  layoutWidth: number, layoutHeight: number): { x: number; y: number } {
  return {
    x: (clientX - rect.left) * (layoutWidth / Math.max(1e-6, rect.width)),
    y: (clientY - rect.top) * (layoutHeight / Math.max(1e-6, rect.height)),
  };
}
