/** Canvas sizing that stays sharp under the widget's Size zoom. The canvas keeps its
 * CSS layout size; its backing store follows the size it really covers on screen
 * (which includes zoom and display scaling), and drawing uses layout pixels. */

/** The canvas a chart draws into and sizes: the element's offscreen canvas where the browser can hand it one. */
export type DrawingCanvas = HTMLCanvasElement | OffscreenCanvas;
export interface DrawingSurface { canvas: DrawingCanvas; context: CanvasRenderingContext2D | null }

const surfaces = new WeakMap<HTMLCanvasElement, DrawingSurface>();
const elements = new WeakMap<OffscreenCanvas, HTMLCanvasElement>();

/** A chart canvas's drawing, detached from the page's style. On a canvas in the document, setting a
 * font, drawing text or measuring it first brings the whole page's style up to date: a forced style
 * pass for every chart drawn after another painter wrote to the page in the same frame. Handed to an
 * offscreen canvas, the drawing never waits on the page, and the page shows each frame it draws.
 * Size `canvas`, never the element, which can no longer be resized once handed over. */
export function drawingSurface(element: HTMLCanvasElement): DrawingSurface {
  let held = surfaces.get(element);
  if (held) return held;
  let canvas: DrawingCanvas = element;
  if (typeof element.transferControlToOffscreen === 'function') {
    try { canvas = element.transferControlToOffscreen(); elements.set(canvas, element); } catch { canvas = element; }
  }
  held = { canvas, context: canvas.getContext('2d') as CanvasRenderingContext2D | null };
  surfaces.set(element, held);
  return held;
}

/** The page's canvas a drawing canvas shows on. */
export function canvasElement(canvas: DrawingCanvas): HTMLCanvasElement | null {
  return typeof HTMLCanvasElement !== 'undefined' && canvas instanceof HTMLCanvasElement ? canvas : elements.get(canvas as OffscreenCanvas) ?? null;
}

/** Sizes a drawing canvas's backing store; true when it changed (which clears it). */
export function sizeCanvas(canvas: DrawingCanvas, width: number, height: number): boolean {
  if (canvas.width === width && canvas.height === height) return false;
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  return true;
}

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
