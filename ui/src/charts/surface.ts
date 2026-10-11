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

/** A drawing loop's canvas size. Layout sizes are read again only when the canvas resizes,
 * or its parent does (a Size zoom change keeps the canvas's device pixels but not its layout
 * size). Read in every frame, each chart drawn after another has resized its canvas would
 * force a layout. `resized` asks for a frame. */
export function watchCanvasSize(element: HTMLCanvasElement, resized: () => void) {
  let device: { width: number; height: number } | null = null;
  let measured: Pick<SurfaceMeasure, 'layoutWidth' | 'layoutHeight' | 'rectWidth' | 'rectHeight'> | null = null;
  const measure = () => {
    const rect = element.getBoundingClientRect();
    return measured = { layoutWidth: element.clientWidth, layoutHeight: element.clientHeight, rectWidth: rect.width, rectHeight: rect.height };
  };
  const observer = new ResizeObserver(entries => {
    const own = entries.find(entry => entry.target === element);
    if (own) {
      const box = own.devicePixelContentBoxSize?.[0];
      device = box ? { width: box.inlineSize, height: box.blockSize } : null;
    }
    measure();
    resized();
  });
  try { observer.observe(element, { box: 'device-pixel-content-box' }); }
  catch { observer.observe(element); }
  observer.observe(element.parentElement!);
  return {
    /** The layout size, and the backing store that covers it sharply at the current pixel ratio. */
    read() {
      const size = measured ?? measure();
      return { ...size, backing: backingSize({ ...size, devicePixelRatio: window.devicePixelRatio || 1,
        devicePixelWidth: device?.width, devicePixelHeight: device?.height }) };
    },
    disconnect: () => observer.disconnect(),
  };
}

/** A pointer position in the canvas's layout pixels, whatever the zoom. */
export function toLocal(clientX: number, clientY: number, rect: { left: number; top: number; width: number; height: number },
  layoutWidth: number, layoutHeight: number): { x: number; y: number } {
  return {
    x: (clientX - rect.left) * (layoutWidth / Math.max(1e-6, rect.width)),
    y: (clientY - rect.top) * (layoutHeight / Math.max(1e-6, rect.height)),
  };
}
