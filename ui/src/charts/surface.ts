
export type DrawingCanvas = HTMLCanvasElement | OffscreenCanvas;
export interface DrawingSurface { canvas: DrawingCanvas; context: CanvasRenderingContext2D | null }

const surfaces = new WeakMap<HTMLCanvasElement, DrawingSurface>();

export function drawingSurface(element: HTMLCanvasElement): DrawingSurface {
  let held = surfaces.get(element);
  if (held) return held;
  let canvas: DrawingCanvas = element;
  if (typeof element.transferControlToOffscreen === 'function') {
    try { canvas = element.transferControlToOffscreen(); } catch { canvas = element; }
  }
  held = { canvas, context: canvas.getContext('2d') as CanvasRenderingContext2D | null };
  surfaces.set(element, held);
  return held;
}

export function sizeCanvas(canvas: DrawingCanvas, width: number, height: number): boolean {
  if (canvas.width === width && canvas.height === height) return false;
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  return true;
}

export interface SurfaceMeasure {
  layoutWidth: number; layoutHeight: number;
  rectWidth: number; rectHeight: number;
  devicePixelRatio: number;
  devicePixelWidth?: number; devicePixelHeight?: number;
}
export interface BackingSize { width: number; height: number; scaleX: number; scaleY: number }

export const MAX_BACKING_EDGE = 8192;

export function backingSize(measure: SurfaceMeasure): BackingSize {
  const edge = (device: number | undefined, rect: number) => Math.max(1, Math.min(MAX_BACKING_EDGE,
    device && device > 0 ? device : Math.round(rect * measure.devicePixelRatio)));
  const width = edge(measure.devicePixelWidth, measure.rectWidth);
  const height = edge(measure.devicePixelHeight, measure.rectHeight);
  return { width, height, scaleX: width / Math.max(1, measure.layoutWidth), scaleY: height / Math.max(1, measure.layoutHeight) };
}

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
    read() {
      const size = measured ?? measure();
      return { ...size, backing: backingSize({ ...size, devicePixelRatio: window.devicePixelRatio || 1,
        devicePixelWidth: device?.width, devicePixelHeight: device?.height }) };
    },
    disconnect: () => observer.disconnect(),
  };
}

export function windowRatio(): number {
  return typeof devicePixelRatio === 'number' && devicePixelRatio > 0 ? devicePixelRatio : 1;
}

export function deviceScale(element: Element): number {
  const zoom = element.currentCSSZoom;
  return windowRatio() * (zoom > 0 ? zoom : 1);
}

export interface DeviceFit { readonly width: number; readonly height: number; readonly dpr: number }

export function deviceFit(element: Element, cssWidth: number, cssHeight: number): DeviceFit {
  const dpr = deviceScale(element);
  const whole = (css: number) => Math.max(0, Math.floor(css * dpr + 1e-3)) / dpr;
  return { width: whole(cssWidth), height: whole(cssHeight), dpr };
}

export function snapToDevice(css: number, scale: number): number {
  return Math.round(css * scale) / scale;
}

const ratioListeners = new Set<() => void>();
let ratioQuery: { readonly media: MediaQueryList; readonly changed: () => void } | null = null;

function armRatioQuery() {
  if (typeof matchMedia !== 'function') return;
  const media = matchMedia(`(resolution: ${windowRatio()}dppx)`);
  const changed = () => {
    media.removeEventListener('change', changed);
    ratioQuery = null;
    if (ratioListeners.size) armRatioQuery();
    for (const listener of [...ratioListeners]) listener();
  };
  media.addEventListener('change', changed);
  ratioQuery = { media, changed };
}

export function watchDevicePixelRatio(listener: () => void): () => void {
  ratioListeners.add(listener);
  if (!ratioQuery) armRatioQuery();
  return () => {
    ratioListeners.delete(listener);
    if (ratioListeners.size || !ratioQuery) return;
    ratioQuery.media.removeEventListener('change', ratioQuery.changed);
    ratioQuery = null;
  };
}

export function toLocal(clientX: number, clientY: number, rect: { left: number; top: number; width: number; height: number },
  layoutWidth: number, layoutHeight: number): { x: number; y: number } {
  return {
    x: (clientX - rect.left) * (layoutWidth / Math.max(1e-6, rect.width)),
    y: (clientY - rect.top) * (layoutHeight / Math.max(1e-6, rect.height)),
  };
}
